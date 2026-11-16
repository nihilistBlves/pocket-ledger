//! Tauri commands for encrypted backups, restore and recovery mode (ADR 0005).
//!
//! The logic lives in plain functions taking `&AppState` and paths so it is tested
//! without a Tauri runtime; the `#[tauri::command]` wrappers only add the native
//! dialogs (driven from Rust, no dialog permission in the WebView), move heavy work
//! to a blocking thread and emit events. Passphrases are moved into `Zeroizing`
//! buffers at once and never logged, stored or returned. The std `Mutex` is never
//! held across an `.await`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, Utc};
use ledger_core::backup::{
    self, BackupInfo, KdfParams, RecoveryReason,
};
use ledger_core::db::{self, DbKey};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use zeroize::Zeroizing;

use crate::bank_config::CONFIG_FILE;
use crate::errors::{CommandError, ErrorCode};
use crate::state::{AppMode, AppState, DB_FILE};

/// Emitted after the database was replaced, so every screen reloads.
pub const REPLACED_EVENT: &str = "data://replaced";
/// Last verified backup. Holds only the time and the file name.
pub const STATE_FILE: &str = "backup-state.json";
/// Days after which the UI reminds the owner to make a new backup.
pub const REMINDER_DAYS: i64 = 7;
const BACKUP_EXTENSION: &str = "plbackup";

// ---------------------------------------------------------------- DTOs

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AppModeView {
    pub mode: &'static str,
    pub reason: Option<&'static str>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct BackupStatus {
    pub last_backup_at: Option<String>,
    pub last_file_name: Option<String>,
    pub reminder: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PassphraseCheck {
    pub ok: bool,
    pub score: u8,
    pub feedback: Option<&'static str>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PickedBackup {
    pub file_name: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RestoreResult {
    pub backup_created_at: String,
    pub pre_restore_file_name: Option<String>,
}

/// Content of `backup-state.json`: nothing but these two fields.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    last_backup_at: Option<String>,
    last_file_name: Option<String>,
}

// ------------------------------------------------------- plain functions

pub fn app_mode_view(state: &AppState) -> AppModeView {
    match state.mode() {
        AppMode::Ready => AppModeView { mode: "ready", reason: None },
        AppMode::Recovery(reason) => AppModeView {
            mode: "recovery",
            reason: Some(match reason {
                RecoveryReason::KeyMissing => "key_missing",
                RecoveryReason::KeyMalformed => "key_malformed",
                RecoveryReason::KeyWrong => "key_wrong",
            }),
        },
    }
}

fn read_state_file(data_dir: &Path) -> StateFile {
    std::fs::read_to_string(data_dir.join(STATE_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Atomic write (temp file, flush, rename). Only called after a verified backup.
fn write_state_file(data_dir: &Path, info: &BackupInfo) -> Result<(), CommandError> {
    use std::io::Write;
    let content = StateFile {
        last_backup_at: Some(info.created_at.clone()),
        last_file_name: Some(info.file_name.clone()),
    };
    let text = serde_json::to_string_pretty(&content).map_err(|_| internal())?;
    let tmp = data_dir.join(format!("{STATE_FILE}.partial"));
    let target = data_dir.join(STATE_FILE);
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &target)
    };
    write().map_err(|_| {
        let _ = std::fs::remove_file(&tmp);
        CommandError::new(ErrorCode::BackupIo)
    })
}

pub fn backup_status_at(data_dir: &Path, now: DateTime<Utc>) -> BackupStatus {
    let file = read_state_file(data_dir);
    let last = file
        .last_backup_at
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc));
    let reminder = last.is_none_or(|t| now - t > Duration::days(REMINDER_DAYS));
    BackupStatus {
        last_backup_at: last.map(|_| file.last_backup_at.clone().unwrap_or_default()),
        last_file_name: if last.is_some() { file.last_file_name } else { None },
        reminder,
    }
}

pub fn check_passphrase_view(passphrase: &str) -> PassphraseCheck {
    let v = backup::check_passphrase(passphrase);
    PassphraseCheck { ok: v.ok, score: v.score, feedback: v.feedback }
}

fn internal() -> CommandError {
    CommandError::new(ErrorCode::Internal)
}

