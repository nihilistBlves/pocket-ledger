//! Incremental bank sync.
//!
//! One execution is one `sync_runs` row; every linked account is fetched over a
//! window that overlaps the previous one, mapped to [`NewTransaction`] and handed to
//! the ingest pipeline, which dedups, so re-fetching is harmless. The work is split in
//! sync database steps ([`prepare_sync`], [`apply_account`], [`finish_run`]) and one
//! async network step ([`fetch_account`]); [`run_sync`] chains them locking the
//! `Mutex<Connection>` only inside the sync steps, never across an `.await`.
//!
//! Errors, run rows and summaries carry only short codes and counts, never content.

use std::sync::Mutex;
use std::time::Instant;

use chrono::{DateTime, Duration as Days, NaiveDate, Utc};
use rusqlite::{params, Connection};
use thiserror::Error;

use super::connect::{load_session, PROVIDER};
use super::db_timestamp;
use super::enable_banking::{
    Balance, CreditDebitIndicator, EbClient, EbError, NetworkKind, Sensitive, Transaction,
    TransactionStatus,
};
use crate::dedup::reconcile_stale_pending;
use crate::ingest::{ingest, IngestOrigin};
use crate::keystore::{KeystoreError, SecretStore};
use crate::model::{NewTransaction, TxSource, TxStatus};
use crate::money::{parse_amount, signed_from_indicator, DOT};

/// First sync of an account: how far back to look.
pub const INITIAL_LOOKBACK_DAYS: i64 = 90;
/// Later syncs start this many days before the last synced day.
pub const OVERLAP_DAYS: i64 = 5;

const AUTO_MIN_INTERVAL_HOURS: i64 = 6;
const AUTO_MAX_PER_24H: i64 = 4;
const MANUAL_GUARD_SECS: i64 = 60;
const RUNNING_STALE_MINUTES: i64 = 15;

/// Balance types in order of preference.
const BALANCE_PREFERENCE: [&str; 5] = ["CLAV", "ITAV", "ITBD", "CLBD", "XPCD"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTrigger {
    Manual,
    Auto,
}

impl SyncTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncTrigger::Manual => "manual",
            SyncTrigger::Auto => "auto",
        }
    }
}

/// Why a sync was refused before doing anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SyncBlocked {
    #[error("another sync is still running")]
    AlreadyRunning,
    #[error("an automatic sync ran less than 6 hours ago")]
    AutoTooSoon,
    #[error("the automatic sync limit for the last 24 hours was reached")]
    AutoDailyLimit,
    #[error("a sync started less than a minute ago")]
    ManualTooSoon,
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("no active bank connection")]
    NotConnected,
    #[error(transparent)]
    Blocked(#[from] SyncBlocked),
    #[error("the bank connection must be authorized again")]
    ReauthRequired,
    #[error("the bank rate limit was reached; try again later")]
    RateLimited,
    #[error("bank sync failed ({code})")]
    Bank { code: String },
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Store(#[from] KeystoreError),
    #[error("the database lock was poisoned")]
    Poisoned,
}

/// Counts only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub accounts: usize,
    pub inserted: usize,
    /// Rows rewritten or promoted from pending to booked.
    pub updated: usize,
    /// Pending rows in the fetched data.
    pub pending: usize,
    /// Rows ignored: unusable status, unparsable amount, missing date or rejected row.
    pub skipped: usize,
    pub transfers_matched: usize,
    pub duration_ms: u64,
}

/// An account to sync and its window.
#[derive(Debug, Clone)]
pub struct AccountPlan {
    pub account_id: i64,
    /// Provider account uid, valid for the current session only.
    pub uid: Sensitive<String>,
    pub date_from: NaiveDate,
    pub date_to: NaiveDate,
}

/// Output of [`prepare_sync`].
#[derive(Debug)]
pub struct SyncPlan {
    pub run_id: i64,
    pub today: NaiveDate,
    pub accounts: Vec<AccountPlan>,
}

/// Plain data fetched for one account.
#[derive(Debug)]
pub struct AccountFetch {
    pub balances: Vec<Balance>,
    pub transactions: Vec<Transaction>,
}

/// Counts for one account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountApplied {
    pub inserted: usize,
    pub updated: usize,
    pub pending: usize,
    pub skipped: usize,
    pub transfers_matched: usize,
}

/// Window to fetch: from `last_synced_to - overlap` (or the initial lookback) to today.
pub fn sync_window(last_synced_to: Option<NaiveDate>, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let from = match last_synced_to {
        Some(last) => last - Days::days(OVERLAP_DAYS),
        None => today - Days::days(INITIAL_LOOKBACK_DAYS),
    };
    (from.min(today), today)
}

