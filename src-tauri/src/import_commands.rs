//! Tauri commands of the "Importar" screen (docs/decisions/phase-8.md).
//!
//! Privacy: the WebView never sends or receives a file path. Only Rust opens the
//! native Open dialog and reads the file; the bytes wait in `AppState::picked_import`
//! until the import is committed or cancelled. Errors carry fixed codes only, and
//! nothing here logs file names or contents.
//!
//! The logic lives in plain functions taking `&AppState` (tested without a Tauri
//! runtime); the `#[tauri::command]` wrappers add the dialog and the refresh event.
//! Every database command is refused in recovery mode; writes also take the busy
//! guard so they never overlap a bank sync, a backup or a restore.

use std::path::{Path, PathBuf};

use ledger_core::bank::overview::{self, AccountView};
use ledger_core::import::{
    self, FileFormat, ImportBatchRow, ImportPreview, ImportProfile, RawTable, StoredProfile,
    UndoReport, MAX_FILE_BYTES,
};
use ledger_core::ledger;
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

use crate::bank_commands::SYNCED_EVENT;
use crate::errors::{CommandError, ErrorCode};
use crate::state::{AppState, PickedImport};

/// Longest profile name accepted, in characters.
const MAX_PROFILE_NAME_CHARS: usize = 80;

// ---------------------------------------------------------------- DTOs

/// A stored import profile as the UI sees it.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    pub name: String,
    pub builtin: bool,
    pub profile: ImportProfile,
}

impl From<StoredProfile> for ProfileView {
    fn from(p: StoredProfile) -> Self {
        Self {
            name: p.name,
            builtin: p.builtin,
            profile: p.profile,
        }
    }
}

/// What the UI learns about the chosen file. The path never leaves Rust.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PickedImportView {
    /// File name only, no directory.
    pub file_name: String,
    pub size_bytes: usize,
    /// Name of the stored profile that reads this file, if one clearly does.
    pub detected_profile: Option<String>,
    /// Starting point for the generic mapping: sniffed CSV, or Excel for `.xlsx`/`.xls`.
    pub suggested_format: FileFormat,
    pub profiles: Vec<ProfileView>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportCommitResult {
    pub batch_id: i64,
    pub account_id: i64,
    pub inserted: usize,
    /// Rows already stored (identical, or twins that received a reference).
    pub duplicates: usize,
    /// Stored rows that were rewritten.
    pub updated: usize,
    /// Rows rejected by the parser or by the pipeline.
    pub invalid_rows: usize,
    pub transfers_matched: usize,
    pub categorized: usize,
    /// Whether `save_profile_as` was given and the profile was saved.
    pub profile_saved: bool,
}

// ------------------------------------------------------------- helpers

fn internal() -> CommandError {
    CommandError::new(ErrorCode::Internal)
}

fn lock(state: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, CommandError> {
    state.db.lock().map_err(|_| internal())
}

fn lock_picked(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, Option<PickedImport>>, CommandError> {
    state.picked_import.lock().map_err(|_| internal())
}

/// A trimmed, non-empty name of at most [`MAX_PROFILE_NAME_CHARS`] characters.
fn clean_name(name: &str) -> Result<String, CommandError> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > MAX_PROFILE_NAME_CHARS {
        return Err(CommandError::new(ErrorCode::InvalidInput));
    }
    Ok(name)
}

fn is_excel_name(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    lower.ends_with(".xlsx") || lower.ends_with(".xls")
}

// ------------------------------------------------------------ logic

/// Reads the chosen file (size cap first), stashes it and describes it for the UI.
pub fn pick_with(state: &AppState, path: &Path) -> Result<PickedImportView, CommandError> {
    state.require_ready()?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| CommandError::new(ErrorCode::ImportIo))?;
    let io = |_| CommandError::new(ErrorCode::ImportIo);
    let too_large = || CommandError::new(ErrorCode::ImportFileTooLarge);
    if std::fs::metadata(path).map_err(io)?.len() > MAX_FILE_BYTES as u64 {
        return Err(too_large());
    }
    let bytes = std::fs::read(path).map_err(io)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(too_large());
    }

    let conn = lock(state)?;
    let profiles = import::list_profiles(&conn)?;
    let detected_profile = import::detect_profile(&bytes, &file_name, &profiles);
    let suggested_format = if is_excel_name(&file_name) {
        FileFormat::Excel { sheet: None }
    } else {
        import::sniff_csv_format(&bytes)
    };
    let view = PickedImportView {
        file_name: file_name.clone(),
        size_bytes: bytes.len(),
        detected_profile,
        suggested_format,
        profiles: profiles.into_iter().map(ProfileView::from).collect(),
    };
    let sha256 = import::sha256_hex(&bytes);
    *lock_picked(state)? = Some(PickedImport {
        bytes,
        file_name,
        sha256,
    });
    Ok(view)
}