/// Checks done before the Save dialog opens, so the owner does not pick a file
/// for a request that would be refused anyway.
pub fn precheck_create(
    state: &AppState,
    passphrase: &str,
    confirmation: &str,
) -> Result<(), CommandError> {
    state.require_ready()?;
    if passphrase != confirmation {
        return Err(CommandError::new(ErrorCode::PassphraseMismatch));
    }
    if !backup::check_passphrase(passphrase).ok {
        return Err(CommandError::new(ErrorCode::WeakPassphrase));
    }
    Ok(())
}

fn current_key(state: &AppState) -> Result<DbKey, CommandError> {
    state.db_key.lock().map_err(|_| internal())?.clone().ok_or_else(internal)
}

fn picked_path(state: &AppState) -> Result<PathBuf, CommandError> {
    state
        .picked_backup
        .lock()
        .map_err(|_| internal())?
        .clone()
        .ok_or_else(|| CommandError::new(ErrorCode::NoBackupSelected))
}

fn clear_picked(state: &AppState) {
    if let Ok(mut p) = state.picked_backup.lock() {
        *p = None;
    }
}

/// Creates a verified backup at `target` and records it in `backup-state.json`.
pub fn create_with(
    state: &AppState,
    passphrase: &str,
    confirmation: &str,
    target: &Path,
    params: KdfParams,
    app_version: &str,
) -> Result<BackupInfo, CommandError> {
    precheck_create(state, passphrase, confirmation)?;
    let _busy = state.try_busy()?;
    let key = current_key(state)?;
    let config = std::fs::read(state.data_dir.join(CONFIG_FILE)).ok().map(Zeroizing::new);
    let info = {
        let conn = state.db.lock().map_err(|_| internal())?;
        backup::create_backup(
            &conn,
            &key,
            &state.data_dir,
            config.as_ref().map(|c| c.as_slice()),
            passphrase,
            target,
            params,
            app_version,
        )?
    };
    write_state_file(&state.data_dir, &info)?;
    Ok(info)
}

fn reopen_original(state: &AppState, db_path: &Path, key: &DbKey) -> bool {
    match (db::open(db_path, key), state.db.lock()) {
        (Ok(conn), Ok(mut guard)) => {
            *guard = conn;
            true
        }
        _ => false,
    }
}