/// Checks the sync policy against the recorded runs.
///
/// - never while a run is `running` and younger than 15 minutes;
/// - automatic: not within 6 hours of another automatic run, and at most 4 automatic
///   runs in 24 hours (failed runs count);
/// - manual: not within 60 seconds of any run (double-click guard).
pub fn sync_allowed(
    conn: &Connection,
    trigger: SyncTrigger,
    now: DateTime<Utc>,
) -> Result<(), SyncError> {
    let count = |sql: &str, since: DateTime<Utc>| -> rusqlite::Result<i64> {
        conn.query_row(sql, [db_timestamp(since)], |r| r.get(0))
    };
    let running = count(
        "SELECT count(*) FROM sync_runs WHERE status = 'running' AND started_at > ?1",
        now - Days::minutes(RUNNING_STALE_MINUTES),
    )?;
    if running > 0 {
        return Err(SyncBlocked::AlreadyRunning.into());
    }
    match trigger {
        SyncTrigger::Auto => {
            let recent = count(
                "SELECT count(*) FROM sync_runs WHERE \"trigger\" = 'auto' AND started_at > ?1",
                now - Days::hours(AUTO_MIN_INTERVAL_HOURS),
            )?;
            if recent > 0 {
                return Err(SyncBlocked::AutoTooSoon.into());
            }
            let day = count(
                "SELECT count(*) FROM sync_runs WHERE \"trigger\" = 'auto' AND started_at > ?1",
                now - Days::hours(24),
            )?;
            if day >= AUTO_MAX_PER_24H {
                return Err(SyncBlocked::AutoDailyLimit.into());
            }
        }
        SyncTrigger::Manual => {
            let recent = count(
                "SELECT count(*) FROM sync_runs WHERE started_at > ?1",
                now - Days::seconds(MANUAL_GUARD_SECS),
            )?;
            if recent > 0 {
                return Err(SyncBlocked::ManualTooSoon.into());
            }
        }
    }
    Ok(())
}

/// Sync step 1 (database): checks the connection and the policy, computes each
/// account's window and records the run as `running`. Makes no HTTP call.
pub fn prepare_sync(
    conn: &mut Connection,
    store: &dyn SecretStore,
    trigger: SyncTrigger,
    now: DateTime<Utc>,
) -> Result<SyncPlan, SyncError> {
    match load_session(store)? {
        Some(s) if s.valid_until > now => {}
        _ => return Err(SyncError::NotConnected),
    }

    // A run that never finished (crash) must not block syncing forever.
    conn.execute(
        "UPDATE sync_runs SET status = 'error', error_code = 'ABORTED', finished_at = ?1
         WHERE status = 'running' AND started_at <= ?2",
        params![db_timestamp(now), db_timestamp(now - Days::minutes(RUNNING_STALE_MINUTES))],
    )?;
    sync_allowed(conn, trigger, now)?;

    let today = now.date_naive();
    let accounts = linked_accounts(conn, today)?;
    if accounts.is_empty() {
        return Err(SyncError::NotConnected);
    }
    conn.execute(
        "INSERT INTO sync_runs (provider, started_at, status, \"trigger\")
         VALUES (?1, ?2, 'running', ?3)",
        params![PROVIDER, db_timestamp(now), trigger.as_str()],
    )?;
    Ok(SyncPlan { run_id: conn.last_insert_rowid(), today, accounts })
}

fn linked_accounts(conn: &Connection, today: NaiveDate) -> rusqlite::Result<Vec<AccountPlan>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, l.external_uid, s.last_synced_to
         FROM account_links l
         JOIN accounts a ON a.id = l.account_id
         LEFT JOIN account_sync_state s ON s.account_id = a.id
         WHERE l.provider = ?1 AND a.archived = 0
         ORDER BY a.id",
    )?;
    let rows = stmt.query_map([PROVIDER], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
    })?;
    let mut plans = Vec::new();
    for row in rows {
        let (account_id, uid, last) = row?;
        let last = last.and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok());
        let (date_from, date_to) = sync_window(last, today);
        plans.push(AccountPlan { account_id, uid: Sensitive::new(uid), date_from, date_to });
    }
    Ok(plans)
}

/// Sync step 2 (network): balances and every transaction of the window, any status.
pub async fn fetch_account(client: &EbClient, plan: &AccountPlan) -> Result<AccountFetch, EbError> {
    let balances = client.account_balances(plan.uid.expose()).await?;
    let transactions = client
        .account_transactions(plan.uid.expose(), plan.date_from, plan.date_to, None)
        .await?;
    Ok(AccountFetch { balances, transactions })
}

