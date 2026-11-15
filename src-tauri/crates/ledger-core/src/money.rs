//! Exact money handling: amounts are signed `i64` cents (EUR), never floats.
//!
//! `parse_amount` turns text such as `"1.234,56"` or `"-12.30"` into cents using
//! only integer arithmetic with overflow checks. It is strict on purpose: anything
//! ambiguous or malformed is an error rather than a guess, because a silently
//! wrong amount is worse than a rejected row.
//!
//! Sign convention: negative = outflow, positive = inflow.

use thiserror::Error;

/// How decimal numbers are written in a given source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecimalStyle {
    /// The character that separates euros from cents.
    pub decimal: char,
    /// The grouping character for thousands, if the source uses one.
    /// When set, NBSP (U+00A0), thin space (U+2009) and narrow NBSP (U+202F)
    /// are also accepted as grouping characters.
    pub thousands: Option<char>,
}

/// API style: `1234.56`, no grouping.
pub const DOT: DecimalStyle = DecimalStyle {
    decimal: '.',
    thousands: None,
};

/// Spanish style: `1.234,56`.
pub const ES: DecimalStyle = DecimalStyle {
    decimal: ',',
    thousands: Some('.'),
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("empty amount")]
    Empty,
    #[error("amount has a sign or currency but no digits")]
    OnlySign,
    #[error("more than one sign")]
    MultipleSigns,
    #[error("more than one currency marker")]
    MultipleCurrencies,
    #[error("more than one decimal separator")]
    MultipleDecimalSeparators,
    #[error("missing digits before the decimal separator")]
    EmptyIntegerPart,
    #[error("decimal separator without digits after it")]
    EmptyFraction,
    #[error("thousands separator in the fractional part")]
    ThousandsInFraction,
    #[error("misplaced thousands grouping")]
    MisplacedGrouping,
    #[error("more than two significant decimal digits")]
    TooManyDecimals,
    #[error("invalid character {0:?}")]
    InvalidCharacter(char),
    #[error("amount does not fit in i64 cents")]
    Overflow,
    #[error("decimal and thousands separators must differ")]
    InvalidStyle,
    #[error("unknown credit/debit indicator")]
    InvalidIndicator,
    #[error("unsigned amount must not be negative")]
    NegativeInput,
}

/// Parses a decimal amount into signed cents.
///
/// Accepted: optional sign (`+`, `-`, U+2212) before or after the number,
/// surrounding whitespace, one `€` or `EUR` marker before or after, 0 to 2
/// fractional digits (more only if the extra digits are all zero), and
/// grouping of exactly three digits when `style.thousands` is set.
/// An integer part is required (`",5"` is rejected, `"0,5"` is fine).
pub fn parse_amount(input: &str, style: DecimalStyle) -> Result<i64, MoneyError> {
    if style.thousands == Some(style.decimal) {
        return Err(MoneyError::InvalidStyle);
    }

    let peeled = peel_decorations(input)?;
    let body = peeled.body;
    if body.is_empty() {
        return Err(if peeled.decorated {
            MoneyError::OnlySign
        } else {
            MoneyError::Empty
        });
    }

    let mut parts = body.split(style.decimal);
    let int_part = parts.next().unwrap_or("");
    let frac_part = parts.next();
    if parts.next().is_some() {
        return Err(MoneyError::MultipleDecimalSeparators);
    }

    let int_value = parse_integer_part(int_part, style)?;
    let frac_cents = match frac_part {
        Some(frac) => parse_fraction(frac, style)?,
        None => 0,
    };

    let magnitude = int_value
        .checked_mul(100)
        .and_then(|v| v.checked_add(frac_cents))
        .ok_or(MoneyError::Overflow)?;

    to_signed(magnitude, peeled.negative)
}

