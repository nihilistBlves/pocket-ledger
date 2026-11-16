//! Background automatic sync. Every 30 minutes it tries a sync; the core policy
//! decides whether it may run (at least 6 hours between automatic runs, at most 4 per
//! 24 hours) and refuses silently otherwise.

use std::time::Duration;

use ledger_core::bank::sync::SyncTrigger;
use tauri::{AppHandle, Manager};

use crate::bank_commands::sync_with;
use crate::errors::ErrorCode;
use crate::state::AppState;

const CHECK_EVERY: Duration = Duration::from_secs(30 * 60);
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(60);

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let start = tokio::time::Instant::now() + FIRST_CHECK_AFTER;
        let mut ticker = tokio::time::interval_at(start, CHECK_EVERY);
        loop {
            ticker.tick().await;
            let state = app.state::<AppState>();
            if let Err(e) = sync_with(&app, &state, SyncTrigger::Auto).await {
                // Expected: nothing to sync, policy not satisfied, another operation running
                // or not configured. Everything else is logged by code only.
                let quiet = [
                    ErrorCode::NotConnected,
                    ErrorCode::BlockedTooSoon,
                    ErrorCode::Busy,
                    ErrorCode::NotConfigured,
                    ErrorCode::RecoveryMode,
                ];
                if !quiet.iter().any(|c| c.as_str() == e.code) {
                    log::warn!("automatic bank sync failed: {}", e.code);
                }
            }
        }
    });
}