fn file_name_of(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Restores the picked backup over the running database (ADR 0005, "Restoring").
pub fn restore_with(state: &AppState, passphrase: &str) -> Result<RestoreResult, CommandError> {
    state.require_ready()?;
    let path = picked_path(state)?;
    let _busy = state.try_busy()?;
    let key = current_key(state)?;
    let db_path = state.data_dir.join(DB_FILE);

    // Slow work first, with no lock held: decrypt, build and check the replacement.
    let decrypted = backup::read_backup(&path, passphrase)?;
    let prepared = backup::prepare_restore(&decrypted, &state.data_dir, &key, DB_FILE)?;

    // Now the swap, with the connection mutex held for the shortest possible time.
    let mut guard = state.db.lock().map_err(|_| internal())?;
    let _ = guard.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    let placeholder = Connection::open_in_memory().map_err(|_| internal())?;
    drop(std::mem::replace(&mut *guard, placeholder));

    let outcome = backup::swap_in(prepared, &db_path, &key);
    let (new_conn, pre_restore) = match outcome {
        Ok(ok) => ok,
        Err(e) => {
            drop(guard);
            // Every failure path that did not lose the original leaves it in place.
            let lost = matches!(e, backup::BackupError::RestoreFailedAndRollbackFailed);
            if !lost && !reopen_original(state, &db_path, &key) {
                return Err(CommandError::new(ErrorCode::RestoreFailed));
            }
            return Err(e.into());
        }
    };
    *guard = new_conn;
    drop(guard);

    if let Some(config) = &decrypted.config {
        // The database is already replaced; a config problem must not undo that.
        if backup::install_config(&state.data_dir, CONFIG_FILE, config).is_err() {
            log::warn!("restored backup, but its bank config could not be installed");
        }
    }
    clear_picked(state);
    Ok(RestoreResult {
        backup_created_at: decrypted.manifest.created_at.clone(),
        pre_restore_file_name: pre_restore.as_deref().and_then(file_name_of),
    })
}

/// Recovery mode: the key inside the backup opens the existing database.
pub fn recovery_unlock_with(state: &AppState, passphrase: &str) -> Result<(), CommandError> {
    require_recovery(state)?;
    let path = picked_path(state)?;
    let _busy = state.try_busy_unchecked()?;
    let db_path = state.data_dir.join(DB_FILE);
    let decrypted = backup::read_backup(&path, passphrase)?;
    let key = backup::unlock_with_backup(&db_path, &decrypted, state.store.as_ref())?;
    let conn = db::open(&db_path, &key).map_err(|_| internal())?;
    leave_recovery(state, conn, key)?;
    clear_picked(state);
    Ok(())
}

/// Recovery mode: replace the unreadable database with the backup's data.
pub fn recovery_restore_with(state: &AppState, passphrase: &str) -> Result<(), CommandError> {
    require_recovery(state)?;
    let path = picked_path(state)?;
    let _busy = state.try_busy_unchecked()?;
    let db_path = state.data_dir.join(DB_FILE);
    let decrypted = backup::read_backup(&path, passphrase)?;
    let (conn, key) = backup::recovery_restore(&db_path, &decrypted, state.store.as_ref())?;
    leave_recovery(state, conn, key)?;
    if let Some(config) = &decrypted.config {
        if backup::install_config(&state.data_dir, CONFIG_FILE, config).is_err() {
            log::warn!("restored backup, but its bank config could not be installed");
        }
    }
    clear_picked(state);
    Ok(())
}

fn require_recovery(state: &AppState) -> Result<(), CommandError> {
    match state.mode() {
        AppMode::Recovery(_) => Ok(()),
        AppMode::Ready => Err(internal()),
    }
}

fn leave_recovery(state: &AppState, conn: Connection, key: DbKey) -> Result<(), CommandError> {
    *state.db.lock().map_err(|_| internal())? = conn;
    *state.db_key.lock().map_err(|_| internal())? = Some(key);
    *state.mode.write().map_err(|_| internal())? = AppMode::Ready;
    Ok(())
}

/// Stores the dialog's choice and returns just the file name.
pub fn remember_picked(state: &AppState, path: PathBuf) -> Result<PickedBackup, CommandError> {
    let file_name = file_name_of(&path).unwrap_or_default();
    *state.picked_backup.lock().map_err(|_| internal())? = Some(path);
    Ok(PickedBackup { file_name })
}

fn default_file_name() -> String {
    Local::now().format("pocket-ledger-%Y-%m-%d-%H%M.plbackup").to_string()
}

fn with_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension(BACKUP_EXTENSION);
    }
    path
}

// ------------------------------------------------------- dialog helpers

fn save_dialog(app: &AppHandle) -> Option<PathBuf> {
    let mut dialog = app
        .dialog()
        .file()
        .add_filter("Copia de Pocket Ledger", &[BACKUP_EXTENSION])
        .set_file_name(default_file_name());
    if let Ok(docs) = app.path().document_dir() {
        let dir = docs.join("Pocket Ledger").join("Copias");
        if std::fs::create_dir_all(&dir).is_ok() {
            dialog = dialog.set_directory(dir);
        }
    }
    dialog.blocking_save_file().and_then(|p| p.into_path().ok()).map(with_extension)
}

fn open_dialog(app: &AppHandle) -> Option<PathBuf> {
    let mut dialog =
        app.dialog().file().add_filter("Copia de Pocket Ledger", &[BACKUP_EXTENSION]);
    if let Ok(docs) = app.path().document_dir() {
        let dir = docs.join("Pocket Ledger").join("Copias");
        if dir.is_dir() {
            dialog = dialog.set_directory(dir);
        }
    }
    dialog.blocking_pick_file().and_then(|p| p.into_path().ok())
}

async fn blocking<T, F>(app: &AppHandle, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&AppHandle) -> Result<T, CommandError> + Send + 'static,
{
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || f(&app)).await.map_err(|_| internal())?
}

// ------------------------------------------------------------ commands

#[tauri::command]
pub fn app_mode(state: State<'_, AppState>) -> AppModeView {
    app_mode_view(&state)
}

