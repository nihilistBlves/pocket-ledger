//! Creating, reading and verifying backup files.

use std::fs;
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use zeroize::Zeroizing;

use super::error::BackupError;
use super::format::{self, KdfParams, Manifest, FORMAT_VERSION, HEADER_LEN, MAX_PLAINTEXT};
use super::snapshot;
use super::util::{self, TempFile};
use super::{passphrase, DecryptedBackup};
use crate::db::DbKey;

/// Prefix of temporary snapshot files in the data directory.
pub(crate) const SNAPSHOT_PREFIX: &str = ".backup-snapshot-";

/// What the UI needs to know about a freshly created, verified backup.
#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    pub created_at: String,
    pub file_name: String,
    pub size: u64,
}

/// Creates an encrypted, verified backup of the database behind `conn` at `target`.
///
/// `conn` must be the app's single connection (the app is the only writer, so the
/// snapshot is consistent). `config` is the bank config file, if any. The target
/// is written as `<target>.partial` and renamed only when complete; the result is
/// then read back, decrypted and integrity-checked. On a failed verification the
/// target is removed.
#[allow(clippy::too_many_arguments)]
pub fn create_backup(
    conn: &Connection,
    db_key: &DbKey,
    data_dir: &Path,
    config: Option<&[u8]>,
    passphrase_text: &str,
    target: &Path,
    params: KdfParams,
    app_version: &str,
) -> Result<BackupInfo, BackupError> {
    if !passphrase::check(passphrase_text).ok {
        return Err(BackupError::WeakPassphrase);
    }
    if !params.in_bounds() {
        return Err(BackupError::InvalidHeader);
    }
    let schema_version = snapshot::user_version(conn)?;

    // 1. Encrypted snapshot of the database, in a temp file removed on every path.
    let snapshot_file = TempFile::new(data_dir, SNAPSHOT_PREFIX)?;
    snapshot::export_to(conn, snapshot_file.path(), db_key, schema_version)?;
    let db_bytes = Zeroizing::new(fs::read(snapshot_file.path())?);
    drop(snapshot_file);

    // 2. Plaintext (zeroized) -> ciphertext -> file.
    let config_bytes = config.unwrap_or(&[]);
    let created_at = util::utc_rfc3339();
    let manifest = Manifest {
        format: FORMAT_VERSION,
        created_at: created_at.clone(),
        app_version: app_version.to_string(),
        schema_version,
        db_len: db_bytes.len() as u64,
        config_len: config_bytes.len() as u64,
    };
    let plaintext = format::build_plaintext(db_key, &manifest, &db_bytes, config_bytes)?;
    drop(db_bytes);
    let file_bytes = format::seal(plaintext, passphrase_text, params)?;
    util::write_atomic(target, &file_bytes)?;
    let size = file_bytes.len() as u64;
    drop(file_bytes);

    // 3. Verify what is on disk, not what we think we wrote.
    if let Err(e) = verify_written(target, passphrase_text, db_key, &manifest, data_dir) {
        let _ = fs::remove_file(target);
        return Err(e);
    }

    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(BackupInfo {
        created_at,
        file_name,
        size,
    })
}

fn verify_written(
    target: &Path,
    passphrase_text: &str,
    db_key: &DbKey,
    expected: &Manifest,
    data_dir: &Path,
) -> Result<(), BackupError> {
    let read_back = read_backup(target, passphrase_text)?;
    if read_back.db_key.as_bytes() != db_key.as_bytes() || read_back.manifest != *expected {
        return Err(BackupError::IntegrityCheckFailed);
    }
    let staged = snapshot::stage_embedded(&read_back, data_dir, SNAPSHOT_PREFIX)?;
    if snapshot::user_version(&staged.conn)? != expected.schema_version {
        return Err(BackupError::IntegrityCheckFailed);
    }
    Ok(())
}

/// Reads and decrypts a backup file.
pub fn read_backup(path: &Path, passphrase_text: &str) -> Result<DecryptedBackup, BackupError> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_PLAINTEXT + 16 + HEADER_LEN as u64 {
        return Err(BackupError::TooLarge);
    }
    let file = fs::read(path)?;
    let plain = format::open(&file, passphrase_text)?;
    drop(file);
    format::parse_plaintext(&plain)
}

/// Checks that `path` decrypts with `passphrase_text` and is well formed, and
/// returns its manifest.
pub fn verify_backup(path: &Path, passphrase_text: &str) -> Result<Manifest, BackupError> {
    Ok(read_backup(path, passphrase_text)?.manifest)
}
