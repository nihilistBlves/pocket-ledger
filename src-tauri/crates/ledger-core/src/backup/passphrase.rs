//! Passphrase strength gate for backups.

use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

/// Minimum length in characters (after NFC normalization).
pub const MIN_CHARS: usize = 12;
/// Minimum zxcvbn score (0..=4).
pub const MIN_SCORE: u8 = 3;
/// zxcvbn gets slow on very long inputs; the first characters are plenty to score.
const SCORED_CHARS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassphraseVerdict {
    pub ok: bool,
    pub score: u8,
    /// Short advice in Spanish, `None` when the passphrase is accepted.
    pub feedback: Option<&'static str>,
}

/// Accepts a passphrase with at least 12 characters and zxcvbn score >= 3.
pub fn check(passphrase: &str) -> PassphraseVerdict {
    let normalized: Zeroizing<String> = Zeroizing::new(passphrase.nfc().collect());
    let chars = normalized.chars().count();
    let scored: Zeroizing<String> = Zeroizing::new(normalized.chars().take(SCORED_CHARS).collect());
    let score: u8 = zxcvbn::zxcvbn(&scored, &["pocket", "ledger"])
        .score()
        .into();
    let feedback = if chars < MIN_CHARS {
        Some("Usa al menos 12 caracteres.")
    } else if score < MIN_SCORE {
        Some("Es fácil de adivinar: prueba con 4 o 5 palabras al azar.")
    } else {
        None
    };
    PassphraseVerdict {
        ok: feedback.is_none(),
        score,
        feedback,
    }
}
