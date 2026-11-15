//! Encrypted local database (SQLite + SQLCipher) and schema migrations.
//!
//! The 256-bit [`DbKey`] is passed to SQLCipher as a raw key (`x'<hex>'`), so no
//! password-derivation step is involved. `PRAGMA key` must be the very first
//! statement on a connection; right after it we read `sqlite_master` to prove the
//! key is right. Schema changes are forward-only SQL files in `migrations/`,
//! embedded in the binary and applied by `rusqlite_migration`, which keeps the
//! schema version in SQLite's `user_version`. The key is never logged, formatted
//! or included in an error.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, ErrorCode};
use rusqlite_migration::{Migrations, M};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[cfg(test)]
mod tests;

/// Length of the database key in bytes (256 bits).
pub const KEY_LEN: usize = 32;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum DbError {
    #[error("wrong database key, or the file is not a valid encrypted database")]
    WrongKeyOrCorrupt,
    #[error("the database key could not be applied")]
    KeyRejected,
    #[error("SQLCipher is not available in this build (database would not be encrypted)")]
    CipherUnavailable,
    #[error("the operating system random generator failed")]
    Random,
    #[error("invalid database key encoding")]
    InvalidKey,
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
}

/// A 256-bit database key. Wiped from memory when dropped; never printed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct DbKey([u8; KEY_LEN]);

impl DbKey {
    /// Generates a fresh key from the OS cryptographic random generator.
    pub fn generate() -> Result<Self, DbError> {
        let mut bytes = [0u8; KEY_LEN];
        getrandom::fill(&mut bytes).map_err(|_| DbError::Random)?;
        Ok(DbKey(bytes))
    }

    /// Builds a key from raw bytes (e.g. read back from the secret store).
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        DbKey(bytes)
    }

    /// Builds a key from a 64-character hex string.
    pub fn from_hex(text: &str) -> Result<Self, DbError> {
        let mut bytes = [0u8; KEY_LEN];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| DbError::InvalidKey)?;
        Ok(DbKey(bytes))
    }

    /// Raw key bytes, for storing in the secret store.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Lower-case hex form (64 characters), wiped on drop.
    pub fn to_hex(&self) -> Zeroizing<String> {
        Zeroizing::new(hex::encode(self.0))
    }
}

impl std::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbKey(**redacted**)")
    }
}

/// All migrations, in order. Forward-only: never edit a shipped file, add a new one.
pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(include_str!("migrations/0001_initial.sql")),
        M::up(include_str!("migrations/0002_seed_categories.sql")),
        M::up(include_str!("migrations/0003_bank_sync.sql")),
    ])
}

/// Opens (creating if needed) the encrypted database at `path` and migrates it.
pub fn open(path: &Path, key: &DbKey) -> Result<Connection, DbError> {
    let mut conn = Connection::open(path)?;
    apply_key(&conn, key)?;
    verify_key(&conn)?;
    // WAL only makes sense for files; `:memory:` databases stay in memory mode.
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
    finish_setup(&mut conn)?;
    Ok(conn)
}

/// Encrypted in-memory database, for tests.
pub fn open_in_memory(key: &DbKey) -> Result<Connection, DbError> {
    let mut conn = Connection::open_in_memory()?;
    apply_key(&conn, key)?;
    verify_key(&conn)?;
    finish_setup(&mut conn)?;
    Ok(conn)
}

/// Returns the SQLCipher version string (empty means SQLCipher is not linked).
pub fn cipher_version(conn: &Connection) -> Result<String, DbError> {
    let version = conn
        .query_row("PRAGMA cipher_version", [], |row| row.get::<_, String>(0))
        .or_else(|e| match e {
            // Plain SQLite returns no row for this pragma.
            rusqlite::Error::QueryReturnedNoRows => Ok(String::new()),
            other => Err(other),
        })?;
    Ok(version)
}

/// Must be the first statement on the connection.
fn apply_key(conn: &Connection, key: &DbKey) -> Result<(), DbError> {
    let statement = Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", key.to_hex().as_str()));
    // The underlying error is dropped on purpose so nothing key-related can leak.
    conn.execute_batch(&statement)
        .map_err(|_| DbError::KeyRejected)
}

/// Reads the schema table: this fails with NOTADB if the key is wrong.
fn verify_key(conn: &Connection) -> Result<(), DbError> {
    let result = conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    });
    match result {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == ErrorCode::NotADatabase => {
            return Err(DbError::WrongKeyOrCorrupt);
        }
        Err(e) => return Err(DbError::Sqlite(e)),
    }
    // Without SQLCipher the key pragma is silently ignored: refuse to continue.
    if cipher_version(conn)?.is_empty() {
        return Err(DbError::CipherUnavailable);
    }
    Ok(())
}

fn finish_setup(conn: &mut Connection) -> Result<(), DbError> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    migrations().to_latest(conn)?;
    Ok(())
}