pub fn raw_rows_with(
    state: &AppState,
    format: &FileFormat,
    max_rows: usize,
) -> Result<RawTable, CommandError> {
    state.require_ready()?;
    let picked = lock_picked(state)?;
    let picked = picked
        .as_ref()
        .ok_or_else(|| CommandError::new(ErrorCode::ImportNoFile))?;
    Ok(import::raw_rows(
        &picked.bytes,
        &picked.file_name,
        format,
        max_rows,
    )?)
}

pub fn preview_with(
    state: &AppState,
    profile: &ImportProfile,
    account_id: Option<i64>,
) -> Result<ImportPreview, CommandError> {
    state.require_ready()?;
    let picked = lock_picked(state)?;
    let picked = picked
        .as_ref()
        .ok_or_else(|| CommandError::new(ErrorCode::ImportNoFile))?;
    let mut conn = lock(state)?;
    Ok(import::preview_import(
        &mut conn,
        account_id,
        &picked.bytes,
        &picked.file_name,
        profile,
    )?)
}

/// Imports the stashed file. The stash is cleared only on success, so after a
/// refusal (wrong account, bad mapping) the owner can fix it and retry.
pub fn commit_with(
    state: &AppState,
    profile: &ImportProfile,
    profile_name: &str,
    account_id: i64,
    save_profile_as: Option<&str>,
) -> Result<ImportCommitResult, CommandError> {
    let _busy = state.try_busy()?;
    let profile_name = clean_name(profile_name)?;
    let save_as = save_profile_as.map(clean_name).transpose()?;

    let mut stash = lock_picked(state)?;
    let result = {
        let picked = stash
            .as_ref()
            .ok_or_else(|| CommandError::new(ErrorCode::ImportNoFile))?;
        if import::sha256_hex(&picked.bytes) != picked.sha256 {
            return Err(CommandError::new(ErrorCode::ImportFileChanged));
        }
        let mut conn = lock(state)?;
        // Refuse before importing, so a built-in name never fails after the fact.
        if let Some(name) = &save_as {
            if import::list_profiles(&conn)?
                .iter()
                .any(|p| p.builtin && p.name == *name)
            {
                return Err(CommandError::new(ErrorCode::ImportBuiltinProfile));
            }
        }
        let outcome = import::import_file(
            &mut conn,
            account_id,
            &picked.bytes,
            &picked.file_name,
            &profile_name,
            profile,
        )?;
        let profile_saved = match &save_as {
            Some(name) => import::save_profile(&conn, name, profile).is_ok(),
            None => false,
        };
        let r = &outcome.report;
        ImportCommitResult {
            batch_id: outcome.batch_id,
            account_id,
            inserted: r.inserted,
            duplicates: r.duplicates + r.adopted,
            updated: r.updated + r.promoted,
            invalid_rows: outcome.parse_errors.len() + r.errors.len(),
            transfers_matched: r.transfers_matched,
            categorized: r.categorized,
            profile_saved,
        }
    };
    *stash = None;
    Ok(result)
}

pub fn cancel_with(state: &AppState) -> Result<(), CommandError> {
    *lock_picked(state)? = None;
    Ok(())
}

