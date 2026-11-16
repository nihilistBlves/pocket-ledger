//! Tauri commands of the "Resumen" and "Movimientos" screens (docs/decisions/phase-6.md).
//! Thin wrappers: the std `Mutex<Connection>` is locked in a synchronous block, never
//! across an `.await`, and every failure is mapped to a fixed [`CommandError`].

use ledger_core::ledger::{self, Dashboard, SetCategoryResult, TxFilter, TxPage};
use ledger_core::ledger::{CategoryOption, TxRow};
use rusqlite::Connection;
use tauri::State;

use crate::errors::{CommandError, ErrorCode};
use crate::state::AppState;

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, CommandError> {
    state.db.lock().map_err(|_| CommandError::new(ErrorCode::Internal))
}

#[tauri::command]
pub fn ledger_dashboard(
    state: State<'_, AppState>,
    month: String,
    today: String,
) -> Result<Dashboard, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::dashboard(&conn, &month, &today)?)
}

#[tauri::command]
pub fn ledger_transactions(state: State<'_, AppState>, filter: TxFilter) -> Result<TxPage, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::list_transactions(&conn, &filter)?)
}

#[tauri::command]
pub fn ledger_categories(state: State<'_, AppState>) -> Result<Vec<CategoryOption>, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::categories(&conn)?)
}

#[tauri::command]
pub fn ledger_tags(state: State<'_, AppState>) -> Result<Vec<String>, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::tag_names(&conn)?)
}

// Arguments are the contract's top-level fields (Tauri maps camelCase keys to these
// snake_case parameters): `invoke('ledger_set_category', { txId, categoryId, createRule })`.

#[tauri::command]
pub fn ledger_set_category(
    state: State<'_, AppState>,
    tx_id: i64,
    category_id: i64,
    create_rule: bool,
) -> Result<SetCategoryResult, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::set_category(&conn, tx_id, category_id, create_rule)?)
}

#[tauri::command]
pub fn ledger_set_notes(
    state: State<'_, AppState>,
    tx_id: i64,
    notes: Option<String>,
) -> Result<TxRow, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::set_notes(&conn, tx_id, notes.as_deref())?)
}

#[tauri::command]
pub fn ledger_set_tags(
    state: State<'_, AppState>,
    tx_id: i64,
    tags: Vec<String>,
) -> Result<TxRow, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::set_tags(&conn, tx_id, &tags)?)
}

#[tauri::command]
pub fn ledger_unlink_transfer(state: State<'_, AppState>, tx_id: i64) -> Result<Vec<TxRow>, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::unlink_transfer(&conn, tx_id)?)
}
