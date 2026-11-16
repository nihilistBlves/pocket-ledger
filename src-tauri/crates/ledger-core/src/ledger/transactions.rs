//! The filtered, paginated transaction list, plus the category and tag lookups.

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::{params_from_iter, Connection, Row};
use serde::{Deserialize, Serialize};

use super::period::parse_date;
use super::{LedgerError, Result, TxRow, DEFAULT_LIMIT, EFFECTIVE_DATE, MAX_LIMIT, MAX_TEXT_CHARS};
use crate::normalize::normalize_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusFilter {
    Pending,
    Booked,
}

/// Every field is optional; all given fields must match.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TxFilter {
    /// Inclusive effective dates, `YYYY-MM-DD`.
    pub from: Option<String>,
    pub to: Option<String>,
    pub account_id: Option<i64>,
    /// The category itself or any child of it.
    pub category_id: Option<i64>,
    pub uncategorized: Option<bool>,
    /// Accent/case-insensitive; every word must appear in description, counterparty,
    /// remittance or notes.
    pub text: Option<String>,
    /// On the absolute amount.
    pub min_cents: Option<i64>,
    pub max_cents: Option<i64>,
    pub direction: Option<Direction>,
    pub status: Option<StatusFilter>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxSummary {
    pub income_cents: i64,
    pub expense_cents: i64,
    pub net_cents: i64,
    pub transfer_count: i64,
    pub pending_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxPage {
    pub rows: Vec<TxRow>,
    /// Rows matching the filter, regardless of pagination.
    pub total: i64,
    pub summary: TxSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryOption {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub key: Option<String>,
    pub name: String,
    /// `income`, `expense`, `transfer` or `system`.
    pub kind: String,
    /// Stored colour token (`cat-1`..`cat-8`, `other`); `None` = default / inherit.
    pub color: Option<String>,
}

pub(crate) const ROW_SELECT: &str = "SELECT t.id, t.account_id, a.name, t.status, \
     COALESCE(t.booking_date, t.value_date, substr(t.created_at, 1, 10)), \
     t.booking_date, t.value_date, t.amount_cents, t.currency, t.description, \
     t.counterparty, t.remittance, t.category_id, c.name, c.key, pc.id, pc.name, \
     t.category_source, COALESCE(c.kind = 'transfer', 0), t.transfer_peer_id, pa.name, t.notes \
     FROM transactions t \
     JOIN accounts a ON a.id = t.account_id \
     LEFT JOIN categories c ON c.id = t.category_id \
     LEFT JOIN categories pc ON pc.id = c.parent_id \
     LEFT JOIN transactions peer ON peer.id = t.transfer_peer_id \
     LEFT JOIN accounts pa ON pa.id = peer.account_id";

fn map_row(r: &Row<'_>) -> rusqlite::Result<TxRow> {
    Ok(TxRow {
        id: r.get(0)?,
        account_id: r.get(1)?,
        account_name: r.get(2)?,
        status: r.get(3)?,
        date: r.get(4)?,
        booking_date: r.get(5)?,
        value_date: r.get(6)?,
        amount_cents: r.get(7)?,
        currency: r.get(8)?,
        description: r.get(9)?,
        counterparty: r.get(10)?,
        remittance: r.get(11)?,
        category_id: r.get(12)?,
        category_name: r.get(13)?,
        category_key: r.get(14)?,
        parent_category_id: r.get(15)?,
        parent_category_name: r.get(16)?,
        category_source: r.get(17)?,
        is_transfer: r.get::<_, i64>(18)? != 0,
        transfer_peer_id: r.get(19)?,
        transfer_peer_account_name: r.get(20)?,
        notes: r.get(21)?,
        tags: Vec::new(),
    })
}

fn ids_json(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string())
}

/// Runs `ROW_SELECT` with `tail` appended (WHERE/ORDER/LIMIT) and attaches the tags.
pub(crate) fn query_rows(conn: &Connection, tail: &str, params: Vec<Value>) -> Result<Vec<TxRow>> {
    let mut stmt = conn.prepare(&format!("{ROW_SELECT} {tail}"))?;
    let mut rows = stmt
        .query_map(params_from_iter(params), map_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    attach_tags(conn, &mut rows)?;
    Ok(rows)
}

/// The rows with these ids, in the order given (unknown ids are skipped).
pub(crate) fn rows_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<TxRow>> {
    let mut found = query_rows(
        conn,
        "WHERE t.id IN (SELECT value FROM json_each(?1))",
        vec![Value::Text(ids_json(ids))],
    )?;
    let mut ordered = Vec::with_capacity(found.len());
    for id in ids {
        if let Some(pos) = found.iter().position(|r| r.id == *id) {
            ordered.push(found.swap_remove(pos));
        }
    }
    Ok(ordered)
}

fn attach_tags(conn: &Connection, rows: &mut [TxRow]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let mut stmt = conn.prepare(
        "SELECT tt.transaction_id, g.name
         FROM transaction_tags tt JOIN tags g ON g.id = tt.tag_id
         WHERE tt.transaction_id IN (SELECT value FROM json_each(?1))
         ORDER BY g.name COLLATE NOCASE",
    )?;
    let mut by_tx: HashMap<i64, Vec<String>> = HashMap::new();
    for pair in stmt.query_map([ids_json(&ids)], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (tx, name) = pair?;
        by_tx.entry(tx).or_default().push(name);
    }
    for row in rows {
        row.tags = by_tx.remove(&row.id).unwrap_or_default();
    }
    Ok(())
}

/// WHERE clause under construction, with numbered parameters.
struct Where {
    conds: Vec<String>,
    params: Vec<Value>,
}

impl Where {
    fn push(&mut self, cond: &str, value: Value) {
        self.params.push(value);
        self.conds.push(cond.replace('?', &format!("?{}", self.params.len())));
    }

    fn sql(&self) -> String {
        if self.conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", self.conds.join(" AND "))
        }
    }
}

fn build_where(f: &TxFilter) -> Result<Where> {
    let mut w = Where { conds: vec!["a.is_own = 1".to_string()], params: Vec::new() };
    if let Some(from) = &f.from {
        parse_date(from)?;
        w.push(&format!("{EFFECTIVE_DATE} >= ?"), Value::Text(from.clone()));
    }
    if let Some(to) = &f.to {
        parse_date(to)?;
        w.push(&format!("{EFFECTIVE_DATE} <= ?"), Value::Text(to.clone()));
    }
    if let Some(id) = f.account_id {
        w.push("t.account_id = ?", Value::Integer(id));
    }
    if let Some(id) = f.category_id {
        w.push("(t.category_id = ? OR c.parent_id = ?)", Value::Integer(id));
    }
    if f.uncategorized == Some(true) {
        w.conds.push("(t.category_id IS NULL OR c.key = 'uncategorized')".to_string());
    }
    for (value, cond) in [(f.min_cents, "abs(t.amount_cents) >= ?"), (f.max_cents, "abs(t.amount_cents) <= ?")] {
        if let Some(v) = value {
            if v < 0 {
                return Err(LedgerError::InvalidInput("amount bounds must not be negative"));
            }
            w.push(cond, Value::Integer(v));
        }
    }
    match f.direction {
        Some(Direction::In) => w.conds.push("t.amount_cents > 0".to_string()),
        Some(Direction::Out) => w.conds.push("t.amount_cents < 0".to_string()),
        None => {}
    }
    match f.status {
        Some(StatusFilter::Pending) => w.conds.push("t.status = 'pending'".to_string()),
        Some(StatusFilter::Booked) => w.conds.push("t.status = 'booked'".to_string()),
        None => {}
    }
    Ok(w)
}

/// Ids of the rows (already matching `base`) whose texts contain every word of `text`,
/// ignoring case and accents. SQLite has no accent folding, so the comparison runs in
/// Rust with the same normalization the rules use.
fn text_matches(conn: &Connection, base: &Where, text: &str) -> Result<Vec<i64>> {
    let terms: Vec<String> = normalize_text(text).split(' ').filter(|s| !s.is_empty()).map(str::to_owned).collect();
    let sql = format!(
        "SELECT t.id, t.description, t.counterparty, t.remittance, t.notes
         FROM transactions t
         JOIN accounts a ON a.id = t.account_id
         LEFT JOIN categories c ON c.id = t.category_id
         {}",
        base.sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut ids = Vec::new();
    let mut rows = stmt.query(params_from_iter(base.params.iter()))?;
    while let Some(r) = rows.next()? {
        let mut hay = normalize_text(&r.get::<_, String>(1)?);
        for i in 2..5 {
            if let Some(s) = r.get::<_, Option<String>>(i)? {
                hay.push(' ');
                hay.push_str(&normalize_text(&s));
            }
        }
        if terms.iter().all(|t| hay.contains(t.as_str())) {
            ids.push(r.get(0)?);
        }
    }
    Ok(ids)
}

pub fn list_transactions(conn: &Connection, filter: &TxFilter) -> Result<TxPage> {
    let limit = filter.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(LedgerError::InvalidInput("limit must be between 1 and 500"));
    }
    let offset = filter.offset.unwrap_or(0);
    if offset < 0 {
        return Err(LedgerError::InvalidInput("offset must not be negative"));
    }
    let mut w = build_where(filter)?;
    if let Some(text) = filter.text.as_deref() {
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(LedgerError::InvalidInput("search text is too long"));
        }
        if !normalize_text(text).is_empty() {
            let ids = text_matches(conn, &w, text)?;
            w.push("t.id IN (SELECT value FROM json_each(?))", Value::Text(ids_json(&ids)));
        }
    }
    let where_sql = w.sql();

    let (total, income, expense, transfers, pending): (i64, i64, i64, i64, i64) = conn.query_row(
        &format!(
            "SELECT count(*),
                    COALESCE(SUM(CASE WHEN COALESCE(c.kind, '') <> 'transfer' AND t.amount_cents > 0
                                      THEN t.amount_cents END), 0),
                    COALESCE(SUM(CASE WHEN COALESCE(c.kind, '') <> 'transfer' AND t.amount_cents < 0
                                      THEN -t.amount_cents END), 0),
                    COALESCE(SUM(COALESCE(c.kind, '') = 'transfer'), 0),
                    COALESCE(SUM(t.status = 'pending'), 0)
             FROM transactions t
             JOIN accounts a ON a.id = t.account_id
             LEFT JOIN categories c ON c.id = t.category_id
             {where_sql}"
        ),
        params_from_iter(w.params.iter()),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;

    let mut params = w.params.clone();
    params.push(Value::Integer(limit));
    params.push(Value::Integer(offset));
    let (l, o) = (params.len() - 1, params.len());
    let rows = query_rows(
        conn,
        &format!("{where_sql} ORDER BY {EFFECTIVE_DATE} DESC, t.id DESC LIMIT ?{l} OFFSET ?{o}"),
        params,
    )?;
    Ok(TxPage {
        rows,
        total,
        summary: TxSummary {
            income_cents: income,
            expense_cents: expense,
            net_cents: income - expense,
            transfer_count: transfers,
            pending_count: pending,
        },
    })
}

/// The category tree flattened: groups first (by `sort_order`), each followed by its children.
pub fn categories(conn: &Connection) -> Result<Vec<CategoryOption>> {
    let mut stmt = conn.prepare(
        "SELECT id, parent_id, key, name, kind, color FROM categories
         ORDER BY COALESCE((SELECT p.sort_order FROM categories p WHERE p.id = categories.parent_id), sort_order),
                  COALESCE(parent_id, id), parent_id IS NOT NULL, sort_order, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CategoryOption { id: r.get(0)?, parent_id: r.get(1)?, key: r.get(2)?, name: r.get(3)?, kind: r.get(4)?, color: r.get(5)? })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// All tag names, for suggestions.
pub fn tag_names(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(rows)
}