pub fn profiles_with(state: &AppState) -> Result<Vec<ProfileView>, CommandError> {
    state.require_ready()?;
    let conn = lock(state)?;
    let profiles = import::list_profiles(&conn)?;
    Ok(profiles.into_iter().map(ProfileView::from).collect())
}

pub fn save_profile_with(
    state: &AppState,
    name: &str,
    profile: &ImportProfile,
) -> Result<(), CommandError> {
    let _busy = state.try_busy()?;
    let name = clean_name(name)?;
    let conn = lock(state)?;
    Ok(import::save_profile(&conn, &name, profile)?)
}

pub fn history_with(state: &AppState, limit: usize) -> Result<Vec<ImportBatchRow>, CommandError> {
    state.require_ready()?;
    let conn = lock(state)?;
    Ok(import::list_import_batches(&conn, limit)?)
}

pub fn undo_with(state: &AppState, batch_id: i64) -> Result<UndoReport, CommandError> {
    let _busy = state.try_busy()?;
    let mut conn = lock(state)?;
    Ok(import::undo_import_batch(&mut conn, batch_id)?)
}

pub fn accounts_with(state: &AppState) -> Result<Vec<AccountView>, CommandError> {
    state.require_ready()?;
    let conn = lock(state)?;
    Ok(overview::accounts(&conn)?)
}

pub fn create_account_with(
    state: &AppState,
    name: &str,
    kind: Option<&str>,
) -> Result<AccountView, CommandError> {
    let _busy = state.try_busy()?;
    let conn = lock(state)?;
    let id = ledger::create_manual_account(&conn, name, kind).map_err(|e| match e {
        ledger::LedgerError::Conflict(_) => CommandError::new(ErrorCode::AccountNameTaken),
        other => CommandError::from(other),
    })?;
    overview::accounts(&conn)?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(internal)
}

// ------------------------------------------------------ dialog helper

fn open_dialog(app: &AppHandle) -> Option<PathBuf> {
    let mut dialog = app
        .dialog()
        .file()
        .add_filter("Extractos (CSV y Excel)", &["csv", "txt", "xlsx", "xls"])
        .add_filter("CSV", &["csv", "txt"])
        .add_filter("Excel", &["xlsx", "xls"]);
    if let Ok(dir) = app.path().download_dir() {
        if dir.is_dir() {
            dialog = dialog.set_directory(dir);
        }
    }
    dialog.blocking_pick_file().and_then(|p| p.into_path().ok())
}

// ------------------------------------------------------------ commands

/// Opens the native Open dialog. `None` when cancelled (the previous pick, if any,
/// stays stashed).
#[tauri::command]
pub async fn import_pick_file(app: AppHandle) -> Result<Option<PickedImportView>, CommandError> {
    app.state::<AppState>().require_ready()?;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = open_dialog(&app) else {
            return Ok(None);
        };
        pick_with(&app.state::<AppState>(), &path).map(Some)
    })
    .await
    .map_err(|_| internal())?
}

/// The first `max_rows` (at most 200) rows of the chosen file read with `format`.
#[tauri::command]
pub fn import_raw_rows(
    state: State<'_, AppState>,
    format: FileFormat,
    max_rows: usize,
) -> Result<RawTable, CommandError> {
    raw_rows_with(&state, &format, max_rows)
}

/// Dry run: writes nothing. `account_id` is optional; with it the counts and the
/// identity check are included.
#[tauri::command]
pub fn import_preview(
    state: State<'_, AppState>,
    profile: ImportProfile,
    account_id: Option<i64>,
) -> Result<ImportPreview, CommandError> {
    preview_with(&state, &profile, account_id)
}

#[tauri::command]
pub fn import_commit(
    app: AppHandle,
    state: State<'_, AppState>,
    profile: ImportProfile,
    profile_name: String,
    account_id: i64,
    save_profile_as: Option<String>,
) -> Result<ImportCommitResult, CommandError> {
    let result = commit_with(
        &state,
        &profile,
        &profile_name,
        account_id,
        save_profile_as.as_deref(),
    )?;
    // The ledger screens reload on this event.
    let _ = app.emit(SYNCED_EVENT, ());
    Ok(result)
}

