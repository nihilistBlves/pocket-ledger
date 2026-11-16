//! Managed application state: the encrypted database, the secret store and the
//! "bank busy" guard.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use ledger_core::backup::{self, RecoveryReason, StartupOutcome};
use ledger_core::db::{self, DbKey};
use ledger_core::keystore::{OsKeyring, SecretStore};
use rusqlite::Connection;
use thiserror::Error;
use tokio::sync::MutexGuard as AsyncGuard;

use crate::errors::{CommandError, ErrorCode};

pub const DB_FILE: &str = "pocket-ledger.db";

/// Startup failures. Fixed texts: nothing from the keyring or the database.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("could not create the application data directory")]
    DataDir,
    #[error("could not read or create the database key in the OS secret store")]
    Key,
    #[error("could not open the encrypted database")]
    Database,
}

/// Whether the app can use its database. In recovery mode the database is not
/// open (see ADR 0005) and only the recovery commands work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Ready,
    Recovery(RecoveryReason),
}

pub struct AppState {
    /// In recovery mode this holds a placeholder in-memory connection.
    pub db: Mutex<Connection>,
    pub store: Box<dyn SecretStore>,
    /// Held by the one connect / sync / disconnect / backup / restore that is running.
    busy: tokio::sync::Mutex<()>,
    pub data_dir: PathBuf,
    pub mode: RwLock<AppMode>,
    /// The current database key, needed to back up and restore. Never serialized.
    pub db_key: Mutex<Option<DbKey>>,
    /// Backup file chosen in the native dialog (the UI never sends paths).
    pub picked_backup: Mutex<Option<PathBuf>>,
}

impl AppState {
    /// Production startup: key from the OS keyring, database in `data_dir`.
    pub fn init(data_dir: &Path) -> Result<Self, StateError> {
        Self::init_with_store(data_dir, Box::new(OsKeyring))
    }

    /// Startup with an injectable secret store. Stale backup leftovers are removed
    /// first, then the key/database pair is checked; a mismatch starts recovery mode.
    pub fn init_with_store(
        data_dir: &Path,
        store: Box<dyn SecretStore>,
    ) -> Result<Self, StateError> {
        std::fs::create_dir_all(data_dir).map_err(|_| StateError::DataDir)?;
        let _ = backup::cleanup_stale(data_dir);
        let db_path = data_dir.join(DB_FILE);
        let outcome = backup::startup_check(&db_path, store.as_ref()).map_err(|e| match e {
            backup::BackupError::Keystore(_) => StateError::Key,
            _ => StateError::Database,
        })?;
        match outcome {
            StartupOutcome::Ready(key) => {
                let conn = db::open(&db_path, &key).map_err(|_| StateError::Database)?;
                Ok(Self::new_ready(conn, store, data_dir.to_path_buf(), Some(key)))
            }
            StartupOutcome::Recovery { reason } => {
                let placeholder =
                    Connection::open_in_memory().map_err(|_| StateError::Database)?;
                let state = Self::new_ready(placeholder, store, data_dir.to_path_buf(), None);
                *state.mode.write().map_err(|_| StateError::Database)? =
                    AppMode::Recovery(reason);
                Ok(state)
            }
        }
    }

    /// Ready state without a known key (tests).
    #[cfg(test)]
    pub fn new(conn: Connection, store: Box<dyn SecretStore>, data_dir: PathBuf) -> Self {
        Self::new_ready(conn, store, data_dir, None)
    }

    pub fn new_ready(
        conn: Connection,
        store: Box<dyn SecretStore>,
        data_dir: PathBuf,
        key: Option<DbKey>,
    ) -> Self {
        Self {
            db: Mutex::new(conn),
            store,
            busy: tokio::sync::Mutex::new(()),
            data_dir,
            mode: RwLock::new(AppMode::Ready),
            db_key: Mutex::new(key),
            picked_backup: Mutex::new(None),
        }
    }

    pub fn mode(&self) -> AppMode {
        self.mode.read().map_or(AppMode::Ready, |m| *m)
    }

    /// Fails with `recovery_mode` unless the database is open and usable.
    pub fn require_ready(&self) -> Result<(), CommandError> {
        match self.mode.read() {
            Ok(m) if *m == AppMode::Ready => Ok(()),
            Ok(_) => Err(CommandError::new(ErrorCode::RecoveryMode)),
            Err(_) => Err(CommandError::new(ErrorCode::Internal)),
        }
    }

    /// Takes the busy guard or fails with `busy`; never waits. Refused in recovery mode.
    pub fn try_busy(&self) -> Result<AsyncGuard<'_, ()>, CommandError> {
        self.require_ready()?;
        self.try_busy_unchecked()
    }

    /// Like [`try_busy`](Self::try_busy) but also usable in recovery mode (recovery commands).
    pub fn try_busy_unchecked(&self) -> Result<AsyncGuard<'_, ()>, CommandError> {
        self.busy.try_lock().map_err(|_| CommandError::new(ErrorCode::Busy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::db::{open_in_memory, DbKey, KEY_LEN};
    use ledger_core::keystore::MemoryStore;

    fn state() -> AppState {
        let conn = open_in_memory(&DbKey::from_bytes([0x11; KEY_LEN])).unwrap();
        AppState::new(conn, Box::new(MemoryStore::new()), PathBuf::new())
    }

    #[test]
    fn only_one_bank_operation_at_a_time() {
        let s = state();
        let first = s.try_busy().unwrap();
        assert_eq!(s.try_busy().unwrap_err().code, "busy");
        drop(first);
        assert!(s.try_busy().is_ok());
    }

    #[test]
    fn busy_guard_is_refused_in_recovery_mode_but_recovery_can_take_it() {
        let s = state();
        *s.mode.write().unwrap() = AppMode::Recovery(RecoveryReason::KeyMissing);
        assert_eq!(s.try_busy().unwrap_err().code, "recovery_mode");
        assert_eq!(s.require_ready().unwrap_err().code, "recovery_mode");
        assert!(s.try_busy_unchecked().is_ok());
    }
}
