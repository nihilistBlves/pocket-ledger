//! Startup check and recovery when the database cannot be opened with the key
//! in the secret store. In recovery mode nothing is created, overwritten or
//! deleted: unreadable files are renamed aside, replaced keys are saved aside.

use std::fs;
use std::path::Path;

use rusqlite::Connection;

use super::error::BackupError;
use super::util::{self, aside_path, with_suffix, SIDECARS};
use super::restore::{prepare_restore, swap_in};
use super::DecryptedBackup;
use crate::db::{self, DbError, DbKey, KEY_LEN};
use crate::keystore::{self, SecretStore, DB_KEY_ENTRY};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryReason {
    /// A database exists but the secret store has no key.
    KeyMissing,
    /// The stored key is not 32 bytes.
    KeyMalformed,
    /// The stored key does not open the database.
    KeyWrong,
}

#[derive(Debug)]
pub enum StartupOutcome {
    Ready(DbKey),
    Recovery { reason: RecoveryReason },
}

/// Decides whether the app can start normally. See ADR 0005, "Recovery mode".
/// In the `Recovery` case the key store and the database file are left untouched.
pub fn startup_check(
    db_path: &Path,
    store: &dyn SecretStore,
) -> Result<StartupOutcome, BackupError> {
    let has_data = match fs::metadata(db_path) {
        Ok(m) => m.len() > 0,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    let stored = store.get(DB_KEY_ENTRY)?;

    if !has_data {
        // Fresh install. A malformed entry is never overwritten.
        if stored.as_ref().is_some_and(|v| v.len() != KEY_LEN) {
            return Ok(StartupOutcome::Recovery {
                reason: RecoveryReason::KeyMalformed,
            });
        }
        return Ok(StartupOutcome::Ready(keystore::load_or_create_db_key(
            store,
        )?));
    }

    let Some(stored) = stored else {
        return Ok(StartupOutcome::Recovery {
            reason: RecoveryReason::KeyMissing,
        });
    };
    let Ok(bytes) = <[u8; KEY_LEN]>::try_from(stored.as_slice()) else {
        return Ok(StartupOutcome::Recovery {
            reason: RecoveryReason::KeyMalformed,
        });
    };
    let key = DbKey::from_bytes(bytes);
    match db::open_unmigrated(db_path, &key) {
        Ok(_conn) => Ok(StartupOutcome::Ready(key)),
        Err(DbError::WrongKeyOrCorrupt) => Ok(StartupOutcome::Recovery {
            reason: RecoveryReason::KeyWrong,
        }),
        Err(other) => Err(BackupError::Db(other)),
    }
}

/// Copies the current store entry (if any) to `db-key.replaced-<UTC>`.
fn save_entry_aside(store: &dyn SecretStore) -> Result<(), BackupError> {
    let Some(existing) = store.get(DB_KEY_ENTRY)? else {
        return Ok(());
    };
    let stamp = util::utc_stamp();
    let mut name = format!("{DB_KEY_ENTRY}.replaced-{stamp}");
    let mut n = 0u32;
    while store.get(&name)?.is_some() {
        n += 1;
        name = format!("{DB_KEY_ENTRY}.replaced-{stamp}-{n}");
    }
    store.set(&name, &existing)?;
    Ok(())
}

/// Tries the key inside a backup against the existing database. If it opens, the
/// key becomes the stored key (a different previous entry is first saved as
/// `db-key.replaced-<UTC>`). Otherwise the store is not touched.
pub fn unlock_with_backup(
    db_path: &Path,
    backup: &DecryptedBackup,
    store: &dyn SecretStore,
) -> Result<DbKey, BackupError> {
    fs::metadata(db_path)?;
    match db::open_unmigrated(db_path, &backup.db_key) {
        Ok(_conn) => {}
        Err(DbError::WrongKeyOrCorrupt) => return Err(BackupError::BackupKeyDoesNotOpenDatabase),
        Err(other) => return Err(BackupError::Db(other)),
    }
    let current = store.get(DB_KEY_ENTRY)?;
    if current
        .as_ref()
        .is_none_or(|v| v.as_slice() != backup.db_key.as_bytes())
    {
        save_entry_aside(store)?;
        store.set(DB_KEY_ENTRY, backup.db_key.as_bytes())?;
    }
    Ok(backup.db_key.clone())
}

/// "Restore a backup" in recovery mode. Order matters, so that a failure at any
/// step leaves the installation no worse than before:
///
/// 1. a fresh key is generated (memory only) and the replacement database is
///    fully built and checked next to the live file (`prepare_restore`);
/// 2. only then the existing key entry, if any, is saved as
///    `db-key.replaced-<UTC>` and the fresh key is stored;
/// 3. the unreadable database is renamed to `<name>.unreadable-<UTC>` (never
///    deleted, sidecars follow) and the replacement is moved into place.
///
/// Returns the open connection and the key now in the store.
pub fn recovery_restore(
    db_path: &Path,
    backup: &DecryptedBackup,
    store: &dyn SecretStore,
) -> Result<(Connection, DbKey), BackupError> {
    let data_dir = db_path
        .parent()
        .ok_or(BackupError::Io(std::io::ErrorKind::InvalidInput))?;
    let db_file_name = db_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(BackupError::Io(std::io::ErrorKind::InvalidInput))?;

    let key = DbKey::generate()?;
    let prepared = prepare_restore(backup, data_dir, &key, db_file_name)?;

    save_entry_aside(store)?;
    store.set(DB_KEY_ENTRY, key.as_bytes())?;

    if db_path.exists() {
        let aside = aside_path(db_path, ".unreadable", &util::utc_stamp());
        fs::rename(db_path, &aside)?;
        for s in SIDECARS {
            let from = with_suffix(db_path, s);
            if from.exists() {
                fs::rename(&from, with_suffix(&aside, s))?;
            }
        }
    }
    let (conn, _) = swap_in(prepared, db_path, &key)?;
    Ok((conn, key))
}