#[tauri::command]
pub fn import_cancel(state: State<'_, AppState>) -> Result<(), CommandError> {
    cancel_with(&state)
}

#[tauri::command]
pub fn import_profiles(state: State<'_, AppState>) -> Result<Vec<ProfileView>, CommandError> {
    profiles_with(&state)
}

#[tauri::command]
pub fn import_save_profile(
    state: State<'_, AppState>,
    name: String,
    profile: ImportProfile,
) -> Result<(), CommandError> {
    save_profile_with(&state, &name, &profile)
}

#[tauri::command]
pub fn import_history(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<ImportBatchRow>, CommandError> {
    history_with(&state, limit)
}

#[tauri::command]
pub fn import_undo(
    app: AppHandle,
    state: State<'_, AppState>,
    batch_id: i64,
) -> Result<UndoReport, CommandError> {
    let report = undo_with(&state, batch_id)?;
    let _ = app.emit(SYNCED_EVENT, ());
    Ok(report)
}

/// Accounts an import can target (the same list as the bank overview, without the
/// connection state).
#[tauri::command]
pub fn import_accounts(state: State<'_, AppState>) -> Result<Vec<AccountView>, CommandError> {
    accounts_with(&state)
}

/// Creates an account without a bank connection. `kind`: `checking` (default),
/// `savings`, `card`, `cash` or `other`.
#[tauri::command]
pub fn ledger_create_account(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    kind: Option<String>,
) -> Result<AccountView, CommandError> {
    let account = create_account_with(&state, &name, kind.as_deref())?;
    let _ = app.emit(SYNCED_EVENT, ());
    Ok(account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::db::{open_in_memory, DbKey, KEY_LEN};
    use ledger_core::import::{generic_csv_es, seed_builtin_profiles};
    use ledger_core::keystore::MemoryStore;

    use crate::state::AppMode;
    use ledger_core::backup::RecoveryReason;

    const FIXTURE_ES: &[u8] =
        include_bytes!("../crates/ledger-core/tests/fixtures/generic_es_1252.csv");
    const OPENBANK: &[u8] =
        include_bytes!("../crates/ledger-core/tests/fixtures/openbank_demo.html");

    fn state() -> AppState {
        let conn = open_in_memory(&DbKey::from_bytes([0x11; KEY_LEN])).unwrap();
        seed_builtin_profiles(&conn).unwrap();
        conn.execute("INSERT INTO accounts (name) VALUES ('Cuenta ficticia')", [])
            .unwrap();
        AppState::new(conn, Box::new(MemoryStore::new()), PathBuf::new())
    }

    fn write(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn code<T: std::fmt::Debug>(r: Result<T, CommandError>) -> String {
        r.unwrap_err().code
    }

    fn stashed(state: &AppState) -> bool {
        state.picked_import.lock().unwrap().is_some()
    }

    fn tx_count(state: &AppState) -> i64 {
        state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM transactions", [], |r| r.get(0))
            .unwrap()
    }

    fn account_id(state: &AppState) -> i64 {
        state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM accounts LIMIT 1", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn pick_reads_stashes_detects_and_returns_no_path() {
        let dir = tempfile::tempdir().unwrap();
        let s = state();
        let path = write(&dir, "movimientos.xls", OPENBANK);
        let view = pick_with(&s, &path).unwrap();
        assert_eq!(view.file_name, "movimientos.xls");
        assert_eq!(view.size_bytes, OPENBANK.len());
        assert_eq!(view.detected_profile.as_deref(), Some("Openbank Excel"));
        assert_eq!(view.suggested_format, FileFormat::Excel { sheet: None });
        assert_eq!(view.profiles.len(), 2);
        assert!(stashed(&s));
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains(dir.path().to_str().unwrap()));
        assert!(json.contains("\"fileName\"") && json.contains("\"suggestedFormat\""));

        // A CSV gets a sniffed format.
        let csv = write(&dir, "extracto.csv", FIXTURE_ES);
        let view = pick_with(&s, &csv).unwrap();
        assert_eq!(
            view.detected_profile.as_deref(),
            Some("Generic CSV (es-ES)")
        );
        assert!(matches!(
            view.suggested_format,
            FileFormat::Csv { delimiter: ';', .. }
        ));
    }

    #[test]
    fn pick_refuses_big_and_missing_files_and_recovery_mode() {
        let dir = tempfile::tempdir().unwrap();
        let s = state();
        let big = write(&dir, "grande.csv", &vec![b'a'; MAX_FILE_BYTES + 1]);
        assert_eq!(code(pick_with(&s, &big)), "import_file_too_large");
        assert_eq!(
            code(pick_with(&s, &dir.path().join("no-existe.csv"))),
            "import_io"
        );
        assert!(!stashed(&s));
        *s.mode.write().unwrap() = AppMode::Recovery(RecoveryReason::KeyMissing);
        let ok = write(&dir, "a.csv", FIXTURE_ES);
        assert_eq!(code(pick_with(&s, &ok)), "recovery_mode");
        assert_eq!(code(history_with(&s, 10)), "recovery_mode");
        assert_eq!(code(profiles_with(&s)), "recovery_mode");
        assert_eq!(code(accounts_with(&s)), "recovery_mode");
        assert_eq!(
            code(commit_with(&s, &generic_csv_es(), "p", 1, None)),
            "recovery_mode"
        );
        assert_eq!(code(undo_with(&s, 1)), "recovery_mode");
    }

    #[test]
    fn raw_rows_and_preview_need_a_picked_file() {
        let s = state();
        let format = FileFormat::Excel { sheet: None };
        assert_eq!(code(raw_rows_with(&s, &format, 5)), "import_no_file");
        assert_eq!(
            code(preview_with(&s, &generic_csv_es(), None)),
            "import_no_file"
        );
        assert_eq!(
            code(commit_with(&s, &generic_csv_es(), "p", 1, None)),
            "import_no_file"
        );
    }

    #[test]
    fn preview_writes_nothing_and_commit_clears_the_stash() {
        let dir = tempfile::tempdir().unwrap();
        let s = state();
        pick_with(&s, &write(&dir, "extracto.csv", FIXTURE_ES)).unwrap();
        let account = account_id(&s);

        let rows = raw_rows_with(
            &s,
            &FileFormat::Csv {
                delimiter: ';',
                encoding: ledger_core::import::CsvEncoding::Windows1252,
                quote: '"',
            },
            3,
        )
        .unwrap();
        assert_eq!(rows.rows.len(), 3);
        let preview = preview_with(&s, &generic_csv_es(), Some(account)).unwrap();
        assert_eq!(preview.counts.unwrap().would_insert, 3);
        assert_eq!(tx_count(&s), 0);
        assert!(stashed(&s));

        let done =
            commit_with(&s, &generic_csv_es(), "Generic CSV (es-ES)", account, None).unwrap();
        assert_eq!(
            (done.inserted, done.duplicates, done.invalid_rows),
            (3, 0, 3)
        );
        assert!(!done.profile_saved);
        assert_eq!(tx_count(&s), 3);
        assert!(!stashed(&s));
        // The stash is gone: a second commit has nothing to import.
        assert_eq!(
            code(commit_with(&s, &generic_csv_es(), "p", account, None)),
            "import_no_file"
        );

        let history = history_with(&s, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].can_undo);
        let undo = undo_with(&s, history[0].id).unwrap();
        assert_eq!(undo.deleted, 3);
        assert_eq!(tx_count(&s), 0);
        assert_eq!(
            code(undo_with(&s, history[0].id)),
            "import_batch_already_undone"
        );
        assert_eq!(code(undo_with(&s, 999)), "import_batch_not_found");
    }

    #[test]
    fn failed_commit_keeps_the_stash_for_a_retry() {
        let dir = tempfile::tempdir().unwrap();
        let s = state();
        pick_with(&s, &write(&dir, "extracto.csv", FIXTURE_ES)).unwrap();
        assert_eq!(
            code(commit_with(&s, &generic_csv_es(), "p", 9_999, None)),
            "import_account_not_found"
        );
        assert!(stashed(&s));
        assert_eq!(
            code(commit_with(&s, &generic_csv_es(), "   ", 1, None)),
            "invalid_input"
        );
        assert_eq!(tx_count(&s), 0);
        // A held busy guard (sync, backup...) refuses the write and keeps the stash.
        let held = s.try_busy().unwrap();
        assert_eq!(
            code(commit_with(&s, &generic_csv_es(), "p", 1, None)),
            "busy"
        );
        assert_eq!(code(undo_with(&s, 1)), "busy");
        drop(held);
        assert!(stashed(&s));
    }

    #[test]
    fn commit_can_save_the_profile_but_never_over_a_built_in() {
        let dir = tempfile::tempdir().unwrap();
        let s = state();
        pick_with(&s, &write(&dir, "extracto.csv", FIXTURE_ES)).unwrap();
        let account = account_id(&s);
        assert_eq!(
            code(commit_with(
                &s,
                &generic_csv_es(),
                "p",
                account,
                Some("Openbank Excel")
            )),
            "import_builtin_profile"
        );
        assert_eq!(tx_count(&s), 0);
        assert!(stashed(&s));

        let done = commit_with(
            &s,
            &generic_csv_es(),
            "Mi banco",
            account,
            Some("  Mi banco "),
        )
        .unwrap();
        assert!(done.profile_saved);
        let names: Vec<String> = profiles_with(&s)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert!(names.contains(&"Mi banco".to_string()));

        assert_eq!(
            code(save_profile_with(
                &s,
                "Generic CSV (es-ES)",
                &generic_csv_es()
            )),
            "import_builtin_profile"
        );
        save_profile_with(&s, "Otro", &generic_csv_es()).unwrap();
    }

    #[test]
    fn cancel_clears_the_stash() {
        let dir = tempfile::tempdir().unwrap();
        let s = state();
        pick_with(&s, &write(&dir, "extracto.csv", FIXTURE_ES)).unwrap();
        assert!(stashed(&s));
        cancel_with(&s).unwrap();
        assert!(!stashed(&s));
        cancel_with(&s).unwrap(); // idempotent
    }

    #[test]
    fn a_swapped_stash_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = state();
        pick_with(&s, &write(&dir, "extracto.csv", FIXTURE_ES)).unwrap();
        s.picked_import
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .bytes
            .push(b'x');
        assert_eq!(
            code(commit_with(&s, &generic_csv_es(), "p", 1, None)),
            "import_file_changed"
        );
        assert_eq!(tx_count(&s), 0);
    }

    #[test]
    fn manual_accounts_are_created_and_listed() {
        let s = state();
        let created = create_account_with(&s, " Efectivo ", Some("cash")).unwrap();
        assert_eq!(
            (created.name.as_str(), created.kind.as_str()),
            ("Efectivo", "cash")
        );
        assert!(created.display_suffix.is_none() && created.balance_cents.is_none());
        assert_eq!(accounts_with(&s).unwrap().len(), 2);
        assert_eq!(
            code(create_account_with(&s, "efectivo", None)),
            "account_name_taken"
        );
        assert_eq!(code(create_account_with(&s, "", None)), "invalid_input");
        assert_eq!(
            code(create_account_with(&s, "X", Some("loan"))),
            "invalid_input"
        );
    }

    #[test]
    fn import_errors_map_to_fixed_codes_without_details() {
        use ledger_core::import::ImportError;
        let e: CommandError = ImportError::MissingColumn("SECRETO".into()).into();
        assert_eq!(e.code, "import_missing_column");
        assert!(!e.message.contains("SECRETO"));
        let e: CommandError = ImportError::AccountBelongsToOther {
            existing_account_id: 7,
        }
        .into();
        assert_eq!(e.code, "import_account_belongs_to_other");
        assert!(!e.message.contains('7'));
        assert_eq!(
            CommandError::from(ImportError::BatchHasSyncedRows).code,
            "import_batch_synced"
        );
    }
}