#[tauri::command]
pub fn backup_status(state: State<'_, AppState>) -> BackupStatus {
    backup_status_at(&state.data_dir, Utc::now())
}

#[tauri::command]
pub async fn backup_check_passphrase(passphrase: String) -> PassphraseCheck {
    let passphrase = Zeroizing::new(passphrase);
    check_passphrase_view(&passphrase)
}

#[tauri::command]
pub async fn backup_create(
    app: AppHandle,
    passphrase: String,
    confirmation: String,
) -> Result<Option<BackupInfo>, CommandError> {
    let passphrase = Zeroizing::new(passphrase);
    let confirmation = Zeroizing::new(confirmation);
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        precheck_create(&state, &passphrase, &confirmation)?;
        let Some(target) = save_dialog(app) else {
            return Ok(None);
        };
        create_with(
            &state,
            &passphrase,
            &confirmation,
            &target,
            KdfParams::DEFAULT,
            env!("CARGO_PKG_VERSION"),
        )
        .map(Some)
    })
    .await
}

#[tauri::command]
pub async fn backup_pick_file(app: AppHandle) -> Result<Option<PickedBackup>, CommandError> {
    blocking(&app, |app| {
        let state = app.state::<AppState>();
        match open_dialog(app) {
            Some(path) => remember_picked(&state, path).map(Some),
            None => Ok(None),
        }
    })
    .await
}

#[tauri::command]
pub async fn backup_restore(
    app: AppHandle,
    passphrase: String,
) -> Result<RestoreResult, CommandError> {
    let passphrase = Zeroizing::new(passphrase);
    let result = blocking(&app, move |app| {
        let state = app.state::<AppState>();
        restore_with(&state, &passphrase)
    })
    .await;
    if result.is_ok() {
        let _ = app.emit(REPLACED_EVENT, ());
    }
    result
}

#[tauri::command]
pub async fn recovery_unlock(app: AppHandle, passphrase: String) -> Result<(), CommandError> {
    let passphrase = Zeroizing::new(passphrase);
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        recovery_unlock_with(&state, &passphrase)
    })
    .await
}

