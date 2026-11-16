//! The "Resumen" screen: totals of the month, comparison period, 12-month evolution,
//! spending by top-level category, account balances and the latest rows.

use std::collections::HashMap;

use rusqlite::types::Value;
use rusqlite::Connection;
use serde::Serialize;

use super::period::{iso, month_key, periods, twelve_months};
use super::transactions::query_rows;
use super::{PeriodTotals, Result, TxRow, EFFECTIVE_DATE};

/// Rows shown under "Últimos movimientos".
pub const RECENT_LIMIT: i64 = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DateRange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthTotals {
    pub month: String,
    #[serde(flatten)]
    pub totals: PeriodTotals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorySpend {
    /// Top-level category; `None` for "Sin categoría".
    pub category_id: Option<i64>,
    pub name: String,
    pub key: Option<String>,
    pub expense_cents: i64,
    pub tx_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountBalance {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub display_suffix: Option<String>,
    /// Latest balance reported by the bank, not a sum of transactions.
    pub balance_cents: Option<i64>,
    pub currency: String,
    pub balance_date: Option<String>,
    pub last_synced_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub month: String,
    pub is_current_month: bool,
    pub through_day: Option<u32>,
    pub totals: PeriodTotals,
    pub previous: PeriodTotals,
    pub previous_label_range: DateRange,
    pub evolution: Vec<MonthTotals>,
    pub categories: Vec<CategorySpend>,
    pub accounts: Vec<AccountBalance>,
    pub recent: Vec<TxRow>,
    pub month_tx_count: i64,
}

/// Shared FROM/WHERE of every aggregate: own accounts, effective date in `?1..=?2`,
/// internal transfers left out.
const AGG_FROM: &str = "FROM transactions t
     JOIN accounts a ON a.id = t.account_id
     LEFT JOIN categories c ON c.id = t.category_id";

fn totals(conn: &Connection, from: &str, to: &str) -> Result<PeriodTotals> {
    let (income, expense): (i64, i64) = conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(CASE WHEN t.amount_cents > 0 THEN t.amount_cents END), 0),
                    COALESCE(SUM(CASE WHEN t.amount_cents < 0 THEN -t.amount_cents END), 0)
             {AGG_FROM}
             WHERE a.is_own = 1 AND COALESCE(c.kind, '') <> 'transfer'
               AND {EFFECTIVE_DATE} BETWEEN ?1 AND ?2"
        ),
        [from, to],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(PeriodTotals::new(income, expense))
}

fn evolution(conn: &Connection, last_month_start: chrono::NaiveDate, current: PeriodTotals) -> Result<Vec<MonthTotals>> {
    let months = twelve_months(last_month_start);
    let (Some(first), Some(last)) = (months.first(), months.last()) else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT substr({EFFECTIVE_DATE}, 1, 7) AS m,
                COALESCE(SUM(CASE WHEN t.amount_cents > 0 THEN t.amount_cents END), 0),
                COALESCE(SUM(CASE WHEN t.amount_cents < 0 THEN -t.amount_cents END), 0)
         {AGG_FROM}
         WHERE a.is_own = 1 AND COALESCE(c.kind, '') <> 'transfer'
           AND {EFFECTIVE_DATE} BETWEEN ?1 AND ?2
         GROUP BY m"
    ))?;
    let mut by_month: HashMap<String, PeriodTotals> = HashMap::new();
    let rows = stmt.query_map([iso(*first), iso(super::period::last_day_of_month(*last))], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
    })?;
    for row in rows {
        let (m, income, expense) = row?;
        by_month.insert(m, PeriodTotals::new(income, expense));
    }
    let last_key = month_key(*last);
    Ok(months
        .iter()
        .map(|d| {
            let key = month_key(*d);
            // The selected month uses the same range as the headline totals.
            let totals = if key == last_key {
                current
            } else {
                by_month.get(&key).copied().unwrap_or_else(|| PeriodTotals::new(0, 0))
            };
            MonthTotals { month: key, totals }
        })
        .collect())
}

