//! Text normalization shared by dedup, rules and learning.
//!
//! `normalize_text` turns any bank text into a stable form (upper case, no
//! accents, only letters/digits separated by single spaces). `merchant_key`
//! builds on it to get a short "who is this" key by dropping common Spanish
//! bank noise (COMPRA, PAGO, card masks, numbers, dates...).

use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// Leading phrases that describe the operation, not the merchant.
/// Longer phrases first, so "COMPRA TARJ" wins over "COMPRA".
const LEADING_NOISE: [&str; 17] = [
    "COMPRA TARJ",
    "COMPRA EN",
    "PAGO MOVIL",
    "A FAVOR DE",
    "TRANSFERENCIA",
    "COMPRA",
    "PAGO",
    "RECIBO",
    "TRANSF",
    "BIZUM",
    "CARGO",
    "ADEUDO",
    "TARJETA",
    "TARJ",
    "DE",
    "EN",
    "A",
];

/// Minimum length (in characters) for a merchant key to be useful.
const MIN_KEY_CHARS: usize = 3;

/// Unicode NFKD, strip accents, upper case, every non letter/digit becomes a
/// space, spaces collapsed and trimmed. "Cafetería  Luna!" -> "CAFETERIA LUNA".
pub fn normalize_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut gap = false; // a separator was seen since the last kept character
    for ch in input.nfkd().filter(|c| !is_combining_mark(*c)) {
        for up in ch.to_uppercase() {
            if up.is_alphanumeric() {
                if gap && !out.is_empty() {
                    out.push(' ');
                }
                gap = false;
                out.push(up);
            } else {
                gap = true;
            }
        }
    }
    out
}

/// A short key identifying the merchant of a transaction, or `None` when the
/// text holds nothing meaningful. The counterparty wins over the description.
pub fn merchant_key(description: &str, counterparty: Option<&str>) -> Option<String> {
    let key = match counterparty.map(normalize_text) {
        Some(cp) if !cp.is_empty() => cp,
        _ => strip_noise(&normalize_text(description)),
    };
    if key.chars().count() < MIN_KEY_CHARS {
        None
    } else {
        Some(key)
    }
}

/// Removes numbers, card masks and leading operation words from a normalized text.
/// Dates like "12/03" or "120324" are already plain digit tokens after normalizing.
fn strip_noise(normalized: &str) -> String {
    let mut tokens: Vec<&str> = normalized
        .split(' ')
        .filter(|t| !t.is_empty() && !is_digits(t) && !is_card_mask(t))
        .collect();

    // Drop leading operation words, repeatedly ("PAGO MOVIL A FAVOR DE ...").
    'again: loop {
        for phrase in LEADING_NOISE {
            let words: Vec<&str> = phrase.split(' ').collect();
            if tokens.len() >= words.len() && tokens[..words.len()] == words[..] {
                tokens.drain(..words.len());
                continue 'again;
            }
        }
        break;
    }
    tokens.join(" ")
}

fn is_digits(token: &str) -> bool {
    token.chars().all(|c| c.is_ascii_digit())
}

/// "XXXX1234" style masks (the "****1234" form turns into plain digits).
fn is_card_mask(token: &str) -> bool {
    token.len() >= 4
        && token.starts_with("XXXX")
        && token.chars().all(|c| c == 'X' || c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_text_table() {
        let cases = [
            ("Cafetería  Luna!", "CAFETERIA LUNA"),
            ("  supermercados-norte, S.L. ", "SUPERMERCADOS NORTE S L"),
            ("Ñandú Ávila", "NANDU AVILA"),
            ("ＡＢＣ１２３", "ABC123"), // full-width forms folded by NFKD
            ("", ""),
            ("***", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_text(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn merchant_key_table() {
        let cases: [(&str, Option<&str>, Option<&str>); 14] = [
            (
                "COMPRA TARJ. 1234 CAFETERIA LUNA",
                None,
                Some("CAFETERIA LUNA"),
            ),
            (
                "Compra en Supermercados Norte 12/03",
                None,
                Some("SUPERMERCADOS NORTE"),
            ),
            (
                "PAGO MOVIL A FAVOR DE FERRETERIA SOL",
                None,
                Some("FERRETERIA SOL"),
            ),
            (
                "RECIBO GIMNASIO CENTRO 0045 120324",
                None,
                Some("GIMNASIO CENTRO"),
            ),
            ("BIZUM DE Ana Demo", None, Some("ANA DEMO")),
            (
                "TRANSFERENCIA A FAVOR DE Pedro Demo",
                None,
                Some("PEDRO DEMO"),
            ),
            ("COMPRA XXXX1234 LIBRERIA PINO", None, Some("LIBRERIA PINO")),
            ("COMPRA ****1234 LIBRERIA PINO", None, Some("LIBRERIA PINO")),
            (
                "CARGO TARJETA 5555 FARMACIA ROBLE",
                None,
                Some("FARMACIA ROBLE"),
            ),
            ("Compra", None, None),
            ("12/03 4455", None, None),
            ("ab", None, None),
            (
                "COMPRA ALGO",
                Some("Panadería Trigo"),
                Some("PANADERIA TRIGO"),
            ),
            ("COMPRA CAFETERIA LUNA", Some("  "), Some("CAFETERIA LUNA")),
        ];
        for (desc, cp, expected) in cases {
            assert_eq!(
                merchant_key(desc, cp).as_deref(),
                expected,
                "desc {desc:?} cp {cp:?}"
            );
        }
    }
}