/// Applies the sign to an unsigned magnitude, allowing exactly `i64::MIN`.
fn to_signed(magnitude: u64, negative: bool) -> Result<i64, MoneyError> {
    if negative {
        if magnitude == i64::MIN.unsigned_abs() {
            Ok(i64::MIN)
        } else {
            let positive = i64::try_from(magnitude).map_err(|_| MoneyError::Overflow)?;
            Ok(-positive)
        }
    } else {
        i64::try_from(magnitude).map_err(|_| MoneyError::Overflow)
    }
}

struct Peeled<'a> {
    negative: bool,
    body: &'a str,
    /// True if a sign or currency marker was present.
    decorated: bool,
}

/// Strips whitespace, one sign and one currency marker from both ends.
fn peel_decorations(input: &str) -> Result<Peeled<'_>, MoneyError> {
    let mut s = input.trim();
    let mut sign: Option<bool> = None; // Some(true) = negative
    let mut currency = false;

    loop {
        s = s.trim();

        if let Some(c) = s.chars().next() {
            if let Some(negative) = sign_value(c) {
                if sign.is_some() {
                    return Err(MoneyError::MultipleSigns);
                }
                sign = Some(negative);
                s = &s[c.len_utf8()..];
                continue;
            }
        }
        if let Some(c) = s.chars().next_back() {
            if let Some(negative) = sign_value(c) {
                if sign.is_some() {
                    return Err(MoneyError::MultipleSigns);
                }
                sign = Some(negative);
                s = &s[..s.len() - c.len_utf8()];
                continue;
            }
        }
        let stripped = strip_currency_prefix(s).or_else(|| strip_currency_suffix(s));
        if let Some(rest) = stripped {
            if currency {
                return Err(MoneyError::MultipleCurrencies);
            }
            currency = true;
            s = rest;
            continue;
        }
        break;
    }

    Ok(Peeled {
        negative: sign == Some(true),
        body: s,
        decorated: sign.is_some() || currency,
    })
}

/// Returns `Some(true)` for minus signs, `Some(false)` for plus, else `None`.
fn sign_value(c: char) -> Option<bool> {
    match c {
        '+' => Some(false),
        '-' | '\u{2212}' => Some(true),
        _ => None,
    }
}

fn strip_currency_prefix(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_prefix('€') {
        return Some(rest);
    }
    if s.len() >= 3 && s.is_char_boundary(3) && s[..3].eq_ignore_ascii_case("EUR") {
        return Some(&s[3..]);
    }
    None
}

fn strip_currency_suffix(s: &str) -> Option<&str> {
    if let Some(rest) = s.strip_suffix('€') {
        return Some(rest);
    }
    let n = s.len();
    if n >= 3 && s.is_char_boundary(n - 3) && s[n - 3..].eq_ignore_ascii_case("EUR") {
        return Some(&s[..n - 3]);
    }
    None
}

fn is_thousands_char(c: char, thousands: char) -> bool {
    c == thousands || matches!(c, '\u{00A0}' | '\u{2009}' | '\u{202F}')
}

/// Parses the part before the decimal separator into whole euros.
fn parse_integer_part(int_part: &str, style: DecimalStyle) -> Result<u64, MoneyError> {
    if int_part.is_empty() {
        return Err(MoneyError::EmptyIntegerPart);
    }

    let mut separator: Option<char> = None;
    for c in int_part.chars() {
        if c.is_ascii_digit() {
            continue;
        }
        match style.thousands {
            Some(t) if is_thousands_char(c, t) => match separator {
                None => separator = Some(c),
                // Mixing different grouping characters is ambiguous.
                Some(first) if first != c => return Err(MoneyError::MisplacedGrouping),
                Some(_) => {}
            },
            _ => return Err(MoneyError::InvalidCharacter(c)),
        }
    }

    if let Some(sep) = separator {
        // First group 1..=3 digits, every following group exactly 3.
        for (index, group) in int_part.split(sep).enumerate() {
            let ok = if index == 0 {
                (1..=3).contains(&group.len())
            } else {
                group.len() == 3
            };
            if !ok {
                return Err(MoneyError::MisplacedGrouping);
            }
        }
    }

    let mut value: u64 = 0;
    for c in int_part.chars().filter(char::is_ascii_digit) {
        value = push_digit(value, c)?;
    }
    Ok(value)
}

