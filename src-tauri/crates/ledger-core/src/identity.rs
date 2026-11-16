//! Stable account identity shared by bank sync and file import.
//!
//! An account is identified by the SHA-256 (hex) of its normalized IBAN, stored in
//! `accounts.identity_hash`, so a bank-synced account and an imported one with the
//! same IBAN converge on the same row. The IBAN itself is never stored; only its
//! last 4 characters are kept (`accounts.display_suffix`) for display.

use sha2::{Digest, Sha256};

/// Identity of a bank account. `Debug` shows only the hash and the suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub hash: String,
    pub suffix: Option<String>,
}

impl AccountIdentity {
    /// `iban` may contain spaces and lower case. No checksum is verified here: the
    /// value comes from the bank, and the hash only has to be stable.
    pub fn from_iban(iban: &str) -> Option<Self> {
        let normalized = normalize_iban(iban);
        if normalized.is_empty() {
            return None;
        }
        let suffix: String =
            normalized.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        Some(Self {
            hash: hex::encode(Sha256::digest(normalized.as_bytes())),
            suffix: Some(suffix),
        })
    }

    /// Fallback when the bank gives no IBAN, only its opaque identification hash.
    /// Prefixed so it can never collide with an IBAN-derived hash.
    pub fn from_identification_hash(value: &str) -> Option<Self> {
        let value = value.trim();
        (!value.is_empty()).then(|| Self {
            hash: hex::encode(Sha256::digest(format!("eb-identification:{value}").as_bytes())),
            suffix: None,
        })
    }
}

/// Removes whitespace and upper-cases.
pub fn normalize_iban(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_uppercase()).collect()
}

/// IBAN shape after normalization: two letters, two digits, then 11 to 30
/// alphanumerics (15 to 34 characters in total). The check digits are not verified,
/// so test data like `ES00 0000 ...` is accepted.
pub fn is_iban_shaped(normalized: &str) -> bool {
    let b = normalized.as_bytes();
    (15..=34).contains(&b.len())
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..4].iter().all(u8::is_ascii_digit)
        && b[4..].iter().all(u8::is_ascii_alphanumeric)
}

/// The IBAN in a free-text cell such as `"ES00 0000 0000 0000 0000 0000"`, if the
/// whole cell (ignoring spaces) is IBAN-shaped.
pub fn iban_in_cell(text: &str) -> Option<String> {
    let normalized = normalize_iban(text);
    is_iban_shaped(&normalized).then_some(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_normalizes_the_iban() {
        let a = AccountIdentity::from_iban(" es00 0000 0000 0000 0000 0001 ").unwrap();
        let b = AccountIdentity::from_iban("ES0000000000000000000001").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.suffix.as_deref(), Some("0001"));
        assert!(AccountIdentity::from_iban("  ").is_none());
    }

    #[test]
    fn identification_hash_never_collides_with_an_iban() {
        let iban = AccountIdentity::from_iban("ES0000000000000000000001").unwrap();
        let other = AccountIdentity::from_identification_hash("ES0000000000000000000001").unwrap();
        assert_ne!(iban.hash, other.hash);
        assert_eq!(other.suffix, None);
    }

    #[test]
    fn iban_shape() {
        assert_eq!(
            iban_in_cell("ES00 0000 0000 0000 0000 0000").as_deref(),
            Some("ES0000000000000000000000")
        );
        assert_eq!(iban_in_cell("de99 1234 5678 9012 3456 78").as_deref(), Some("DE99123456789012345678"));
        for not_iban in ["Cuenta demo", "1.000,00", "15/11/2026", "ES00 1234", "0000 0000 0000 0000 0000", ""] {
            assert_eq!(iban_in_cell(not_iban), None, "{not_iban}");
        }
    }
}
