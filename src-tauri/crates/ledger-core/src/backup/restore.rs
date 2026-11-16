//! Restoring a decrypted backup: prepare off to the side, then swap files.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::error::BackupError;
use super::snapshot;
use super::util::{self, aside_path, remove_with_sidecars, with_suffix, SIDECARS};
use super::DecryptedBackup;
use crate::db::{self, DbError, DbKey};

const STAGING_PREFIX: &str = ".restore-staging-";

/// A fully built, migrated and checked replacement database, encrypted with the
/// installation's current key, waiting next to the live file. Dropping it without
/// calling [`swap_in`] deletes it.
#[derive(Debug)]
pub struct PreparedRestore {
    path: PathBuf,
    armed: bool,
}

impl PreparedRestore {
    /// Where the replacement database lives (`<db file>.restoring`).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PreparedRestore {
    fn drop(&mut self) {
        if self.armed {
            remove_with_sidecars(&self.path);
        }
    }
}

/// Builds the replacement database without touching the live one.
///
/// The embedded database is staged, checked (integrity, schema not newer than
/// this build), exported into `<db_file_name>.restoring` encrypted with
/// `current_key`, then opened with forward migrations and checked again. Staging
/// files are removed on every path.
pub fn prepare_restore(
    backup: &DecryptedBackup,
    data_dir: &Path,
    current_key: &DbKey,
    db_file_name: &str,
) -> Result<PreparedRestore, BackupError> {
    let restoring = data_dir.join(format!("{db_file_name}.restoring"));
    remove_with_sidecars(&restoring);
    let prepared = PreparedRestore {
        path: restoring,
        armed: true,
    };

    {
        let staged = snapshot::stage_embedded(backup, data_dir, STAGING_PREFIX)?;
        let version = snapshot::user_version(&staged.conn)?;
        if version > db::latest_schema_version() {
            return Err(BackupError::NewerSchema);
        }
        snapshot::export_to(&staged.conn, &prepared.path, current_key, version)?;
    } // staging connection closed and file removed here

    {
        let conn = db::open(&prepared.path, current_key)?;
        if !snapshot::integrity_ok(&conn)? {
            return Err(BackupError::IntegrityCheckFailed);
        }
        // Leave a single self-contained file: no -wal/-shm to move around later.
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    }
    Ok(prepared)
}

type Opener = dyn Fn(&Path, &DbKey) -> Result<Connection, DbError>;

/// Puts the prepared database in place of `db_path` and opens it.
///
/// The caller guarantees that no connection to `db_path` is open (and has
/// checkpointed the WAL with TRUNCATE before closing it). Returns the new
/// connection and, if a database existed, the path of the pre-restore copy.
/// If `db_path` does not exist (recovery mode) the prepared file is just renamed
/// into place.
///
/// On failure the previous database is renamed back; the error says whether that
/// worked. After [`BackupError::RestoreRolledBack`] the caller reopens
/// `db_path` with the key it already holds.
pub fn swap_in(
    prepared: PreparedRestore,
    db_path: &Path,
    current_key: &DbKey,
) -> Result<(Connection, Option<PathBuf>), BackupError> {
    swap_in_with(prepared, db_path, current_key, &db::open)
}

pub(crate) fn swap_in_with(
    mut prepared: PreparedRestore,
    db_path: &Path,
    current_key: &DbKey,
    opener: &Opener,
) -> Result<(Connection, Option<PathBuf>), BackupError> {
    let existing = db_path.exists();
    let pre = if existing {
        Some(aside_path(db_path, ".pre-restore", &util::utc_stamp()))
    } else {
        None
    };

    // Move the live database (and any leftover sidecars) out of the way.
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    if let Some(pre) = &pre {
        let mut pairs = vec![(db_path.to_path_buf(), pre.clone())];
        for s in SIDECARS {
            let from = with_suffix(db_path, s);
            if from.exists() {
                pairs.push((from, with_suffix(pre, s)));
            }
        }
        for (from, to) in pairs {
            if let Err(e) = fs::rename(&from, &to) {
                // Nothing of the new database is in place yet: undo what moved.
                undo_moves(&moved);
                return Err(e.into());
            }
            moved.push((from, to));
        }
    }

    if let Err(e) = fs::rename(&prepared.path, db_path) {
        return Err(rollback_error(&moved, db_path, e.into()));
    }
    match opener(db_path, current_key) {
        Ok(conn) => {
            prepared.armed = false;
            Ok((conn, pre))
        }
        Err(open_err) => {
            // Discard the failed replacement, then restore the original.
            remove_with_sidecars(db_path);
            if moved.is_empty() {
                return Err(BackupError::Db(open_err));
            }
            Err(rollback_error(&moved, db_path, BackupError::Db(open_err)))
        }
    }
}

/// Renames moved files back. Returns true if every rename worked.
fn undo_moves(moved: &[(PathBuf, PathBuf)]) -> bool {
    let mut all_ok = true;
    for (from, to) in moved.iter().rev() {
        if fs::rename(to, from).is_err() {
            all_ok = false;
        }
    }
    all_ok
}

fn rollback_error(moved: &[(PathBuf, PathBuf)], db_path: &Path, cause: BackupError) -> BackupError {
    if moved.is_empty() {
        return cause;
    }
    // The new file must not be in the way of the original.
    if db_path.exists() && !moved.iter().any(|(from, _)| from == db_path) {
        remove_with_sidecars(db_path);
    }
    if undo_moves(moved) {
        BackupError::RestoreRolledBack
    } else {
        BackupError::RestoreFailedAndRollbackFailed
    }
}

/// Installs a configuration file next to the database: the existing one is kept
/// as `<file_name>.pre-restore-<UTC>` and the new content is written atomically.
/// Returns the path of the kept copy, if any.
pub fn install_config(
    data_dir: &Path,
    file_name: &str,
    bytes: &[u8],
) -> Result<Option<PathBuf>, BackupError> {
    let plain_name = Path::new(file_name)
        .file_name()
        .is_some_and(|n| n == std::ffi::OsStr::new(file_name));
    if !plain_name {
        return Err(BackupError::Io(std::io::ErrorKind::InvalidInput));
    }
    let target = data_dir.join(file_name);
    let kept = if target.exists() {
        let aside = aside_path(&target, ".pre-restore", &util::utc_stamp());
        fs::rename(&target, &aside)?;
        Some(aside)
    } else {
        None
    };
    util::write_atomic(&target, bytes)?;
    Ok(kept)
}

/// Removes leftovers of interrupted backups and restores: `.backup-snapshot-*`,
/// `.restore-staging-*`, `*.restoring` (and its `-wal`/`-shm`) and `*.partial`.
/// Returns how many files were removed. Nothing else in the directory is touched.
pub fn cleanup_stale(data_dir: &Path) -> Result<usize, BackupError> {
    let mut removed = 0;
    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = name.starts_with(super::create::SNAPSHOT_PREFIX)
            || name.starts_with(STAGING_PREFIX)
            || [".restoring", ".restoring-wal", ".restoring-shm", ".partial"]
                .iter()
                .any(|s| name.ends_with(s));
        if stale && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}