/// Parses the digits after the decimal separator into cents (0..=99).
fn parse_fraction(frac: &str, style: DecimalStyle) -> Result<u64, MoneyError> {
    if frac.is_empty() {
        return Err(MoneyError::EmptyFraction);
    }
    for c in frac.chars() {
        if c.is_ascii_digit() {
            continue;
        }
        return match style.thousands {
            Some(t) if is_thousands_char(c, t) => Err(MoneyError::ThousandsInFraction),
            _ => Err(MoneyError::InvalidCharacter(c)),
        };
    }

    let digits = frac.as_bytes();
    // Digits beyond the second must all be zero: "1.230" is 1.23, "1.235" is an error.
    if digits.iter().skip(2).any(|&d| d != b'0') {
        return Err(MoneyError::TooManyDecimals);
    }
    let tens = u64::from(digits[0] - b'0');
    let units = digits.get(1).map_or(0, |&d| u64::from(d - b'0'));
    Ok(tens * 10 + units)
}

/// `value * 10 + digit` with overflow check. `c` must be an ASCII digit.
fn push_digit(value: u64, c: char) -> Result<u64, MoneyError> {
    let digit = c.to_digit(10).ok_or(MoneyError::InvalidCharacter(c))?;
    value
        .checked_mul(10)
        .and_then(|v| v.checked_add(u64::from(digit)))
        .ok_or(MoneyError::Overflow)
}

/// Applies an ISO 20022 credit/debit indicator to an unsigned amount.
/// `"CRDT"` keeps it positive, `"DBIT"` makes it negative.
pub fn signed_from_indicator(cents_unsigned: i64, indicator: &str) -> Result<i64, MoneyError> {
    if cents_unsigned < 0 {
        return Err(MoneyError::NegativeInput);
    }
    match indicator {
        "CRDT" => Ok(cents_unsigned),
        "DBIT" => Ok(-cents_unsigned), // cannot overflow: input is >= 0
        _ => Err(MoneyError::InvalidIndicator),
    }
}

