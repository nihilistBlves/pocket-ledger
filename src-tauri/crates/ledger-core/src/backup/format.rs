//! `.plbackup` version 1: header layout, key derivation, AEAD and payload layout.
//! See `docs/adr/0005-backups.md`.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{AeadInOut, Key, KeyInit, XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use super::error::BackupError;
use crate::db::{DbKey, KEY_LEN};

pub const MAGIC: [u8; 8] = *b"PLEDGBK\0";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 72;
const KDF_ARGON2ID: u8 = 1;
const AEAD_XCHACHA20POLY1305: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// Largest plaintext (key + manifest + database + config) we create or accept.
pub const MAX_PLAINTEXT: u64 = 1 << 30;
const MAX_MANIFEST_LEN: usize = 64 * 1024;

/// Argon2id cost parameters, stored in the header of every backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl KdfParams {
    /// Production parameters: 256 MiB, 3 passes, 4 lanes.
    pub const DEFAULT: KdfParams = KdfParams {
        m_kib: 262_144,
        t: 3,
        p: 4,
    };
    /// Cheapest parameters the reader accepts (used by tests).
    pub const MIN: KdfParams = KdfParams {
        m_kib: 32_768,
        t: 1,
        p: 1,
    };

    /// Safe bounds enforced before any key derivation runs.
    pub fn in_bounds(&self) -> bool {
        (32_768..=1_048_576).contains(&self.m_kib)
            && (1..=10).contains(&self.t)
            && (1..=16).contains(&self.p)
    }
}

/// Non-secret description of the backup, stored encrypted inside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u16,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub app_version: String,
    pub schema_version: i64,
    pub db_len: u64,
    pub config_len: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Header {
    pub params: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub ct_len: u64,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

impl Header {
    pub(crate) fn encode(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0..8].copy_from_slice(&MAGIC);
        h[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        h[10] = KDF_ARGON2ID;
        h[11] = AEAD_XCHACHA20POLY1305;
        h[12..16].copy_from_slice(&self.params.m_kib.to_le_bytes());
        h[16..20].copy_from_slice(&self.params.t.to_le_bytes());
        h[20..24].copy_from_slice(&self.params.p.to_le_bytes());
        h[24..40].copy_from_slice(&self.salt);
        h[40..64].copy_from_slice(&self.nonce);
        h[64..72].copy_from_slice(&self.ct_len.to_le_bytes());
        h
    }

    /// Parses and validates the header of a whole backup file. Nothing here runs
    /// the KDF; every check that does not need the passphrase happens first.
    pub(crate) fn decode(file: &[u8]) -> Result<Header, BackupError> {
        if file.len() < HEADER_LEN || file[0..8] != MAGIC {
            return Err(BackupError::NotABackup);
        }
        let version = u16::from_le_bytes([file[8], file[9]]);
        if version != FORMAT_VERSION {
            return Err(BackupError::UnsupportedVersion(version));
        }
        if file[10] != KDF_ARGON2ID || file[11] != AEAD_XCHACHA20POLY1305 {
            return Err(BackupError::InvalidHeader);
        }
        let params = KdfParams {
            m_kib: u32_at(file, 12),
            t: u32_at(file, 16),
            p: u32_at(file, 20),
        };
        if !params.in_bounds() {
            return Err(BackupError::InvalidHeader);
        }
        let ct_len = u64::from_le_bytes(file[64..72].try_into().expect("8 bytes"));
        if ct_len != (file.len() - HEADER_LEN) as u64 {
            return Err(BackupError::InvalidHeader);
        }
        let min_ct = (KEY_LEN + 4 + TAG_LEN) as u64;
        if ct_len < min_ct {
            return Err(BackupError::InvalidHeader);
        }
        if ct_len - TAG_LEN as u64 > MAX_PLAINTEXT {
            return Err(BackupError::TooLarge);
        }
        Ok(Header {
            params,
            salt: file[24..40].try_into().expect("16 bytes"),
            nonce: file[40..64].try_into().expect("24 bytes"),
            ct_len,
        })
    }
}

#[cfg(test)]
thread_local! {
    /// Number of Argon2 runs on this thread, so tests can prove a file was
    /// rejected before the KDF.
    pub(crate) static KDF_RUNS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Passphrase (NFC) -> 32-byte key with Argon2id v0x13.
fn derive_key(
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    params: KdfParams,
) -> Result<Zeroizing<[u8; 32]>, BackupError> {
    if !params.in_bounds() {
        return Err(BackupError::InvalidHeader);
    }
    #[cfg(test)]
    KDF_RUNS.with(|c| c.set(c.get() + 1));
    let normalized: Zeroizing<String> = Zeroizing::new(passphrase.nfc().collect());
    let argon_params = Params::new(params.m_kib, params.t, params.p, Some(32))
        .map_err(|_| BackupError::InvalidHeader)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(normalized.as_bytes(), salt, key.as_mut_slice())
        .map_err(|e| match e {
            argon2::Error::OutOfMemory => BackupError::Io(std::io::ErrorKind::OutOfMemory),
            _ => BackupError::InvalidHeader,
        })?;
    Ok(key)
}

fn cipher(key: &Zeroizing<[u8; 32]>) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(&Key::try_from(key.as_slice()).expect("32-byte key"))
}

/// Encrypts `plaintext` and returns the complete backup file bytes.
pub(crate) fn seal(
    plaintext: Zeroizing<Vec<u8>>,
    passphrase: &str,
    params: KdfParams,
) -> Result<Vec<u8>, BackupError> {
    if plaintext.len() as u64 > MAX_PLAINTEXT {
        return Err(BackupError::TooLarge);
    }
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut salt).map_err(|_| BackupError::Random)?;
    getrandom::fill(&mut nonce).map_err(|_| BackupError::Random)?;
    let header = Header {
        params,
        salt,
        nonce,
        ct_len: (plaintext.len() + TAG_LEN) as u64,
    };
    let aad = header.encode();
    let key = derive_key(passphrase, &salt, params)?;
    // Encrypt in a buffer that already has room for the tag, so the plaintext is
    // never copied to a reallocated (and therefore un-wiped) block.
    let mut buffer = Zeroizing::new(Vec::with_capacity(plaintext.len() + TAG_LEN));
    buffer.extend_from_slice(&plaintext);
    drop(plaintext);
    cipher(&key)
        .encrypt_in_place(&XNonce::from(nonce), &aad, &mut *buffer)
        .map_err(|_| BackupError::WrongPassphraseOrCorrupt)?;
    let mut file = Vec::with_capacity(HEADER_LEN + buffer.len());
    file.extend_from_slice(&aad);
    file.extend_from_slice(&buffer);
    Ok(file)
}

