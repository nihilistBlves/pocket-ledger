//! Read-only queries behind the "Cuentas y conexión" screen: the connection state,
//! the accounts with their latest bank balance and the last sync runs.
//!
//! Everything here is safe to show: no session id, no account uid, no IBAN (only the
//! 4-character display suffix stored with the account) and no error text (sync runs
//! carry short codes only).

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;
use serde::Serialize;

use super::connect::{load_session, MAX_VALIDITY_DAYS_CAP};
use crate::keystore::{KeystoreError, SecretStore};

/// Sync runs shown in the history.
pub const HISTORY_LIMIT: usize = 5;
/// Minimum gap between automatic runs (mirrors the sync policy).
const AUTO_MIN_INTERVAL_HOURS: i64 = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionView {
    NotConnected,
    Connected {
        aspsp_name: String,
        valid_until: DateTime<Utc>,
        days_remaining: i64,
        consent_days_total: i64,
        granted_on: DateTime<Utc>,
    },
    Expired {
        aspsp_name: String,
        valid_until: DateTime<Utc>,
        consent_days_total: i64,
        granted_on: DateTime<Utc>,
    },
}

/// Connection state with the consent dates. Never fails on a corrupt session entry
/// (that reads as not connected, like everywhere else).
pub fn connection_view(
    store: &dyn SecretStore,
    now: DateTime<Utc>,
) -> Result<ConnectionView, KeystoreError> {
    let Some(s) = load_session(store)? else {
        return Ok(ConnectionView::NotConnected);
    };
    let total = (s.valid_until - s.connected_at).num_days().clamp(1, i64::from(MAX_VALIDITY_DAYS_CAP));
    Ok(if s.valid_until <= now {
        ConnectionView::Expired {
            aspsp_name: s.aspsp_name.clone(),
            valid_until: s.valid_until,
            consent_days_total: total,
            granted_on: s.connected_at,
        }
    } else {
        ConnectionView::Connected {
            aspsp_name: s.aspsp_name.clone(),
            valid_until: s.valid_until,
            days_remaining: (s.valid_until - now).num_days(),
            consent_days_total: total,
            granted_on: s.connected_at,
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountView {
    pub id: i64,
    pub name: String,
    /// `checking`, `savings`, `card`, `cash` or `other`.
    pub kind: String,
    /// Last 4 characters of the IBAN, when known.
    pub display_suffix: Option<String>,
    pub balance_cents: Option<i64>,
    pub currency: String,
    /// UTC timestamp of the last successful sync of this account.
    pub last_synced_at: Option<String>,
}

/// Non-archived accounts of the user with the latest bank balance.
pub fn accounts(conn: &Connection) -> rusqlite::Result<Vec<AccountView>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, a.kind, a.display_suffix, b.amount_cents, a.currency, s.last_success_at
         FROM accounts a
         LEFT JOIN account_balances b ON b.account_id = a.id
         LEFT JOIN account_sync_state s ON s.account_id = a.id
         WHERE a.archived = 0 AND a.is_own = 1
         ORDER BY a.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(AccountView {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: r.get(2)?,
            display_suffix: r.get(3)?,
            balance_cents: r.get(4)?,
            currency: r.get(5)?,
            last_synced_at: r.get(6)?,
        })
    })?;
    rows.collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunView {
    pub started_at: String,
    /// `manual` or `auto`.
    pub trigger: String,
    /// `running`, `ok` or `error`.
    pub status: String,
    pub inserted: i64,
    pub updated: i64,
    pub error_code: Option<String>,
    pub duration_ms: Option<i64>,
}

/// The most recent runs, newest first.
pub fn recent_runs(conn: &Connection, limit: usize) -> rusqlite::Result<Vec<RunView>> {
    let mut stmt = conn.prepare(
        "SELECT started_at, \"trigger\", status, inserted, updated, error_code,
                CASE WHEN finished_at IS NULL THEN NULL
                     ELSE CAST(ROUND((julianday(finished_at) - julianday(started_at)) * 86400000) AS INTEGER)
                END
         FROM sync_runs ORDER BY started_at DESC, id DESC LIMIT ?1",
    )?;
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = stmt.query_map([limit], |r| {
        Ok(RunView {
            started_at: r.get(0)?,
            trigger: r.get(1)?,
            status: r.get(2)?,
            inserted: r.get(3)?,
            updated: r.get(4)?,
            error_code: r.get(5)?,
            duration_ms: r.get::<_, Option<i64>>(6)?.map(|d| d.max(0)),
        })
    })?;
    rows.collect()
}

/// Earliest moment the next automatic sync can run according to the 6 hour spacing
/// (the app checks every 30 minutes, so the real run is at most that much later).
/// `now` when nothing prevents it.
pub fn next_auto_sync_at(
    conn: &Connection,
    now: DateTime<Utc>,
) -> rusqlite::Result<DateTime<Utc>> {
    let last: Option<String> = conn.query_row(
        "SELECT max(started_at) FROM sync_runs WHERE \"trigger\" = 'auto'",
        [],
        |r| r.get(0),
    )?;
    let Some(last) = last.and_then(|t| DateTime::parse_from_rfc3339(&t).ok()) else {
        return Ok(now);
    };
    let earliest = last.with_timezone(&Utc) + Duration::hours(AUTO_MIN_INTERVAL_HOURS);
    Ok(earliest.max(now))
}


#[cfg(test)]
mod tests;
