//! Where secrets live. The app talks to the [`SecretStore`] trait; production uses
//! [`OsKeyring`] (Windows Credential Manager, service "pocket-ledger") and tests use
//! [`MemoryStore`]. [`load_or_create_db_key`] returns the database key, generating
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

/// Secret store backed by the operating system keyring.
#[derive(Debug, Default)]
pub struct OsKeyring;

impl OsKeyring {
    fn entry(name: &str) -> Result<keyring::Entry, KeystoreError> {
        keyring::Entry::new(SERVICE, name).map_err(|e| KeystoreError::Backend(e.to_string()))
    }
}

impl SecretStore for OsKeyring {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, KeystoreError> {
        match Self::entry(name)?.get_secret() {
            Ok(bytes) => Ok(Some(Zeroizing::new(bytes))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeystoreError::Backend(e.to_string())),
        }
    }

    fn set(&self, name: &str, value: &[u8]) -> Result<(), KeystoreError> {
        Self::entry(name)?
            .set_secret(value)
            .map_err(|e| KeystoreError::Backend(e.to_string()))
    }

    fn delete(&self, name: &str) -> Result<(), KeystoreError> {
        match Self::entry(name)?.delete_credential() {
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

    /// Touches the real Windows Credential Manager with a throwaway entry.
    /// Run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn os_keyring_roundtrip() {
        let store = OsKeyring;
        let name = "pocket-ledger-test";
        store.delete(name).unwrap();
        assert!(store.get(name).unwrap().is_none());
        store.set(name, &[1, 2, 3, 4]).unwrap();
        assert_eq!(store.get(name).unwrap().unwrap().as_slice(), &[1, 2, 3, 4]);
        store.delete(name).unwrap();
        assert!(store.get(name).unwrap().is_none());
    }
}
