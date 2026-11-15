//! Managed application state: the encrypted database, the secret store and the
//! "bank busy" guard.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ledger_core::db;
use ledger_core::keystore::{load_or_create_db_key, OsKeyring, SecretStore};
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

pub struct AppState {
    pub db: Mutex<Connection>,
    pub store: Box<dyn SecretStore>,
    /// Held by the one connect / sync / disconnect that is running.
    busy: tokio::sync::Mutex<()>,
    pub data_dir: PathBuf,
}

impl AppState {
    /// Production startup: key from the OS keyring, database in `data_dir`.
    pub fn init(data_dir: &Path) -> Result<Self, StateError> {
        std::fs::create_dir_all(data_dir).map_err(|_| StateError::DataDir)?;
        let store = OsKeyring;
        let key = load_or_create_db_key(&store).map_err(|_| StateError::Key)?;
        let conn = db::open(&data_dir.join(DB_FILE), &key).map_err(|_| StateError::Database)?;
        Ok(Self::new(conn, Box::new(store), data_dir.to_path_buf()))
    }

    pub fn new(conn: Connection, store: Box<dyn SecretStore>, data_dir: PathBuf) -> Self {
        Self { db: Mutex::new(conn), store, busy: tokio::sync::Mutex::new(()), data_dir }
    }

    /// Takes the busy guard or fails with `busy`; never waits.
    pub fn try_busy(&self) -> Result<AsyncGuard<'_, ()>, CommandError> {
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
}