/// Result of mapping one API transaction.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Mapped {
    Row(NewTransaction),
    /// Not stored: unusable status, bad amount or no date. Carries no content.
    Skipped,
}

pub(crate) fn map_transaction(tx: &Transaction) -> Mapped {
    let status = match tx.status {
        TransactionStatus::Book => TxStatus::Booked,
        TransactionStatus::Pdng | TransactionStatus::Hold => TxStatus::Pending,
        TransactionStatus::Cncl
        | TransactionStatus::Rjct
        | TransactionStatus::Schd
        | TransactionStatus::Othr => return Mapped::Skipped,
    };

    // The credit/debit indicator is authoritative; the amount's own sign is ignored.
    let amount = tx.transaction_amount.expose();
    let unsigned = amount.amount.trim();
    let unsigned = unsigned.strip_prefix('-').unwrap_or(unsigned);
    let indicator = match tx.credit_debit_indicator {
        CreditDebitIndicator::Crdt => "CRDT",
        CreditDebitIndicator::Dbit => "DBIT",
    };
    let Ok(amount_cents) =
        parse_amount(unsigned, DOT).and_then(|c| signed_from_indicator(c, indicator))
    else {
        return Mapped::Skipped;
    };

    let date = |d: Option<NaiveDate>| d.map(|d| d.to_string());
    let (booking_date, value_date) = match status {
        TxStatus::Booked => {
            let booking = tx.booking_date.or(tx.value_date).or(tx.transaction_date);
            match booking {
                Some(_) => (date(booking), date(tx.value_date)),
                None => return Mapped::Skipped,
            }
        }
        TxStatus::Pending => {
            let value = tx.value_date.or(tx.transaction_date).or(tx.booking_date);
            match value {
                Some(_) => (None, date(value)),
                None => return Mapped::Skipped,
            }
        }
    };

    let party = match tx.credit_debit_indicator {
        CreditDebitIndicator::Dbit => tx.creditor.as_ref(),
        CreditDebitIndicator::Crdt => tx.debtor.as_ref(),
    };
    let counterparty =
        party.and_then(|p| clean(p.expose().name.as_ref().map(|n| n.expose().as_str())));
    let joined = tx
        .remittance_information
        .expose()
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let remittance = clean(Some(&joined));
    let note = clean(tx.note.as_ref().map(|n| n.expose().as_str()));
    let description = remittance
        .clone()
        .or_else(|| counterparty.clone())
        .or(note)
        .unwrap_or_default();

    Mapped::Row(NewTransaction {
        status,
        booking_date,
        value_date,
        amount_cents,
        currency: amount.currency.clone(),
        description,
        counterparty,
        remittance,
        // Never `transaction_id`: it is not stable.
        entry_reference: clean(tx.entry_reference.as_ref().map(|r| r.expose().as_str())),
    })
}

fn clean(text: Option<&str>) -> Option<String> {
    text.map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned)
}

/// Picks the balance to show: by [`BALANCE_PREFERENCE`], else the first one.
pub(crate) fn pick_balance(balances: &[Balance]) -> Option<&Balance> {
    balances.iter().min_by_key(|b| {
        BALANCE_PREFERENCE
            .iter()
            .position(|p| *p == b.balance_type)
            .unwrap_or(BALANCE_PREFERENCE.len())
    })
}

/// Sync step 3 (database): maps and ingests one account's data, removes pending rows
/// the bank no longer reports, stores the latest balance and moves the sync window.
pub fn apply_account(
    conn: &mut Connection,
    run_id: i64,
    plan: &AccountPlan,
    fetch: &AccountFetch,
    now: DateTime<Utc>,
) -> rusqlite::Result<AccountApplied> {
    let mut applied = AccountApplied::default();
    let mut rows = Vec::with_capacity(fetch.transactions.len());
    for tx in &fetch.transactions {
        match map_transaction(tx) {
            Mapped::Row(row) => {
                if row.status == TxStatus::Pending {
                    applied.pending += 1;
                }
                rows.push(row);
            }
            Mapped::Skipped => applied.skipped += 1,
        }
    }

    let report =
        ingest(conn, plan.account_id, rows, TxSource::Bank, IngestOrigin::SyncRun(run_id))?;
    applied.inserted = report.inserted;
    applied.updated = report.updated + report.promoted;
    applied.skipped += report.errors.len();
    applied.transfers_matched = report.transfers_matched;

    let db_tx = conn.transaction()?;
    reconcile_stale_pending(
        &db_tx,
        plan.account_id,
        &plan.date_from.to_string(),
        &plan.date_to.to_string(),
        &report.seen_ids,
    )?;
    if let Some(balance) = pick_balance(&fetch.balances) {
        let amount = balance.balance_amount.expose();
        if let Ok(cents) = parse_amount(&amount.amount, DOT) {
            db_tx.execute(
                "INSERT INTO account_balances
                     (account_id, amount_cents, currency, balance_type, reference_date, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id) DO UPDATE SET
                     amount_cents = excluded.amount_cents, currency = excluded.currency,
                     balance_type = excluded.balance_type, reference_date = excluded.reference_date,
                     fetched_at = excluded.fetched_at",
                params![
                    plan.account_id,
                    cents,
                    amount.currency,
                    balance.balance_type,
                    balance.reference_date.map(|d| d.to_string()),
                    db_timestamp(now)
                ],
            )?;
        }
    }
    db_tx.execute(
        "INSERT INTO account_sync_state (account_id, last_synced_to, last_success_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(account_id) DO UPDATE SET
             last_synced_to = excluded.last_synced_to, last_success_at = excluded.last_success_at",
        params![plan.account_id, plan.date_to.to_string(), db_timestamp(now)],
    )?;
    db_tx.commit()?;
    Ok(applied)
}

