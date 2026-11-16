//! Where secrets live. The app talks to the [`SecretStore`] trait; production uses
//! [`OsKeyring`] (Windows Credential Manager, service "pocket-ledger") and tests use
//! [`MemoryStore`]. Debug runs with a data dir override use the separate service
//! [`DEV_SERVICE`] (see [`service_for`]) so they never read or overwrite the
//! production entries. [`load_or_create_db_key`] returns the database key, generating
//! and saving it on first run. If a stored key exists but is malformed we fail
//! instead of generating a new one, because that would orphan the encrypted database.
//!
//! Note: `keyring` is built with only the `windows-native` backend; other platforms
//! need their own feature enabled before shipping there.

use std::collections::HashMap;
use std::sync::Mutex;

use thiserror::Error;
use zeroize::Zeroizing;

use crate::db::{DbError, DbKey, KEY_LEN};

/// Service name under which entries are stored in the OS secret store.
pub const SERVICE: &str = "pocket-ledger";
/// Service name used by debug runs whose data dir is overridden, so a development
/// database or bank session never shares entries with the production app.
pub const DEV_SERVICE: &str = "pocket-ledger-dev";
/// Entry name of the database key.
pub const DB_KEY_ENTRY: &str = "db-key";

#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("secret store backend error: {0}")]
    Backend(String),
    #[error("the stored database key is malformed; refusing to replace it")]
    MalformedStoredKey,
    #[error("secret store lock poisoned")]
    Poisoned,
    #[error(transparent)]
    Db(#[from] DbError),
}

pub trait SecretStore: Send + Sync {
    /// Returns the secret, or `None` if no entry exists.
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeystoreError>;
    /// Creates or overwrites the entry.
    fn set(&self, name: &str, value: &[u8]) -> Result<(), KeystoreError>;
    /// Removes the entry; deleting a missing entry is not an error.
    fn delete(&self, name: &str) -> Result<(), KeystoreError>;
}

/// Keyring service for a run: [`DEV_SERVICE`] when the (debug-only) data dir
/// override is active, [`SERVICE`] otherwise.
pub fn service_for(data_dir_overridden: bool) -> &'static str {
    if data_dir_overridden {
        DEV_SERVICE
    } else {
        SERVICE
    }
}

/// Secret store backed by the operating system keyring, under one service name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsKeyring {
    service: String,
}

impl OsKeyring {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, KeystoreError> {
        keyring::Entry::new(&self.service, name).map_err(|e| KeystoreError::Backend(e.to_string()))
    }
}

impl Default for OsKeyring {
    /// The production service, [`SERVICE`].
    fn default() -> Self {
        Self::new(SERVICE)
    }
}

impl SecretStore for OsKeyring {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeystoreError> {
        match self.entry(name)?.get_secret() {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeystoreError::Backend(e.to_string())),
        }
    }

    fn set(&self, name: &str, value: &[u8]) -> Result<(), KeystoreError> {
        self.entry(name)?
            .set_secret(value)
            .map_err(|e| KeystoreError::Backend(e.to_string()))
    }

    fn delete(&self, name: &str) -> Result<(), KeystoreError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeystoreError::Backend(e.to_string())),
        }
    }
}

/// In-memory store for tests.
#[derive(Debug, Default)]
pub struct MemoryStore {
    items: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeystoreError> {
        let items = self.items.lock().map_err(|_| KeystoreError::Poisoned)?;
        Ok(items.get(name).map(|v| Zeroizing::new(v.clone())))
    }

    fn set(&self, name: &str, value: &[u8]) -> Result<(), KeystoreError> {
        let mut items = self.items.lock().map_err(|_| KeystoreError::Poisoned)?;
        items.insert(name.to_string(), value.to_vec());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), KeystoreError> {
        let mut items = self.items.lock().map_err(|_| KeystoreError::Poisoned)?;
        items.remove(name);
        Ok(())
    }
}

/// Returns the database key from the store, creating and saving one on first run.
pub fn load_or_create_db_key(store: &dyn SecretStore) -> Result<DbKey, KeystoreError> {
    if let Some(stored) = store.get(DB_KEY_ENTRY)? {
        let bytes: [u8; KEY_LEN] = stored
            .as_slice()
            .try_into()
            .map_err(|_| KeystoreError::MalformedStoredKey)?;
        return Ok(DbKey::from_bytes(bytes));
    }
    let key = DbKey::generate()?;
    store.set(DB_KEY_ENTRY, key.as_bytes())?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryStore::new();
        assert!(store.get("a").unwrap().is_none());
        store.set("a", b"one").unwrap();
        assert_eq!(store.get("a").unwrap().unwrap().as_slice(), b"one");
        store.set("a", b"two").unwrap();
        assert_eq!(store.get("a").unwrap().unwrap().as_slice(), b"two");
        store.delete("a").unwrap();
        assert!(store.get("a").unwrap().is_none());
        store.delete("a").unwrap(); // deleting twice is fine
    }

    #[test]
    fn first_run_creates_and_second_run_reuses_key() {
        let store = MemoryStore::new();
        let first = load_or_create_db_key(&store).unwrap();
        let second = load_or_create_db_key(&store).unwrap();
        assert_eq!(first.as_bytes(), second.as_bytes());
        let stored = store.get(DB_KEY_ENTRY).unwrap().unwrap();
        assert_eq!(stored.as_slice(), first.as_bytes());
    }

    #[test]
    fn malformed_stored_key_is_not_replaced() {
        let store = MemoryStore::new();
        store.set(DB_KEY_ENTRY, b"short").unwrap();
        assert!(matches!(
            load_or_create_db_key(&store),
            Err(KeystoreError::MalformedStoredKey)
        ));
        assert_eq!(
            store.get(DB_KEY_ENTRY).unwrap().unwrap().as_slice(),
            b"short"
        );
    }

    #[test]
    fn stored_key_opens_a_database() {
        let store = MemoryStore::new();
        let key = load_or_create_db_key(&store).unwrap();
        let conn = crate::db::open_in_memory(&key).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 46);
    }

    #[test]
    fn production_service_is_the_default() {
        assert_eq!(OsKeyring::default().service(), "pocket-ledger");
        assert_eq!(service_for(false), SERVICE);
    }

    #[test]
    fn data_dir_override_selects_the_dev_service() {
        assert_eq!(service_for(true), "pocket-ledger-dev");
        assert_ne!(service_for(true), service_for(false));
        assert_eq!(OsKeyring::new(service_for(true)).service(), DEV_SERVICE);
    }

    /// Touches the real Windows Credential Manager with a throwaway entry.
    /// Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn os_keyring_roundtrip() {
        let store = OsKeyring::new(DEV_SERVICE);
        let name = "pocket-ledger-test";
        store.delete(name).unwrap();
        assert!(store.get(name).unwrap().is_none());
        store.set(name, &[1, 2, 3, 4]).unwrap();
        assert_eq!(store.get(name).unwrap().unwrap().as_slice(), &[1, 2, 3, 4]);
        store.delete(name).unwrap();
        assert!(store.get(name).unwrap().is_none());
    }
}