/// Formats cents as Spanish text, e.g. `-123456` -> `"-1.234,56"`.
/// For tests and debugging only; the UI formats through i18n.
pub fn format_cents_es(cents: i64) -> String {
    let abs = cents.unsigned_abs(); // safe for i64::MIN
    let euros = (abs / 100).to_string();
    let rest = abs % 100;

    let mut grouped = String::new();
    let len = euros.len();
    for (i, c) in euros.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(c);
    }

    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}{grouped},{rest:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str, style: DecimalStyle) -> i64 {
        parse_amount(s, style).unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
    }

    fn err(s: &str, style: DecimalStyle) -> MoneyError {
        match parse_amount(s, style) {
            Ok(v) => panic!("{s:?} should fail but gave {v}"),
            Err(e) => e,
        }
    }

    #[test]
    fn dot_style_valid_table() {
        let cases: &[(&str, i64)] = &[
            ("0", 0),
            ("0.00", 0),
            ("12", 1200),
            ("12.3", 1230),
            ("12.34", 1234),
            ("1234.56", 123_456),
            ("0.5", 50),
            ("0.05", 5),
            ("+5", 500),
            ("-5", -500),
            ("-5.00", -500),
            ("+0.01", 1),
            ("-0.01", -1),
            ("5-", -500),
            ("5.25-", -525),
            ("5+", 500),
            ("\u{2212}7.10", -710),
            ("7.10\u{2212}", -710),
            ("  42.50  ", 4250),
            ("\t-42.50\n", -4250),
            ("€12.30", 1230),
            ("€ 12.30", 1230),
            ("12.30€", 1230),
            ("12.30 €", 1230),
            ("12.30 EUR", 1230),
            ("EUR 12.30", 1230),
            ("eur12.30", 1230),
            ("-€12.30", -1230),
            ("€ -12.30", -1230),
            ("- 12.30", -1230),
            ("12.30- EUR", -1230),
            ("1.230", 123),
            ("1.2300", 123),
            ("1.000", 100),
            ("0.000", 0),
            ("007", 700),
            ("00.10", 10),
            ("-0", 0),
            ("-0.00", 0),
        ];
        for (input, expected) in cases {
            assert_eq!(ok(input, DOT), *expected, "input {input:?}");
        }
    }

    #[test]
    fn es_style_valid_table() {
        let cases: &[(&str, i64)] = &[
            ("1.234,56", 123_456),
            ("-1.234,56", -123_456),
            ("1.234.567,89", 123_456_789),
            ("1234,56", 123_456),
            ("1234567,89", 123_456_789),
            ("1,5", 150),
            ("1,50", 150),
            ("1,500", 150),
            ("12", 1200),
            ("1.234", 123_400),
            ("999", 99_900),
            ("999.999", 99_999_900),
            ("0,99", 99),
            ("1\u{00A0}234,56", 123_456),
            ("1\u{2009}234,56", 123_456),
            ("1\u{202F}234,56", 123_456),
            ("1.234,56-", -123_456),
            ("1.234,56 €", 123_456),
            ("€ 1.234,56", 123_456),
            ("-1.234,56 EUR", -123_456),
        ];
        for (input, expected) in cases {
            assert_eq!(ok(input, ES), *expected, "input {input:?}");
        }
    }

    #[test]
    fn custom_style_comma_thousands_dot_decimal() {
        let style = DecimalStyle {
            decimal: '.',
            thousands: Some(','),
        };
        assert_eq!(ok("1,234.56", style), 123_456);
        assert_eq!(ok("-12,345,678.90", style), -1_234_567_890);
        assert_eq!(err("1,23.45", style), MoneyError::MisplacedGrouping);
    }

    #[test]
    fn invalid_table() {
        let cases: &[(&str, DecimalStyle, MoneyError)] = &[
            ("", DOT, MoneyError::Empty),
            ("   ", DOT, MoneyError::Empty),
            ("-", DOT, MoneyError::OnlySign),
            ("+", DOT, MoneyError::OnlySign),
            ("\u{2212}", DOT, MoneyError::OnlySign),
            ("€", DOT, MoneyError::OnlySign),
            ("EUR", DOT, MoneyError::OnlySign),
            ("- €", DOT, MoneyError::OnlySign),
            ("--5", DOT, MoneyError::MultipleSigns),
            ("-5-", DOT, MoneyError::MultipleSigns),
            ("+-5", DOT, MoneyError::MultipleSigns),
            ("€5€", DOT, MoneyError::MultipleCurrencies),
            ("EUR 5 €", DOT, MoneyError::MultipleCurrencies),
            ("1.2.3", DOT, MoneyError::MultipleDecimalSeparators),
            ("1,2,3", ES, MoneyError::MultipleDecimalSeparators),
            ("1.234,5,6", ES, MoneyError::MultipleDecimalSeparators),
            (".5", DOT, MoneyError::EmptyIntegerPart),
            (",5", ES, MoneyError::EmptyIntegerPart),
            ("5.", DOT, MoneyError::EmptyFraction),
            ("5,", ES, MoneyError::EmptyFraction),
            ("1,23.4", ES, MoneyError::ThousandsInFraction),
            ("1,23\u{00A0}4", ES, MoneyError::ThousandsInFraction),
            ("1.235", DOT, MoneyError::TooManyDecimals),
            ("1.2301", DOT, MoneyError::TooManyDecimals),
            ("1,001", ES, MoneyError::TooManyDecimals),
            ("1,234,56", DOT, MoneyError::InvalidCharacter(',')),
            ("1 000", DOT, MoneyError::InvalidCharacter(' ')),
            (
                "1\u{00A0}000",
                DOT,
                MoneyError::InvalidCharacter('\u{00A0}'),
            ),
            ("abc", DOT, MoneyError::InvalidCharacter('a')),
            ("12a", DOT, MoneyError::InvalidCharacter('a')),
            ("1e5", DOT, MoneyError::InvalidCharacter('e')),
            ("1E5", DOT, MoneyError::InvalidCharacter('E')),
            ("NaN", DOT, MoneyError::InvalidCharacter('N')),
            ("inf", DOT, MoneyError::InvalidCharacter('i')),
            ("-Infinity", DOT, MoneyError::InvalidCharacter('I')),
            ("1.5.0,0", DOT, MoneyError::MultipleDecimalSeparators),
            (
                "\u{0661}\u{0662}",
                DOT,
                MoneyError::InvalidCharacter('\u{0661}'),
            ),
            ("1.2.3,4", ES, MoneyError::MisplacedGrouping),
            ("1.23,4", ES, MoneyError::MisplacedGrouping),
            ("12.34.5", ES, MoneyError::MisplacedGrouping),
            ("1234.567", ES, MoneyError::MisplacedGrouping),
            ("1234.56", ES, MoneyError::MisplacedGrouping),
            ("1.2345,00", ES, MoneyError::MisplacedGrouping),
            (".234,56", ES, MoneyError::MisplacedGrouping),
            ("1..234", ES, MoneyError::MisplacedGrouping),
            ("1.234.,56", ES, MoneyError::MisplacedGrouping),
            ("1.234 567,89", ES, MoneyError::InvalidCharacter(' ')),
            ("1.234\u{00A0}567,89", ES, MoneyError::MisplacedGrouping),
        ];
        for (input, style, expected) in cases {
            assert_eq!(err(input, *style), *expected, "input {input:?}");
        }
    }

    #[test]
    fn invalid_style_is_rejected() {
        let style = DecimalStyle {
            decimal: '.',
            thousands: Some('.'),
        };
        assert_eq!(err("1.5", style), MoneyError::InvalidStyle);
    }

    #[test]
    fn i64_limits() {
        assert_eq!(ok("92233720368547758.07", DOT), i64::MAX);
        assert_eq!(ok("-92233720368547758.07", DOT), -i64::MAX);
        assert_eq!(ok("-92233720368547758.08", DOT), i64::MIN);
        assert_eq!(ok("92233720368547758.07-", DOT), -i64::MAX);

        assert_eq!(err("92233720368547758.08", DOT), MoneyError::Overflow);
        assert_eq!(err("-92233720368547758.09", DOT), MoneyError::Overflow);
        assert_eq!(err("92233720368547759", DOT), MoneyError::Overflow);
        // Integer part itself overflows u64.
        assert_eq!(err("99999999999999999999", DOT), MoneyError::Overflow);
        assert_eq!(
            err("1000000000000000000000000000000.00", DOT),
            MoneyError::Overflow
        );
        // Times 100 overflows u64.
        assert_eq!(err("184467440737095517", DOT), MoneyError::Overflow);
        assert_eq!(err("-184467440737095517", DOT), MoneyError::Overflow);

        assert_eq!(ok("92.233.720.368.547.758,07", ES), i64::MAX);
        assert_eq!(ok("-92.233.720.368.547.758,08", ES), i64::MIN);
        assert_eq!(err("92.233.720.368.547.758,08", ES), MoneyError::Overflow);
    }

    #[test]
    fn indicator_table() {
        assert_eq!(signed_from_indicator(1234, "CRDT"), Ok(1234));
        assert_eq!(signed_from_indicator(1234, "DBIT"), Ok(-1234));
        assert_eq!(signed_from_indicator(0, "DBIT"), Ok(0));
        assert_eq!(signed_from_indicator(0, "CRDT"), Ok(0));
        assert_eq!(signed_from_indicator(i64::MAX, "DBIT"), Ok(-i64::MAX));
        assert_eq!(signed_from_indicator(i64::MAX, "CRDT"), Ok(i64::MAX));

        for bad in [
            "", "crdt", "dbit", "CRDT ", " DBIT", "CR", "DEBIT", "+", "-",
        ] {
            assert_eq!(
                signed_from_indicator(100, bad),
                Err(MoneyError::InvalidIndicator),
                "indicator {bad:?}"
            );
        }
        assert_eq!(
            signed_from_indicator(-1, "CRDT"),
            Err(MoneyError::NegativeInput)
        );
        assert_eq!(
            signed_from_indicator(i64::MIN, "DBIT"),
            Err(MoneyError::NegativeInput)
        );
        // Negative input is reported even when the indicator is also bad.
        assert_eq!(
            signed_from_indicator(-1, "XXXX"),
            Err(MoneyError::NegativeInput)
        );
    }

    #[test]
    fn format_table() {
        let cases: &[(i64, &str)] = &[
            (0, "0,00"),
            (1, "0,01"),
            (-1, "-0,01"),
            (99, "0,99"),
            (100, "1,00"),
            (123_456, "1.234,56"),
            (-123_456, "-1.234,56"),
            (99_999, "999,99"),
            (100_000, "1.000,00"),
            (123_456_789, "1.234.567,89"),
            (i64::MAX, "92.233.720.368.547.758,07"),
            (i64::MIN, "-92.233.720.368.547.758,08"),
        ];
        for (cents, expected) in cases {
            assert_eq!(format_cents_es(*cents), *expected);
        }
    }

    #[test]
    fn round_trip_es_dense_range() {
        for cents in -200_000_i64..=200_000 {
            let text = format_cents_es(cents);
            assert_eq!(ok(&text, ES), cents, "text {text:?}");
        }
    }

    #[test]
    fn round_trip_es_edges_and_pseudo_random() {
        let mut values: Vec<i64> = vec![
            i64::MAX,
            i64::MAX - 1,
            i64::MAX - 99,
            i64::MAX - 100,
            i64::MIN,
            i64::MIN + 1,
            i64::MIN + 99,
            i64::MIN + 100,
            0,
            1,
            -1,
        ];
        // Powers of ten and their neighbours exercise every grouping boundary.
        let mut p: i64 = 1;
        for _ in 0..18 {
            for delta in [-1_i64, 0, 1] {
                values.push(p + delta);
                values.push(-(p + delta));
            }
            p *= 10;
        }
        // Deterministic LCG (integer only) spreading values over all magnitudes.
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for i in 0..20_000_u32 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let shift = i % 64;
            let raw = (state >> shift) as i64; // top bit cleared by the shift unless shift == 0
            values.push(raw);
            values.push(raw.wrapping_neg());
        }
        for cents in values {
            let text = format_cents_es(cents);
            assert_eq!(ok(&text, ES), cents, "text {text:?}");
        }
    }

    #[test]
    fn round_trip_dot_plain() {
        for cents in (-100_000_i64..=100_000).step_by(7) {
            let sign = if cents < 0 { "-" } else { "" };
            let abs = cents.unsigned_abs();
            let text = format!("{sign}{}.{:02}", abs / 100, abs % 100);
            assert_eq!(ok(&text, DOT), cents, "text {text:?}");
        }
        for cents in [i64::MAX, i64::MIN, i64::MIN + 1] {
            let sign = if cents < 0 { "-" } else { "" };
            let abs = cents.unsigned_abs();
            let text = format!("{sign}{}.{:02}", abs / 100, abs % 100);
            assert_eq!(ok(&text, DOT), cents, "text {text:?}");
        }
    }

    #[test]
    fn non_ascii_does_not_panic() {
        // Multi-byte characters near the 3-byte currency checks must not panic.
        for s in [
            "€€",
            "ñ",
            "ñ€",
            "€ñ",
            "12ñ",
            "éé",
            "€€€",
            "EU€",
            "ñ12",
            "1€2",
        ] {
            assert!(parse_amount(s, DOT).is_err(), "input {s:?}");
        }
    }
}