/// Closes the run row: `Ok(())` marks it `ok`, `Err(code)` marks it `error`.
pub fn finish_run(
    conn: &Connection,
    run_id: i64,
    outcome: Result<(), &str>,
    now: DateTime<Utc>,
) -> rusqlite::Result<()> {
    let (status, code) = match outcome {
        Ok(()) => ("ok", None),
        Err(code) => ("error", Some(code)),
    };
    conn.execute(
        "UPDATE sync_runs SET status = ?1, error_code = ?2, finished_at = ?3 WHERE id = ?4",
        params![status, code, db_timestamp(now), run_id],
    )?;
    Ok(())
}

/// Short, content-free code for a client error (stored in `sync_runs.error_code`).
pub fn error_code(e: &EbError) -> String {
    match e {
        EbError::Api { code, .. } => code.to_string(),
        EbError::Network { kind: NetworkKind::Timeout } => "NETWORK_TIMEOUT".into(),
        EbError::Network { kind: NetworkKind::Connect } => "NETWORK_CONNECT".into(),
        EbError::Network { kind: NetworkKind::Other } => "NETWORK_OTHER".into(),
        EbError::Decode { .. } => "DECODE_ERROR".into(),
        EbError::PaginationLoop | EbError::PaginationLimit => "PAGINATION".into(),
        EbError::KeyNotConfigured | EbError::KeyFileNotFound | EbError::KeyInvalid => {
            "KEY_ERROR".into()
        }
        EbError::InvalidConfig => "INVALID_CONFIG".into(),
    }
}

fn classify(e: &EbError) -> SyncError {
    if e.requires_reauthorization() {
        SyncError::ReauthRequired
    } else if e.is_rate_limited() {
        SyncError::RateLimited
    } else {
        SyncError::Bank { code: error_code(e) }
    }
}

/// Runs a whole sync. The connection is locked only inside the synchronous steps.
/// On the first error the run is closed as `error` and the sync stops; accounts
/// already synced stay committed.
pub async fn run_sync(
    db: &Mutex<Connection>,
    client: &EbClient,
    store: &dyn SecretStore,
    trigger: SyncTrigger,
    now: DateTime<Utc>,
) -> Result<SyncSummary, SyncError> {
    let started = Instant::now();
    let plan = {
        let mut conn = db.lock().map_err(|_| SyncError::Poisoned)?;
        prepare_sync(&mut conn, store, trigger, now)?
    };

    let mut summary = SyncSummary::default();
    for account in &plan.accounts {
        let step = match fetch_account(client, account).await {
            Ok(fetch) => {
                let mut conn = db.lock().map_err(|_| SyncError::Poisoned)?;
                apply_account(&mut conn, plan.run_id, account, &fetch, now)
                    .map_err(|e| (SyncError::Db(e), "DB_ERROR".to_owned()))
            }
            Err(e) => Err((classify(&e), error_code(&e))),
        };
        match step {
            Ok(applied) => {
                summary.accounts += 1;
                summary.inserted += applied.inserted;
                summary.updated += applied.updated;
                summary.pending += applied.pending;
                summary.skipped += applied.skipped;
                summary.transfers_matched += applied.transfers_matched;
            }
            Err((err, code)) => {
                let conn = db.lock().map_err(|_| SyncError::Poisoned)?;
                finish_run(&conn, plan.run_id, Err(&code), Utc::now())?;
                return Err(err);
            }
        }
    }
    {
        let conn = db.lock().map_err(|_| SyncError::Poisoned)?;
        finish_run(&conn, plan.run_id, Ok(()), Utc::now())?;
    }
    summary.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(summary)
}

#[cfg(test)]
mod tests;
