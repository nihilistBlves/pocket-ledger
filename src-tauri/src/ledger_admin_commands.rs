//! Tauri commands of the "Categorías", "Reglas" and "Presupuestos" screens
//! (docs/decisions/phase-7.md). Thin wrappers like `ledger_commands`: the `Mutex` is
//! locked synchronously and failures are mapped to a fixed [`CommandError`].

use ledger_core::ledger::{
    self, ApplyResult, BudgetMonth, CategoryNode, DeleteCategoryResult, RuleInput, RuleRow,
    RuleTest, SaveRuleResult,
};
use rusqlite::Connection;
use tauri::State;

use crate::errors::{CommandError, ErrorCode};
use crate::state::AppState;

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, CommandError> {
    state
        .db
        .lock()
        .map_err(|_| CommandError::new(ErrorCode::Internal))
}

#[tauri::command]
pub fn ledger_category_tree(
    state: State<'_, AppState>,
    month: String,
) -> Result<Vec<CategoryNode>, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::category_tree(&conn, &month)?)
}

#[tauri::command]
pub fn ledger_create_category(
    state: State<'_, AppState>,
    parent_id: Option<i64>,
    name: String,
    kind: Option<String>,
) -> Result<CategoryNode, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::create_category(
        &conn,
        parent_id,
        &name,
        kind.as_deref(),
    )?)
}

#[tauri::command]
pub fn ledger_update_category(
    state: State<'_, AppState>,
    id: i64,
    name: String,
    parent_id: Option<i64>,
    color: Option<String>,
) -> Result<CategoryNode, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::update_category(
        &conn,
        id,
        &name,
        parent_id,
        color.as_deref(),
    )?)
}

#[tauri::command]
pub fn ledger_delete_category(
    state: State<'_, AppState>,
    id: i64,
    move_to: i64,
    delete_rules: bool,
) -> Result<DeleteCategoryResult, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::delete_category(&conn, id, move_to, delete_rules)?)
}

#[tauri::command]
pub fn ledger_rules(state: State<'_, AppState>) -> Result<Vec<RuleRow>, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::list_rules(&conn)?)
}

#[tauri::command]
pub fn ledger_save_rule(
    state: State<'_, AppState>,
    id: Option<i64>,
    rule: RuleInput,
    apply_to_existing: bool,
    today: String,
) -> Result<SaveRuleResult, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::save_rule(
        &conn,
        id,
        &rule,
        apply_to_existing,
        &today,
    )?)
}

#[tauri::command]
pub fn ledger_delete_rule(state: State<'_, AppState>, id: i64) -> Result<(), CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::delete_rule(&conn, id)?)
}

#[tauri::command]
pub fn ledger_test_rule(
    state: State<'_, AppState>,
    id: Option<i64>,
    rule: RuleInput,
    today: String,
) -> Result<RuleTest, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::test_rule(&conn, id, &rule, &today)?)
}

#[tauri::command]
pub fn ledger_apply_rules_uncategorized(
    state: State<'_, AppState>,
) -> Result<ApplyResult, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::apply_rules_uncategorized(&conn)?)
}

#[tauri::command]
pub fn ledger_budgets(
    state: State<'_, AppState>,
    month: String,
    today: String,
) -> Result<BudgetMonth, CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::budgets(&conn, &month, &today)?)
}

#[tauri::command]
pub fn ledger_set_budget(
    state: State<'_, AppState>,
    category_id: i64,
    month: String,
    amount_cents: i64,
) -> Result<(), CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::set_budget(
        &conn,
        category_id,
        &month,
        amount_cents,
    )?)
}

#[tauri::command]
pub fn ledger_remove_budget(
    state: State<'_, AppState>,
    category_id: i64,
    month: String,
) -> Result<(), CommandError> {
    let conn = lock(&state)?;
    Ok(ledger::remove_budget(&conn, category_id, &month)?)
}
