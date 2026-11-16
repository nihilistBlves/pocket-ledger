//! Backup errors. Messages are fixed strings: they never carry keys, passphrases,
//! plaintext or file paths (I/O errors keep only the [`std::io::ErrorKind`]).

use thiserror::Error;

use crate::db::DbError;
use crate::keystore::KeystoreError;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("the file is not a Pocket Ledger backup")]
    NotABackup,
    #[error("the backup header is invalid")]
    InvalidHeader,
    #[error("unsupported backup format version {0}")]
    UnsupportedVersion(u16),
    #[error("wrong passphrase, or the backup file is corrupt")]
    WrongPassphraseOrCorrupt,
    #[error("the passphrase is too weak")]
    WeakPassphrase,
    #[error("the backup is too large")]
    TooLarge,
    #[error("the backup comes from a newer version of the app")]
    NewerSchema,
    #[error("the database failed its integrity check")]
    IntegrityCheckFailed,
    #[error("the key in the backup does not open the current database")]
    BackupKeyDoesNotOpenDatabase,
    #[error("the restore failed and the previous database was put back")]
    RestoreRolledBack,
    #[error("the restore failed and the previous database could not be put back")]
    RestoreFailedAndRollbackFailed,
    #[error("file system error ({0:?})")]
    Io(std::io::ErrorKind),
    #[error("database error: {0}")]
    Db(DbError),
    #[error("secret store error: {0}")]
    Keystore(KeystoreError),
    #[error("the operating system random generator failed")]
    Random,
}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        BackupError::Io(e.kind())
    }
}

impl From<DbError> for BackupError {
    fn from(e: DbError) -> Self {
        BackupError::Db(e)
    }
}

impl From<rusqlite::Error> for BackupError {
    fn from(e: rusqlite::Error) -> Self {
        BackupError::Db(DbError::Sqlite(e))
    }
}

impl From<KeystoreError> for BackupError {
    fn from(e: KeystoreError) -> Self {
        BackupError::Keystore(e)
    }
}
