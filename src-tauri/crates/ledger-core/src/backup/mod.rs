//! Encrypted backup and restore (ADR 0005).
//!
//! A `.plbackup` file holds the SQLCipher database, the database key and the bank
//! config, sealed with XChaCha20-Poly1305 under a key derived from a passphrase
//! with Argon2id. Everything here is pure Rust with no Tauri dependency: the app
//! owns the connection swap and calls [`prepare_restore`] / [`swap_in`].
//!
//! Keys, passphrases and plaintext live only in zeroizing buffers, and no error
//! message or `Debug` output contains them.

mod create;
mod error;
pub mod format;
pub mod passphrase;
pub mod recovery;
mod restore;
mod snapshot;
mod util;

#[cfg(test)]
mod tests;

use zeroize::Zeroizing;

use crate::db::DbKey;

pub use create::{create_backup, read_backup, verify_backup, BackupInfo};
pub use error::BackupError;
pub use format::{KdfParams, Manifest};
pub use passphrase::{check as check_passphrase, PassphraseVerdict};
pub use recovery::{
    recovery_restore, startup_check, unlock_with_backup, RecoveryReason, StartupOutcome,
};
pub use restore::{cleanup_stale, install_config, prepare_restore, swap_in, PreparedRestore};

/// A decrypted backup. Holds the database key, so it is wiped on drop and its
/// `Debug` output shows only non-secret facts.
pub struct DecryptedBackup {
    pub db_key: DbKey,
    pub manifest: Manifest,
    /// The SQLCipher database file, still encrypted under `db_key`.
    pub db: Zeroizing<Vec<u8>>,
    /// Bank config file content; `None` if the backup has none.
    pub config: Option<Zeroizing<Vec<u8>>>,
}

impl std::fmt::Debug for DecryptedBackup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecryptedBackup")
            .field("manifest", &self.manifest)
            .field("db_len", &self.db.len())
            .field("config_len", &self.config.as_ref().map_or(0, |c| c.len()))
            .field("secrets", &"**redacted**")
            .finish()
    }
}
