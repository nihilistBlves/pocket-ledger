//! Tiny date parser for import files (no chrono).
//!
//! A format is a string made of `%d`, `%m`, `%Y` (4 digits), `%y` (2 digits, read as
//! 20yy) and literal characters, e.g. `"%d/%m/%Y"`. Parsed dates are checked against
//! the real calendar (leap years included) and returned as `YYYY-MM-DD`.

/// One piece of a date format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Day,
    Month,
    Year4,
    Year2,
    Literal(char),
}

fn tokenize(format: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = format.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            tokens.push(match chars.next()? {
                'd' => Token::Day,
                'm' => Token::Month,
                'Y' => Token::Year4,
                'y' => Token::Year2,
                _ => return None,
            });
        } else {
            tokens.push(Token::Literal(c));
        }
    }
    // A format without day, month and year can never produce a date.
    let has = |t: Token| tokens.contains(&t);
    if has(Token::Day) && has(Token::Month) && (has(Token::Year4) || has(Token::Year2)) {
        Some(tokens)
    } else {
        None
    }
}

/// True if `format` only uses the supported pieces and has day, month and year.
pub fn is_supported_format(format: &str) -> bool {
    tokenize(format).is_some()
}

pub fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        _ => 28,
    }
}

/// Builds `YYYY-MM-DD` if the date exists in the calendar and the year is sane.
pub fn make_date(year: u32, month: u32, day: u32) -> Option<String> {
    if !(1900..=2100).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Reads between `min` and `max` ASCII digits starting at `*pos`.
fn read_number(chars: &[char], pos: &mut usize, min: usize, max: usize) -> Option<u32> {
    let mut value: u32 = 0;
    let mut count = 0;
    while count < max {
        match chars.get(*pos).and_then(|c| c.to_digit(10)) {
            Some(d) => {
                value = value * 10 + d;
                *pos += 1;
                count += 1;
            }
            None => break,
        }
    }
    if count >= min {
        Some(value)
    } else {
        None
    }
}

/// Parses `text` with one format. Returns `YYYY-MM-DD` or `None`.
pub fn parse_date(text: &str, format: &str) -> Option<String> {
    let tokens = tokenize(format)?;
    let chars: Vec<char> = text.trim().chars().collect();
    let mut pos = 0;
    let (mut day, mut month, mut year) = (0, 0, 0);
    for token in tokens {
        match token {
            Token::Day => day = read_number(&chars, &mut pos, 1, 2)?,
            Token::Month => month = read_number(&chars, &mut pos, 1, 2)?,
            Token::Year4 => year = read_number(&chars, &mut pos, 4, 4)?,
            Token::Year2 => year = 2000 + read_number(&chars, &mut pos, 2, 2)?,
            Token::Literal(expected) => {
                if chars.get(pos) != Some(&expected) {
                    return None;
                }
                pos += 1;
            }
        }
    }
    if pos != chars.len() {
        return None;
    }
    make_date(year, month, day)
}

/// Tries each format in order and returns the first date that parses.
pub fn parse_with_formats(text: &str, formats: &[String]) -> Option<String> {
    formats.iter().find_map(|f| parse_date(text, f))
}
