//! SQLCipher export helpers: copy a whole database into a new encrypted file.

use std::path::Path;

use rusqlite::Connection;
use zeroize::Zeroizing;

use super::error::BackupError;
use crate::db::DbKey;

/// `PRAGMA user_version` of the main database.
pub(crate) fn user_version(conn: &Connection) -> Result<i64, BackupError> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// True when `PRAGMA integrity_check` returns exactly one row, `ok`.
pub(crate) fn integrity_ok(conn: &Connection) -> Result<bool, BackupError> {
    let mut stmt = conn.prepare("PRAGMA integrity_check")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows.len() == 1 && rows[0] == "ok")
}

/// Copies the main database of `conn` into a new file at `dest`, encrypted with
/// `dest_key`, and sets the copy's `user_version` (the export does not carry it).
/// The destination must not exist yet. The key travels as a bound parameter,
/// never inside SQL text.
pub(crate) fn export_to(
    conn: &Connection,
    dest: &Path,
    dest_key: &DbKey,
    user_version: i64,
) -> Result<(), BackupError> {
    let dest_str = dest
        .to_str()
        .ok_or(BackupError::Io(std::io::ErrorKind::InvalidInput))?;
    let key_text = Zeroizing::new(format!("x'{}'", dest_key.to_hex().as_str()));
    conn.execute(
        "ATTACH DATABASE ?1 AS pl_snapshot KEY ?2",
        rusqlite::params![dest_str, key_text.as_str()],
    )?;
    let work = (|| -> Result<(), BackupError> {
        conn.execute_batch("SELECT sqlcipher_export('pl_snapshot')")?;
        // `user_version` is an integer we produced ourselves, safe to inline.
        conn.execute_batch(&format!("PRAGMA pl_snapshot.user_version = {user_version}"))?;
        Ok(())
    })();
    let detach = conn.execute_batch("DETACH DATABASE pl_snapshot");
    work?;
    detach?;
    Ok(())
}

/// An embedded database written to a temp file and opened with its own key,
/// without migrations. The connection closes before the file is removed.
pub(crate) struct Staged {
    pub conn: Connection,
    // Field order matters: dropped after `conn`.
    _file: super::util::TempFile,
}

/// Writes the backup's database to a temp file in `dir`, opens it with the
/// backup's key and runs the integrity check. Returns the open staged database.
pub(crate) fn stage_embedded(
    backup: &super::DecryptedBackup,
    dir: &Path,
    prefix: &str,
) -> Result<Staged, BackupError> {
    let file = super::util::TempFile::new(dir, prefix)?;
    std::fs::write(file.path(), backup.db.as_slice())?;
    let conn = crate::db::open_unmigrated(file.path(), &backup.db_key).map_err(|e| match e {
        crate::db::DbError::WrongKeyOrCorrupt => BackupError::IntegrityCheckFailed,
        other => BackupError::Db(other),
    })?;
    if !integrity_ok(&conn)? {
        return Err(BackupError::IntegrityCheckFailed);
    }
    Ok(Staged { conn, _file: file })
}