fn category_spend(conn: &Connection, from: &str, to: &str) -> Result<Vec<CategorySpend>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT g.id, COALESCE(g.name, 'Sin categoría'), g.key, SUM(g.cents), count(*)
         FROM (
           SELECT CASE WHEN c.id IS NULL OR c.key = 'uncategorized' THEN NULL ELSE top.id END AS id,
                  CASE WHEN c.id IS NULL OR c.key = 'uncategorized' THEN NULL ELSE top.name END AS name,
                  CASE WHEN c.id IS NULL OR c.key = 'uncategorized' THEN NULL ELSE top.key END AS key,
                  -t.amount_cents AS cents
           {AGG_FROM}
           LEFT JOIN categories top ON top.id = COALESCE(c.parent_id, c.id)
           WHERE a.is_own = 1 AND COALESCE(c.kind, '') <> 'transfer' AND t.amount_cents < 0
             AND {EFFECTIVE_DATE} BETWEEN ?1 AND ?2
         ) g
         GROUP BY g.id
         ORDER BY SUM(g.cents) DESC, COALESCE(g.name, 'Sin categoría')"
    ))?;
    let rows = stmt
        .query_map([from, to], |r| {
            Ok(CategorySpend {
                category_id: r.get(0)?,
                name: r.get(1)?,
                key: r.get(2)?,
                expense_cents: r.get(3)?,
                tx_count: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn account_balances(conn: &Connection) -> Result<Vec<AccountBalance>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, a.kind, a.display_suffix, b.amount_cents, a.currency,
                b.reference_date, s.last_success_at
         FROM accounts a
         LEFT JOIN account_balances b ON b.account_id = a.id
         LEFT JOIN account_sync_state s ON s.account_id = a.id
         WHERE a.archived = 0 AND a.is_own = 1
         ORDER BY a.id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(AccountBalance {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                display_suffix: r.get(3)?,
                balance_cents: r.get(4)?,
                currency: r.get(5)?,
                balance_date: r.get(6)?,
                last_synced_at: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Dashboard of `month` (`YYYY-MM`) as seen on `today` (`YYYY-MM-DD`).
///
/// Totals, categories and the last evolution point run to `today` for the current
/// month. The recent rows and `month_tx_count` cover the whole calendar month (that is
/// what "Ver todos" opens), transfers included.
pub fn dashboard(conn: &Connection, month: &str, today: &str) -> Result<Dashboard> {
    let p = periods(month, today)?;
    let (from, to) = (iso(p.from), iso(p.to));
    let current = totals(conn, &from, &to)?;
    let (month_from, month_to) = (iso(p.month_start), iso(p.month_end));

    let month_where = format!(
        "WHERE a.is_own = 1 AND {EFFECTIVE_DATE} BETWEEN ?1 AND ?2"
    );
    let month_tx_count: i64 = conn.query_row(
        &format!("SELECT count(*) {AGG_FROM} {month_where}"),
        [&month_from, &month_to],
        |r| r.get(0),
    )?;
    let recent = query_rows(
        conn,
        &format!("{month_where} ORDER BY {EFFECTIVE_DATE} DESC, t.id DESC LIMIT ?3"),
        vec![Value::Text(month_from), Value::Text(month_to), Value::Integer(RECENT_LIMIT)],
    )?;

    Ok(Dashboard {
        month: month_key(p.month_start),
        is_current_month: p.is_current,
        through_day: p.through_day,
        totals: current,
        previous: totals(conn, &iso(p.prev_from), &iso(p.prev_to))?,
        previous_label_range: DateRange { from: iso(p.prev_from), to: iso(p.prev_to) },
        evolution: evolution(conn, p.month_start, current)?,
        categories: category_spend(conn, &from, &to)?,
        accounts: account_balances(conn)?,
        recent,
        month_tx_count,
    })
}