/// Validates the header, derives the key and decrypts. Returns the plaintext.
pub(crate) fn open(file: &[u8], passphrase: &str) -> Result<Zeroizing<Vec<u8>>, BackupError> {
    let header = Header::decode(file)?;
    let key = derive_key(passphrase, &header.salt, header.params)?;
    let mut buffer = Zeroizing::new(Vec::with_capacity(file.len() - HEADER_LEN));
    buffer.extend_from_slice(&file[HEADER_LEN..]);
    cipher(&key)
        .decrypt_in_place(
            &XNonce::from(header.nonce),
            &file[..HEADER_LEN],
            &mut *buffer,
        )
        .map_err(|_| BackupError::WrongPassphraseOrCorrupt)?;
    Ok(buffer)
}

/// Builds `db_key ‖ manifest_len ‖ manifest ‖ db ‖ config`.
pub(crate) fn build_plaintext(
    key: &DbKey,
    manifest: &Manifest,
    db: &[u8],
    config: &[u8],
) -> Result<Zeroizing<Vec<u8>>, BackupError> {
    let manifest_json = serde_json::to_vec(manifest).map_err(|_| BackupError::InvalidHeader)?;
    if manifest_json.len() > MAX_MANIFEST_LEN {
        return Err(BackupError::InvalidHeader);
    }
    let total =
        KEY_LEN as u64 + 4 + manifest_json.len() as u64 + db.len() as u64 + config.len() as u64;
    if total > MAX_PLAINTEXT {
        return Err(BackupError::TooLarge);
    }
    let mut out = Zeroizing::new(Vec::with_capacity(total as usize + TAG_LEN));
    out.extend_from_slice(key.as_bytes());
    out.extend_from_slice(&(manifest_json.len() as u32).to_le_bytes());
    out.extend_from_slice(&manifest_json);
    out.extend_from_slice(db);
    out.extend_from_slice(config);
    Ok(out)
}

/// Splits a decrypted payload, checking that all lengths add up exactly.
pub(crate) fn parse_plaintext(plain: &[u8]) -> Result<super::DecryptedBackup, BackupError> {
    if plain.len() < KEY_LEN + 4 {
        return Err(BackupError::InvalidHeader);
    }
    let key = DbKey::from_bytes(plain[..KEY_LEN].try_into().expect("32 bytes"));
    let manifest_len =
        u32::from_le_bytes(plain[KEY_LEN..KEY_LEN + 4].try_into().expect("4 bytes")) as usize;
    let manifest_start = KEY_LEN + 4;
    if manifest_len > MAX_MANIFEST_LEN || plain.len() < manifest_start + manifest_len {
        return Err(BackupError::InvalidHeader);
    }
    let manifest: Manifest =
        serde_json::from_slice(&plain[manifest_start..manifest_start + manifest_len])
            .map_err(|_| BackupError::InvalidHeader)?;
    if manifest.format != FORMAT_VERSION {
        return Err(BackupError::UnsupportedVersion(manifest.format));
    }
    let rest = &plain[manifest_start + manifest_len..];
    let expected = manifest
        .db_len
        .checked_add(manifest.config_len)
        .ok_or(BackupError::InvalidHeader)?;
    if expected != rest.len() as u64 {
        return Err(BackupError::InvalidHeader);
    }
    let db_len = manifest.db_len as usize;
    let db = Zeroizing::new(rest[..db_len].to_vec());
    let config = if manifest.config_len == 0 {
        None
    } else {
        Some(Zeroizing::new(rest[db_len..].to_vec()))
    };
    Ok(super::DecryptedBackup {
        db_key: key,
        manifest,
        db,
        config,
    })
}
