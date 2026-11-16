//! Monthly budgets behind the "Presupuestos" screen (docs/decisions/phase-7.md).
//!
//! A budget row is valid from `start_month` to `end_month` (NULL = open); for a month M
//! the active row of a category is the one with `start_month <= M` and
//! (`end_month` NULL or `>= M`). Spend counts pending rows, excludes internal transfers
//! and lets refunds reduce it. Everything is integer cents.

use std::collections::HashMap;

use chrono::{Months, NaiveDate};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::period::{iso, last_day_of_month, month_key, parse_month, periods};
use super::{LedgerError, Result, EFFECTIVE_DATE};

pub const MAX_BUDGET_CENTS: i64 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetTotals {
    pub budget_cents: i64,
    pub spent_cents: i64,
    pub remaining_cents: i64,
    pub over_count: i64,
    pub over_cents: i64,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetRow {
    pub category_id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub parent_name: Option<String>,
    pub is_group: bool,
    pub child_names: Vec<String>,
    /// Effective colour: own, else the group's, else `None`.
    pub color: Option<String>,
    pub amount_cents: i64,
    pub spent_cents: i64,
    pub remaining_cents: i64,
    /// `spent * 10000 / amount` (truncated); may exceed 10000 or be negative.
    pub percent_bp: i64,
    /// `ok`, `warn` (spent >= 90 % of the amount) or `over` (spent > amount).
    pub status: &'static str,
    pub avg3_cents: i64,
    pub start_month: String,
    /// Amount active in the month before `start_month`, if any.
    pub previous_amount_cents: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnbudgetedRow {
    pub category_id: i64,
    pub name: String,
    pub parent_name: Option<String>,
    pub color: Option<String>,
    pub spent_cents: i64,
    pub avg3_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetMonth {
    pub month: String,
    pub is_current_month: bool,
    pub day_of_month: Option<u32>,
    pub days_in_month: u32,
    pub totals: BudgetTotals,
    pub budgets: Vec<BudgetRow>,
    pub unbudgeted: Vec<UnbudgetedRow>,
    /// Outflows of the month on rows without a real category ("Sin categoría").
    pub uncategorized_cents: i64,
}

struct Cat {
    id: i64,
    parent_id: Option<i64>,
    name: String,
    kind: String,
    color: Option<String>,
}

/// `n / d` rounded half away from zero (`d > 0`).
fn div_round(n: i64, d: i64) -> i64 {
    let q = n / d;
    let r = n % d;
    if r.abs() * 2 >= d {
        q + n.signum()
    } else {
        q
    }
}

fn month_before(first: NaiveDate, n: u32) -> Result<NaiveDate> {
    first
        .checked_sub_months(Months::new(n))
        .ok_or(LedgerError::InvalidInput("month out of range"))
}

fn prev_month_key(month: &str) -> Result<String> {
    Ok(month_key(month_before(parse_month(month)?, 1)?))
}

/// Net spend (`-sum`) per category over `[from, to]`: own accounts, pending included,
/// internal transfers excluded.
fn net_spend(conn: &Connection, from: &str, to: &str) -> Result<HashMap<i64, i64>> {
    let sql = format!(
        "SELECT t.category_id, COALESCE(-sum(t.amount_cents), 0)
         FROM transactions t
         JOIN accounts a ON a.id = t.account_id AND a.is_own = 1
         JOIN categories c ON c.id = t.category_id
         WHERE c.kind <> 'transfer' AND {EFFECTIVE_DATE} BETWEEN ?1 AND ?2
         GROUP BY t.category_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![from, to], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

/// Budget rows active in `month`, by category: (amount, start_month).
fn active_budgets(conn: &Connection, month: &str) -> Result<HashMap<i64, (i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT b.category_id, b.amount_cents, b.start_month
         FROM budgets b JOIN categories c ON c.id = b.category_id
         WHERE c.kind = 'expense' AND b.start_month <= ?1 AND (b.end_month IS NULL OR b.end_month >= ?1)
         ORDER BY b.start_month, b.id",
    )?;
    let rows = stmt
        .query_map([month], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                (r.get::<_, i64>(1)?, r.get::<_, String>(2)?),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().collect())
}

fn status_of(spent: i64, amount: i64) -> &'static str {
    if spent > amount {
        "over"
    } else if spent * 100 >= amount * 90 {
        "warn"
    } else {
        "ok"
    }
}

/// The budgets screen for `month`.
pub fn budgets(conn: &Connection, month: &str, today: &str) -> Result<BudgetMonth> {
    let per = periods(month, today)?;
    let m = month_key(per.month_start);
    let (from, to) = (iso(per.month_start), iso(per.month_end));
    let avg_from = iso(month_before(per.month_start, 3)?);
    let avg_to = iso(last_day_of_month(month_before(per.month_start, 1)?));

    let cats = {
        let mut stmt = conn.prepare(
            "SELECT id, parent_id, name, kind, color FROM categories ORDER BY sort_order, id",
        )?;
        let cats = stmt
            .query_map([], |r| {
                Ok(Cat {
                    id: r.get(0)?,
                    parent_id: r.get(1)?,
                    name: r.get(2)?,
                    kind: r.get(3)?,
                    color: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        cats
    };
    let by_id: HashMap<i64, &Cat> = cats.iter().map(|c| (c.id, c)).collect();
    let mut kids: HashMap<i64, Vec<&Cat>> = HashMap::new();
    for c in &cats {
        if let Some(p) = c.parent_id {
            kids.entry(p).or_default().push(c);
        }
    }
    let color_of = |c: &Cat| {
        c.color.clone().or_else(|| {
            c.parent_id
                .and_then(|p| by_id.get(&p))
                .and_then(|p| p.color.clone())
        })
    };

    let spent = net_spend(conn, &from, &to)?;
    let past = net_spend(conn, &avg_from, &avg_to)?;
    let active = active_budgets(conn, &m)?;
    let own = |map: &HashMap<i64, i64>, id: i64| map.get(&id).copied().unwrap_or(0);
    // Own rows plus those of the subcategories.
    let with_children = |map: &HashMap<i64, i64>, c: &Cat| {
        own(map, c.id)
            + kids
                .get(&c.id)
                .map_or(0, |k| k.iter().map(|k| own(map, k.id)).sum::<i64>())
    };

    let mut rows = Vec::new();
    for c in cats.iter().filter(|c| c.kind == "expense") {
        let Some((amount, start)) = active.get(&c.id) else {
            continue;
        };
        let spent_cents = with_children(&spent, c);
        let prev = prev_month_key(start)?;
        let previous_amount_cents: Option<i64> = conn
            .query_row(
                "SELECT amount_cents FROM budgets
                 WHERE category_id = ?1 AND start_month <= ?2 AND (end_month IS NULL OR end_month >= ?2)
                 ORDER BY start_month DESC LIMIT 1",
                params![c.id, prev],
                |r| r.get(0),
            )
            .optional()?;
        let is_group = c.parent_id.is_none();
        rows.push(BudgetRow {
            category_id: c.id,
            name: c.name.clone(),
            parent_id: c.parent_id,
            parent_name: c
                .parent_id
                .and_then(|p| by_id.get(&p))
                .map(|p| p.name.clone()),
            is_group,
            child_names: kids
                .get(&c.id)
                .map_or_else(Vec::new, |k| k.iter().map(|k| k.name.clone()).collect()),
            color: color_of(c),
            amount_cents: *amount,
            spent_cents,
            remaining_cents: amount - spent_cents,
            percent_bp: spent_cents * 10_000 / amount,
            status: status_of(spent_cents, *amount),
            avg3_cents: div_round(with_children(&past, c), 3),
            start_month: start.clone(),
            previous_amount_cents,
        });
    }
    rows.sort_by(|a, b| {
        b.percent_bp
            .cmp(&a.percent_bp)
            .then_with(|| a.name.cmp(&b.name))
    });

    let totals = BudgetTotals {
        budget_cents: rows.iter().map(|r| r.amount_cents).sum(),
        spent_cents: rows.iter().map(|r| r.spent_cents).sum(),
        remaining_cents: rows.iter().map(|r| r.remaining_cents).sum(),
        over_count: rows.iter().filter(|r| r.status == "over").count() as i64,
        over_cents: rows
            .iter()
            .filter(|r| r.status == "over")
            .map(|r| r.spent_cents - r.amount_cents)
            .sum(),
        count: rows.len() as i64,
    };

    // Spend that happened on categories no active budget covers.
    let mut unbudgeted = Vec::new();
    for c in cats.iter().filter(|c| c.kind == "expense") {
        let spent_cents = own(&spent, c.id);
        if spent_cents <= 0 {
            continue;
        }
        let covered = active.contains_key(&c.id)
            || c.parent_id.is_some_and(|p| active.contains_key(&p))
            || kids
                .get(&c.id)
                .is_some_and(|k| k.iter().any(|k| active.contains_key(&k.id)));
        if covered {
            continue;
        }
        unbudgeted.push(UnbudgetedRow {
            category_id: c.id,
            name: c.name.clone(),
            parent_name: c
                .parent_id
                .and_then(|p| by_id.get(&p))
                .map(|p| p.name.clone()),
            color: color_of(c),
            spent_cents,
            avg3_cents: div_round(own(&past, c.id), 3),
        });
    }
    unbudgeted.sort_by(|a, b| {
        b.spent_cents
            .cmp(&a.spent_cents)
            .then_with(|| a.name.cmp(&b.name))
    });

    let sql = format!(
        "SELECT COALESCE(sum(CASE WHEN t.amount_cents < 0 THEN -t.amount_cents ELSE 0 END), 0)
         FROM transactions t
         JOIN accounts a ON a.id = t.account_id AND a.is_own = 1
         LEFT JOIN categories c ON c.id = t.category_id
         WHERE (t.category_id IS NULL OR c.kind = 'system') AND {EFFECTIVE_DATE} BETWEEN ?1 AND ?2"
    );
    let uncategorized_cents: i64 = conn.query_row(&sql, params![from, to], |r| r.get(0))?;

    Ok(BudgetMonth {
        month: m,
        is_current_month: per.is_current,
        day_of_month: per.through_day,
        days_in_month: chrono::Datelike::day(&per.month_end),
        totals,
        budgets: rows,
        unbudgeted,
        uncategorized_cents,
    })
}

/// True when any of `categories` has a budget row overlapping `[from, to]` (`to` = open).
fn overlaps(conn: &Connection, categories: &[i64], from: &str, to: Option<&str>) -> Result<bool> {
    for &id in categories {
        let hit: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM budgets
                 WHERE category_id = ?1 AND start_month <= COALESCE(?3, '9999-99')
                   AND (end_month IS NULL OR end_month >= ?2)
                 LIMIT 1",
                params![id, from, to],
                |r| r.get(0),
            )
            .optional()?;
        if hit.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Sets the budget of an expense category from `month` on ("Aplicar desde"). A row that
/// starts at `month` is updated; otherwise the active row is closed the month before and a
/// new one starts (ending where the closed one ended). Earlier months keep their amount.
/// A group and its subcategories can not both have a budget (`Conflict`).
pub fn set_budget(
    conn: &Connection,
    category_id: i64,
    month: &str,
    amount_cents: i64,
) -> Result<()> {
    if !(1..=MAX_BUDGET_CENTS).contains(&amount_cents) {
        return Err(LedgerError::InvalidInput(
            "amount must be between 1 and 100000000 cents",
        ));
    }
    let m = month_key(parse_month(month)?);
    let dbtx = conn.unchecked_transaction()?;
    let (parent_id, kind): (Option<i64>, String) = dbtx
        .query_row(
            "SELECT parent_id, kind FROM categories WHERE id = ?1",
            [category_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or(LedgerError::NotFound("category"))?;
    if kind != "expense" {
        return Err(LedgerError::InvalidInput(
            "only expense categories can have a budget",
        ));
    }
    let related: Vec<i64> = match parent_id {
        Some(p) => vec![p],
        None => {
            let mut stmt = dbtx.prepare("SELECT id FROM categories WHERE parent_id = ?1")?;
            let ids = stmt
                .query_map([category_id], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            ids
        }
    };
    let active: Option<(i64, String, Option<String>)> = dbtx
        .query_row(
            "SELECT id, start_month, end_month FROM budgets
             WHERE category_id = ?1 AND start_month <= ?2 AND (end_month IS NULL OR end_month >= ?2)
             ORDER BY start_month DESC LIMIT 1",
            params![category_id, m],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    match active {
        Some((row_id, start, _)) if start == m => {
            dbtx.execute(
                "UPDATE budgets SET amount_cents = ?1 WHERE id = ?2",
                params![amount_cents, row_id],
            )?;
        }
        Some((row_id, _, end)) => {
            if overlaps(&dbtx, &related, &m, end.as_deref())? {
                return Err(LedgerError::Conflict(
                    "the group or its subcategories already have a budget",
                ));
            }
            dbtx.execute(
                "UPDATE budgets SET end_month = ?1 WHERE id = ?2",
                params![prev_month_key(&m)?, row_id],
            )?;
            insert(&dbtx, category_id, amount_cents, &m, end.as_deref())?;
        }
        None => {
            // A later row of the same category (after a gap) bounds the new one.
            let next: Option<String> = dbtx.query_row(
                "SELECT min(start_month) FROM budgets WHERE category_id = ?1 AND start_month > ?2",
                params![category_id, m],
                |r| r.get(0),
            )?;
            let end = next.map(|n| prev_month_key(&n)).transpose()?;
            if overlaps(&dbtx, &related, &m, end.as_deref())? {
                return Err(LedgerError::Conflict(
                    "the group or its subcategories already have a budget",
                ));
            }
            insert(&dbtx, category_id, amount_cents, &m, end.as_deref())?;
        }
    }
    dbtx.commit()?;
    Ok(())
}

fn insert(
    conn: &Connection,
    category_id: i64,
    amount: i64,
    start: &str,
    end: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO budgets (category_id, amount_cents, start_month, end_month) VALUES (?1, ?2, ?3, ?4)",
        params![category_id, amount, start, end],
    )?;
    Ok(())
}

/// Removes the budget from `month` on: a row that starts then is deleted, an older one
/// is closed the month before (history is kept). `NotFound` when none is active.
pub fn remove_budget(conn: &Connection, category_id: i64, month: &str) -> Result<()> {
    let m = month_key(parse_month(month)?);
    let dbtx = conn.unchecked_transaction()?;
    let active: Option<(i64, String)> = dbtx
        .query_row(
            "SELECT id, start_month FROM budgets
             WHERE category_id = ?1 AND start_month <= ?2 AND (end_month IS NULL OR end_month >= ?2)
             ORDER BY start_month DESC LIMIT 1",
            params![category_id, m],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((row_id, start)) = active else {
        return Err(LedgerError::NotFound("budget"));
    };
    if start == m {
        dbtx.execute("DELETE FROM budgets WHERE id = ?1", [row_id])?;
    } else {
        dbtx.execute(
            "UPDATE budgets SET end_month = ?1 WHERE id = ?2",
            params![prev_month_key(&m)?, row_id],
        )?;
    }
    // "From this month on": rows scheduled for later months go too.
    dbtx.execute(
        "DELETE FROM budgets WHERE category_id = ?1 AND start_month > ?2",
        params![category_id, m],
    )?;
    dbtx.commit()?;
    Ok(())
}
