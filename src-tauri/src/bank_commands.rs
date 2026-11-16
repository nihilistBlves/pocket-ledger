//! Tauri commands of the "Cuentas y conexión" screen. They return plain DTOs and
//! [`CommandError`]; no session id, authorization code, state, account uid, IBAN,
//! holder name, key path or authorization URL ever crosses to the UI.
//!
//! The std `Mutex<Connection>` is locked only in synchronous blocks, never across an
//! `.await`; one connect / sync / disconnect at a time is enforced by the busy guard.

use chrono::{DateTime, Utc};
use ledger_core::bank::connect::{
    authorize, register_session, revoke, revoke_previous, unlink_accounts,
    AuthParams, SESSION_ENTRY,
};
use ledger_core::bank::overview::{
    self, AccountView, ConnectionView, RunView, HISTORY_LIMIT,
};
use ledger_core::bank::sync::{run_sync, SyncSummary, SyncTrigger};
use ledger_core::keystore::SecretStore;
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;

use crate::bank_config::{self, BankConfig};
use crate::errors::{CommandError, ErrorCode};
use crate::state::AppState;

/// Emitted after a sync run that touched the database, so the UI can refresh.
pub const SYNCED_EVENT: &str = "bank://synced";

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct BankOverview {
    pub configured: bool,
    pub aspsp_name: Option<String>,
    pub connection: ConnectionView,
    pub accounts: Vec<AccountView>,
    pub runs: Vec<RunView>,
    pub next_auto_sync_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ConnectResult {
    pub accounts_linked: usize,
    pub valid_until: DateTime<Utc>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SyncResult {
    pub accounts: usize,
    pub inserted: usize,
    pub updated: usize,
    pub pending: usize,
    pub skipped: usize,
    pub duration_ms: u64,
}

impl From<SyncSummary> for SyncResult {
    fn from(s: SyncSummary) -> Self {
        Self {
            accounts: s.accounts,
            inserted: s.inserted,
            updated: s.updated,
            pending: s.pending,
            skipped: s.skipped,
            duration_ms: s.duration_ms,
        }
    }
}

/// Pure assembly of the overview (testable without Tauri, keyring or network).
pub fn build_overview(
    conn: &Connection,
    store: &dyn SecretStore,
    config: Option<&BankConfig>,
    now: DateTime<Utc>,
) -> Result<BankOverview, CommandError> {
    let connection = overview::connection_view(store, now)?;
    let aspsp_name = config.map(|c| c.aspsp_name.clone()).or_else(|| match &connection {
        ConnectionView::Connected { aspsp_name, .. } | ConnectionView::Expired { aspsp_name, .. } => {
            Some(aspsp_name.clone())
        }
        ConnectionView::NotConnected => None,
    });
    let next_auto_sync_at = match (&connection, config) {
        (ConnectionView::Connected { .. }, Some(_)) => Some(overview::next_auto_sync_at(conn, now)?),
        _ => None,
    };
    Ok(BankOverview {
        configured: config.is_some(),
        aspsp_name,
        connection,
        accounts: overview::accounts(conn)?,
        runs: overview::recent_runs(conn, HISTORY_LIMIT)?,
        next_auto_sync_at,
    })
}

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, CommandError> {
    state.require_ready()?;
    state.db.lock().map_err(|_| CommandError::new(ErrorCode::Internal))
}

#[tauri::command]
pub async fn bank_overview(state: State<'_, AppState>) -> Result<BankOverview, CommandError> {
    let config = bank_config::load(&state.data_dir).ok();
    let conn = lock(&state)?;
    build_overview(&conn, state.store.as_ref(), config.as_ref(), Utc::now())
}

/// Authorizes with the bank in the system browser and stores the session. Also the
/// "renew consent" flow: the previous session is revoked best-effort afterwards.
#[tauri::command]
pub async fn bank_connect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ConnectResult, CommandError> {
    let _busy = state.try_busy()?;
    let config = bank_config::load(&state.data_dir)?;
    let client = config.client()?;

    let opener = app.clone();
    let session = authorize(
        &client,
        AuthParams::new(config.aspsp_name.clone(), config.aspsp_country.clone()),
        // The system browser, never a WebView. The error text is dropped (it could echo the URL).
        move |url| opener.opener().open_url(url, None::<&str>).map_err(|_| String::new()),
    )
    .await?;

    let outcome = {
        let mut conn = lock(&state)?;
        register_session(
            &mut conn,
            state.store.as_ref(),
            &config.aspsp_name,
            &config.aspsp_country,
            &session,
            Utc::now(),
        )?
    };
    if let Some(previous) = &outcome.previous_session_id {
        // Best effort: the new session is already stored.
        let _ = revoke_previous(&client, previous).await;
    }
    Ok(ConnectResult { accounts_linked: outcome.accounts_linked, valid_until: outcome.valid_until })
}

/// Runs a sync under the busy guard. `Busy` is returned (not waited for) when another
/// bank operation is running.
pub async fn sync_with(
    app: &AppHandle,
    state: &AppState,
    trigger: SyncTrigger,
) -> Result<SyncResult, CommandError> {
    let _busy = state.try_busy()?;
    let config = bank_config::load(&state.data_dir)?;
    let client = config.client()?;
    let result = run_sync(&state.db, &client, state.store.as_ref(), trigger, Utc::now()).await;
    match result {
        Ok(summary) => {
            let _ = app.emit(SYNCED_EVENT, ());
            Ok(summary.into())
        }
        Err(e) => {
            let error: CommandError = e.into();
            // A failed run is recorded in the history: refresh it too.
            if error.code != ErrorCode::NotConnected.as_str()
                && error.code != ErrorCode::BlockedTooSoon.as_str()
                && error.code != ErrorCode::Busy.as_str()
            {
                let _ = app.emit(SYNCED_EVENT, ());
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn bank_sync_now(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SyncResult, CommandError> {
    sync_with(&app, &state, SyncTrigger::Manual).await
}

/// Revokes the session at the bank (best effort) and removes the local connection.
/// Accounts and imported transactions are kept.
#[tauri::command]
pub async fn bank_disconnect(state: State<'_, AppState>) -> Result<(), CommandError> {
    let _busy = state.try_busy()?;
    match bank_config::load(&state.data_dir).map_err(CommandError::from).and_then(|c| c.client()) {
        Ok(client) => {
            revoke(&client, state.store.as_ref()).await?;
        }
        // Without a usable configuration the bank side can not be reached: still
        // forget the session locally.
        Err(_) => state.store.delete(SESSION_ENTRY)?,
    }
    let conn = lock(&state)?;
    unlink_accounts(&conn)?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use ledger_core::bank::connect::PROVIDER;
    use ledger_core::db::{open_in_memory, DbKey, KEY_LEN};
    use ledger_core::keystore::MemoryStore;

    const FAKE_SESSION: &str = "fake-session-id-must-not-leak";
    const FAKE_UID: &str = "fake-uid-must-not-leak";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 15, 10, 0, 0).unwrap()
    }

    fn config() -> BankConfig {
        bank_config::parse(r#"{"application_id":"app","aspsp_name":"Mock Bank"}"#).unwrap()
    }

    fn db() -> Connection {
        open_in_memory(&DbKey::from_bytes([0x22; KEY_LEN])).unwrap()
    }

    fn store_session(store: &MemoryStore, valid_until: DateTime<Utc>) {
        let json = serde_json::json!({
            "session_id": FAKE_SESSION,
            "valid_until": valid_until,
            "aspsp_name": "Mock Bank",
            "aspsp_country": "ES",
            "connected_at": now() - Duration::days(30),
        });
        store.set(SESSION_ENTRY, json.to_string().as_bytes()).unwrap();
    }

    fn add_account(conn: &Connection) {
        conn.execute(
            "INSERT INTO accounts (name, kind, currency, identity_hash, display_suffix)
             VALUES ('Cuenta ficticia', 'checking', 'EUR', 'hash-1', '1234')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO account_links (account_id, provider, external_uid) VALUES (1, ?1, ?2)",
            rusqlite::params![PROVIDER, FAKE_UID],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO account_balances (account_id, amount_cents, currency, balance_type, fetched_at)
             VALUES (1, 318452, 'EUR', 'CLBD', '2026-11-15T09:00:00.000Z')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn bank_database_access_is_refused_in_recovery_mode() {
        use crate::state::AppMode;
        use ledger_core::backup::RecoveryReason;
        let state = AppState::new(db(), Box::new(MemoryStore::new()), std::path::PathBuf::new());
        assert!(lock(&state).is_ok());
        *state.mode.write().unwrap() = AppMode::Recovery(RecoveryReason::KeyWrong);
        assert_eq!(lock(&state).unwrap_err().code, "recovery_mode");
        assert_eq!(state.try_busy().unwrap_err().code, "recovery_mode");
    }

    #[test]
    fn not_configured_and_not_connected() {
        let conn = db();
        let o = build_overview(&conn, &MemoryStore::new(), None, now()).unwrap();
        assert!(!o.configured);
        assert_eq!(o.aspsp_name, None);
        assert_eq!(o.connection, ConnectionView::NotConnected);
        assert_eq!(o.next_auto_sync_at, None);
        assert!(o.accounts.is_empty() && o.runs.is_empty());
    }

    #[test]
    fn connected_overview_has_accounts_and_next_sync() {
        let conn = db();
        add_account(&conn);
        let store = MemoryStore::new();
        store_session(&store, now() + Duration::days(150));
        let cfg = config();
        let o = build_overview(&conn, &store, Some(&cfg), now()).unwrap();
        assert!(o.configured);
        assert_eq!(o.aspsp_name.as_deref(), Some("Mock Bank"));
        match o.connection {
            ConnectionView::Connected { days_remaining, consent_days_total, .. } => {
                assert_eq!((days_remaining, consent_days_total), (150, 180));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(o.accounts.len(), 1);
        assert_eq!(o.accounts[0].balance_cents, Some(318_452));
        assert_eq!(o.next_auto_sync_at, Some(now()));
    }

    #[test]
    fn expired_connection_has_no_next_sync_and_keeps_the_bank_name_without_config() {
        let conn = db();
        let store = MemoryStore::new();
        store_session(&store, now() - Duration::days(1));
        let o = build_overview(&conn, &store, None, now()).unwrap();
        assert!(matches!(o.connection, ConnectionView::Expired { .. }));
        assert_eq!(o.aspsp_name.as_deref(), Some("Mock Bank"));
        assert_eq!(o.next_auto_sync_at, None);
    }

    #[test]
    fn serialized_overview_has_the_documented_shape_and_no_secrets() {
        let conn = db();
        add_account(&conn);
        conn.execute(
            "INSERT INTO sync_runs (provider, started_at, finished_at, status, \"trigger\", inserted, updated, error_code)
             VALUES ('enable_banking', '2026-11-15T08:00:00.000Z', '2026-11-15T08:00:03.800Z', 'error', 'auto', 0, 0, 'ASPSP_RATE_LIMIT_EXCEEDED')",
            [],
        )
        .unwrap();
        let store = MemoryStore::new();
        store_session(&store, now() + Duration::days(9));
        let o = build_overview(&conn, &store, Some(&config()), now()).unwrap();
        let text = serde_json::to_string(&o).unwrap();
        assert!(!text.contains(FAKE_SESSION) && !text.contains(FAKE_UID));
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["connection"]["kind"], "connected");
        assert_eq!(v["connection"]["days_remaining"], 9);
        assert!(v["connection"]["granted_on"].is_string());
        assert_eq!(v["accounts"][0]["display_suffix"], "1234");
        assert_eq!(v["accounts"][0]["balance_cents"], 318_452);
        assert_eq!(v["runs"][0]["trigger"], "auto");
        assert_eq!(v["runs"][0]["duration_ms"], 3800);
        assert_eq!(v["runs"][0]["error_code"], "ASPSP_RATE_LIMIT_EXCEEDED");
        assert!(v["next_auto_sync_at"].is_string());
    }

    #[test]
    fn sync_result_copies_counts_only() {
        let r: SyncResult = SyncSummary { accounts: 2, inserted: 3, updated: 1, duration_ms: 9, ..Default::default() }.into();
        assert_eq!((r.accounts, r.inserted, r.updated, r.duration_ms), (2, 3, 1, 9));
    }
}