#[tauri::command]
pub async fn recovery_restore(app: AppHandle, passphrase: String) -> Result<(), CommandError> {
    let passphrase = Zeroizing::new(passphrase);
    blocking(&app, move |app| {
        let state = app.state::<AppState>();
        recovery_restore_with(&state, &passphrase)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::db::KEY_LEN;
    use ledger_core::keystore::{MemoryStore, SecretStore, DB_KEY_ENTRY};
    use tempfile::TempDir;

    const PASS: &str = "violin-marmalade-orbit-glacier-seven";
    const VERSION: &str = "0.0.0-test";
    const MIN: KdfParams = KdfParams::MIN;

    fn key(byte: u8) -> DbKey {
        DbKey::from_bytes([byte; KEY_LEN])
    }

    /// A ready state on a real file-backed database with one fictitious account.
    fn ready(dir: &TempDir, byte: u8, account: &str) -> AppState {
        let k = key(byte);
        let conn = db::open(&dir.path().join(DB_FILE), &k).unwrap();
        conn.execute(
            "INSERT INTO accounts (name, kind, currency, identity_hash, display_suffix)
             VALUES (?1, 'checking', 'EUR', ?1, '0000')",
            [account],
        )
        .unwrap();
        let store = MemoryStore::new();
        store.set(DB_KEY_ENTRY, k.as_bytes()).unwrap();
        AppState::new_ready(conn, Box::new(store), dir.path().to_path_buf(), Some(k))
    }

    fn accounts(state: &AppState) -> Vec<String> {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare("SELECT name FROM accounts ORDER BY name").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(Result::unwrap).collect()
    }

    fn make_backup(state: &AppState, name: &str) -> PathBuf {
        let target = state.data_dir.join(name);
        create_with(state, PASS, PASS, &target, MIN, VERSION).unwrap();
        target
    }

    fn code<T: std::fmt::Debug>(r: Result<T, CommandError>) -> String {
        r.unwrap_err().code
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn create_records_a_verified_backup_and_clears_the_reminder() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x31, "Cuenta ficticia A");
        let before = backup_status_at(dir.path(), Utc::now());
        assert!(before.reminder);
        assert_eq!(before.last_backup_at, None);

        let target = dir.path().join("copia.plbackup");
        let info = create_with(&state, PASS, PASS, &target, MIN, VERSION).unwrap();
        assert!(target.is_file());
        assert_eq!(info.file_name, "copia.plbackup");

        let after = backup_status_at(dir.path(), Utc::now());
        assert!(!after.reminder);
        assert_eq!(after.last_backup_at.as_deref(), Some(info.created_at.as_str()));
        assert_eq!(after.last_file_name.as_deref(), Some("copia.plbackup"));

        // The state file holds exactly two fields and nothing secret.
        let text = std::fs::read_to_string(dir.path().join(STATE_FILE)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v.as_object().unwrap().len(), 2);
        assert!(!text.contains(PASS));
        assert!(!text.contains(key(0x31).to_hex().as_str()));
        assert!(!text.contains(dir.path().to_string_lossy().as_ref()));
        // The temp file of the atomic write is gone.
        assert!(!file_names(dir.path()).iter().any(|n| n.ends_with(".partial")));
    }

    #[test]
    fn reminder_after_seven_days() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x32, "Cuenta ficticia A");
        make_backup(&state, "copia.plbackup");
        let now = Utc::now();
        assert!(!backup_status_at(dir.path(), now + Duration::days(6)).reminder);
        assert!(backup_status_at(dir.path(), now + Duration::days(8)).reminder);
        // A damaged state file is the same as no backup.
        std::fs::write(dir.path().join(STATE_FILE), "not json").unwrap();
        let s = backup_status_at(dir.path(), now);
        assert!(s.reminder && s.last_backup_at.is_none() && s.last_file_name.is_none());
    }

    #[test]
    fn create_validates_before_doing_anything() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x33, "Cuenta ficticia A");
        let target = dir.path().join("copia.plbackup");
        assert_eq!(
            code(create_with(&state, PASS, "otra frase distinta", &target, MIN, VERSION)),
            "passphrase_mismatch"
        );
        assert_eq!(
            code(create_with(&state, "corta", "corta", &target, MIN, VERSION)),
            "weak_passphrase"
        );
        assert_eq!(
            code(create_with(&state, "passwordpassword", "passwordpassword", &target, MIN, VERSION)),
            "weak_passphrase"
        );
        assert!(!target.exists());
        assert!(!dir.path().join(STATE_FILE).exists());
    }

    #[test]
    fn create_needs_the_busy_guard() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x34, "Cuenta ficticia A");
        let _held = state.try_busy().unwrap();
        let target = dir.path().join("copia.plbackup");
        assert_eq!(code(create_with(&state, PASS, PASS, &target, MIN, VERSION)), "busy");
    }

    #[test]
    fn create_includes_the_bank_config_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x35, "Cuenta ficticia A");
        let cfg = br#"{"application_id":"app-ficticia","aspsp_name":"Banco de Prueba"}"#;
        std::fs::write(dir.path().join(CONFIG_FILE), cfg).unwrap();
        let target = make_backup(&state, "copia.plbackup");
        let dec = backup::read_backup(&target, PASS).unwrap();
        assert_eq!(dec.config.as_ref().unwrap().as_slice(), cfg);
    }

    #[test]
    fn restore_swaps_data_and_keeps_a_pre_restore_copy() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x36, "Cuenta antes");
        let target = make_backup(&state, "copia.plbackup");
        state
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO accounts (name, kind, currency, identity_hash, display_suffix)
                 VALUES ('Cuenta despues', 'savings', 'EUR', 'h2', '1111')",
                [],
            )
            .unwrap();
        assert_eq!(accounts(&state), ["Cuenta antes", "Cuenta despues"]);

        remember_picked(&state, target).unwrap();
        let result = restore_with(&state, PASS).unwrap();
        assert_eq!(accounts(&state), ["Cuenta antes"]);
        let pre = result.pre_restore_file_name.unwrap();
        assert!(pre.starts_with("pocket-ledger.db.pre-restore-"), "{pre}");
        assert!(dir.path().join(&pre).is_file());
        assert!(!result.backup_created_at.is_empty());
        assert!(state.picked_backup.lock().unwrap().is_none());
        // The new connection is live and writable, and the key did not change.
        state
            .db
            .lock()
            .unwrap()
            .execute("UPDATE accounts SET name = 'Renombrada'", [])
            .unwrap();
        assert_eq!(accounts(&state), ["Renombrada"]);
        assert_eq!(current_key(&state).unwrap().as_bytes(), key(0x36).as_bytes());
        // No staging leftovers; the busy guard was released.
        assert!(!file_names(dir.path()).iter().any(|n| n.ends_with(".restoring")));
        assert!(state.try_busy().is_ok());
    }

    #[test]
    fn restore_with_wrong_passphrase_leaves_the_database_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x37, "Cuenta antes");
        let target = make_backup(&state, "copia.plbackup");
        state
            .db
            .lock()
            .unwrap()
            .execute("UPDATE accounts SET name = 'Cambiada'", [])
            .unwrap();
        let names_before = file_names(dir.path());

        remember_picked(&state, target).unwrap();
        assert_eq!(code(restore_with(&state, "una frase incorrecta pero larga")), "wrong_passphrase");
        assert_eq!(accounts(&state), ["Cambiada"]);
        assert_eq!(file_names(dir.path()), names_before);
        // The pick is kept so the owner can retry.
        assert!(state.picked_backup.lock().unwrap().is_some());
        assert!(state.try_busy().is_ok());
    }

    #[test]
    fn restore_needs_a_picked_file_and_a_ready_app() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x38, "Cuenta antes");
        assert_eq!(code(restore_with(&state, PASS)), "no_backup_selected");

        remember_picked(&state, dir.path().join("no-existe.plbackup")).unwrap();
        assert_eq!(code(restore_with(&state, PASS)), "backup_io");
        assert_eq!(accounts(&state), ["Cuenta antes"]);

        *state.mode.write().unwrap() = AppMode::Recovery(RecoveryReason::KeyMissing);
        assert_eq!(code(restore_with(&state, PASS)), "recovery_mode");
        assert_eq!(
            code(create_with(&state, PASS, PASS, &dir.path().join("x.plbackup"), MIN, VERSION)),
            "recovery_mode"
        );
    }

    #[test]
    fn picking_returns_only_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x39, "Cuenta antes");
        let picked = remember_picked(&state, dir.path().join("carpeta").join("mi.plbackup")).unwrap();
        assert_eq!(picked.file_name, "mi.plbackup");
        let json = serde_json::to_string(&picked).unwrap();
        assert!(!json.contains("carpeta"));
    }

    /// Builds a recovery-mode state: the database exists but the store has no key.
    fn recovery_state(dir: &TempDir, byte: u8) -> AppState {
        {
            let _ = ready(dir, byte, "Cuenta actual");
        } // closes the connection
        let state =
            AppState::init_with_store(dir.path(), Box::new(MemoryStore::new())).unwrap();
        assert_eq!(state.mode(), AppMode::Recovery(RecoveryReason::KeyMissing));
        state
    }

    #[test]
    fn startup_without_key_enters_recovery_and_refuses_bank_work() {
        let dir = tempfile::tempdir().unwrap();
        let state = recovery_state(&dir, 0x41);
        assert_eq!(
            app_mode_view(&state),
            AppModeView { mode: "recovery", reason: Some("key_missing") }
        );
        assert_eq!(state.try_busy().unwrap_err().code, "recovery_mode");
        // Nothing was created or overwritten in the store.
        assert!(state.store.get(DB_KEY_ENTRY).unwrap().is_none());
        assert!(state.db_key.lock().unwrap().is_none());
    }

    #[test]
    fn recovery_unlock_with_the_matching_backup_returns_to_ready() {
        let src = tempfile::tempdir().unwrap();
        let backup_path = {
            let s = ready(&src, 0x42, "Cuenta de la copia");
            make_backup(&s, "copia.plbackup")
        };
        // Same key, same database file: copy the database to the recovery directory.
        let dir = tempfile::tempdir().unwrap();
        std::fs::copy(src.path().join(DB_FILE), dir.path().join(DB_FILE)).unwrap();
        let state =
            AppState::init_with_store(dir.path(), Box::new(MemoryStore::new())).unwrap();
        assert!(matches!(state.mode(), AppMode::Recovery(_)));

        assert_eq!(code(recovery_unlock_with(&state, PASS)), "no_backup_selected");
        remember_picked(&state, backup_path).unwrap();
        assert_eq!(code(recovery_unlock_with(&state, "otra frase larga equivocada")), "wrong_passphrase");
        assert!(matches!(state.mode(), AppMode::Recovery(_)));

        recovery_unlock_with(&state, PASS).unwrap();
        assert_eq!(state.mode(), AppMode::Ready);
        assert_eq!(app_mode_view(&state).mode, "ready");
        assert_eq!(accounts(&state), ["Cuenta de la copia"]);
        assert_eq!(
            state.store.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(),
            key(0x42).as_bytes()
        );
        assert!(state.try_busy().is_ok());
        assert!(state.picked_backup.lock().unwrap().is_none());
    }

    #[test]
    fn recovery_unlock_with_a_foreign_backup_is_refused() {
        let other = tempfile::tempdir().unwrap();
        let backup_path = {
            let s = ready(&other, 0x51, "Otra instalacion");
            make_backup(&s, "ajena.plbackup")
        };
        let dir = tempfile::tempdir().unwrap();
        let state = recovery_state(&dir, 0x52);
        remember_picked(&state, backup_path).unwrap();
        assert_eq!(code(recovery_unlock_with(&state, PASS)), "backup_key_mismatch");
        assert!(matches!(state.mode(), AppMode::Recovery(_)));
        assert!(state.store.get(DB_KEY_ENTRY).unwrap().is_none());
    }

    #[test]
    fn recovery_restore_replaces_the_unreadable_database() {
        let other = tempfile::tempdir().unwrap();
        let backup_path = {
            let s = ready(&other, 0x61, "Cuenta restaurada");
            make_backup(&s, "copia.plbackup")
        };
        let dir = tempfile::tempdir().unwrap();
        let state = recovery_state(&dir, 0x62);
        remember_picked(&state, backup_path).unwrap();

        recovery_restore_with(&state, PASS).unwrap();
        assert_eq!(state.mode(), AppMode::Ready);
        assert_eq!(accounts(&state), ["Cuenta restaurada"]);
        // A fresh key is now the stored key and the old file is kept aside.
        let stored = state.store.get(DB_KEY_ENTRY).unwrap().unwrap();
        assert_eq!(stored.len(), KEY_LEN);
        assert_eq!(current_key(&state).unwrap().as_bytes().as_slice(), stored.as_slice());
        assert_ne!(stored.as_slice(), key(0x62).as_bytes());
        assert!(file_names(dir.path()).iter().any(|n| n.starts_with("pocket-ledger.db.unreadable-")));
        assert!(state.try_busy().is_ok());
    }

    #[test]
    fn recovery_commands_are_refused_when_ready() {
        let dir = tempfile::tempdir().unwrap();
        let state = ready(&dir, 0x71, "Cuenta antes");
        remember_picked(&state, dir.path().join("x.plbackup")).unwrap();
        assert_eq!(code(recovery_unlock_with(&state, PASS)), "internal");
        assert_eq!(code(recovery_restore_with(&state, PASS)), "internal");
    }

    #[test]
    fn passphrase_check_reports_score_and_feedback() {
        let weak = check_passphrase_view("abc");
        assert!(!weak.ok && weak.feedback.is_some());
        let strong = check_passphrase_view(PASS);
        assert!(strong.ok && strong.feedback.is_none() && strong.score >= 3);
    }

    #[test]
    fn default_file_name_follows_the_documented_pattern() {
        let n = default_file_name();
        assert!(n.starts_with("pocket-ledger-") && n.ends_with(".plbackup"), "{n}");
        // pocket-ledger-AAAA-MM-DD-HHMM.plbackup
        assert_eq!(n.len(), "pocket-ledger-2026-11-16-1530.plbackup".len());
        assert_eq!(with_extension(PathBuf::from("a")).extension().unwrap(), "plbackup");
        assert_eq!(with_extension(PathBuf::from("a.plbackup")), PathBuf::from("a.plbackup"));
    }
}
