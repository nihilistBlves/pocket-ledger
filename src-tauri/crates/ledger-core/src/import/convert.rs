//! Turns a table of raw rows into transactions using an [`ImportProfile`].
//!
//! Steps: find the header row, resolve column names to positions, then convert
//! every data row. A bad row becomes a [`RowError`] and never stops the import.
//! Row errors carry only the column label and an error kind, never cell contents.

use super::dates;
use super::profile::{AmountColumns, ColumnRef, ImportProfile};
use super::readers::{Cell, RawRow};
use super::{AmountProblem, ImportError, ParsedImport, RowError, RowErrorKind};
use crate::model::{NewTransaction, TxStatus};
use crate::money::{self, MoneyError};

/// How many rows are searched when the header row is detected automatically.
const HEADER_SEARCH_ROWS: usize = 20;

/// Lowercase, strip accents, collapse spaces. Used to compare header names.
pub fn fold(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars().flat_map(char::to_lowercase) {
        out.push(match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        });
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A column position plus the label used in row errors.
struct Col {
    index: usize,
    label: String,
}

enum AmountCols {
    Single(Col),
    DebitCredit { debit: Col, credit: Col },
}

struct Resolved {
    booking_date: Col,
    value_date: Option<Col>,
    description: Col,
    counterparty: Option<Col>,
    remittance: Option<Col>,
    entry_reference: Option<Col>,
    amount: AmountCols,
}

fn resolve(reference: &ColumnRef, header: &[String]) -> Result<Col, ImportError> {
    match reference {
        ColumnRef::Index(i) => Ok(Col {
            index: *i,
            label: format!("#{i}"),
        }),
        ColumnRef::Name(name) => {
            let wanted = fold(name);
            match header.iter().position(|h| *h == wanted) {
                Some(index) => Ok(Col {
                    index,
                    label: name.clone(),
                }),
                None => Err(ImportError::MissingColumn(name.clone())),
            }
        }
    }
}

fn resolve_optional(
    reference: &Option<ColumnRef>,
    header: &[String],
) -> Result<Option<Col>, ImportError> {
    reference.as_ref().map(|r| resolve(r, header)).transpose()
}

fn resolve_all(profile: &ImportProfile, header: &[String]) -> Result<Resolved, ImportError> {
    let c = &profile.columns;
    Ok(Resolved {
        booking_date: resolve(&c.booking_date, header)?,
        value_date: resolve_optional(&c.value_date, header)?,
        description: resolve(&c.description, header)?,
        counterparty: resolve_optional(&c.counterparty, header)?,
        remittance: resolve_optional(&c.remittance, header)?,
        entry_reference: resolve_optional(&c.entry_reference, header)?,
        amount: match &c.amount {
            AmountColumns::Single { column } => AmountCols::Single(resolve(column, header)?),
            AmountColumns::DebitCredit { debit, credit } => AmountCols::DebitCredit {
                debit: resolve(debit, header)?,
                credit: resolve(credit, header)?,
            },
        },
    })
}

fn folded_header(row: &RawRow) -> Vec<String> {
    row.cells.iter().map(|c| fold(c.text())).collect()
}

/// Finds the header row inside the first rows: the first one that contains every
/// required header name.
fn detect_header(rows: &[RawRow], profile: &ImportProfile) -> Result<usize, ImportError> {
    let required: Vec<String> = profile
        .required_header_names()
        .into_iter()
        .map(fold)
        .collect();
    if required.is_empty() {
        return Ok(0); // everything is by position: the first row is the header
    }
    rows.iter()
        .take(HEADER_SEARCH_ROWS)
        .position(|row| {
            let header = folded_header(row);
            required.iter().all(|name| header.contains(name))
        })
        .ok_or(ImportError::HeaderNotFound)
}

fn cell_at(row: &RawRow, col: &Col) -> Cell {
    row.cells.get(col.index).cloned().unwrap_or(Cell::Empty)
}

/// Trim and collapse any run of whitespace into one space.
fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn optional_text(row: &RawRow, col: &Option<Col>) -> Option<String> {
    let col = col.as_ref()?;
    let text = clean_text(cell_at(row, col).text());
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// Reads a date cell. `Ok(None)` means the cell is empty.
fn read_date(row: &RawRow, col: &Col, formats: &[String]) -> Result<Option<String>, RowErrorKind> {
    let invalid = || RowErrorKind::InvalidDate {
        column: col.label.clone(),
    };
    match cell_at(row, col) {
        Cell::Empty => Ok(None),
        Cell::Date(date) => Ok(Some(date)),
        Cell::Text(text) | Cell::Number(text) => {
            if text.trim().is_empty() {
                Ok(None)
            } else {
                dates::parse_with_formats(&text, formats)
                    .map(Some)
                    .ok_or_else(invalid)
            }
        }
        Cell::Unsupported => Err(invalid()),
    }
}

fn problem_of(error: &MoneyError) -> AmountProblem {
    match error {
        MoneyError::Empty | MoneyError::OnlySign => AmountProblem::Empty,
        MoneyError::TooManyDecimals => AmountProblem::TooManyDecimals,
        MoneyError::Overflow => AmountProblem::Overflow,
        _ => AmountProblem::Malformed,
    }
}

/// Reads one amount cell into signed cents. `Ok(None)` means the cell is empty.
fn read_amount(
    row: &RawRow,
    col: &Col,
    profile: &ImportProfile,
) -> Result<Option<i64>, RowErrorKind> {
    let fail = |problem| RowErrorKind::InvalidAmount {
        column: col.label.clone(),
        problem,
    };
    match cell_at(row, col) {
        Cell::Empty => Ok(None),
        Cell::Text(text) => {
            if text.trim().is_empty() {
                return Ok(None);
            }
            money::parse_amount(&text, profile.decimal.style())
                .map(Some)
                .map_err(|e| fail(problem_of(&e)))
        }
        // Excel numbers always use '.' no matter the profile's decimal style.
        Cell::Number(text) => money::parse_amount(&text, money::DOT)
            .map(Some)
            .map_err(|e| fail(problem_of(&e))),
        Cell::Date(_) | Cell::Unsupported => Err(fail(AmountProblem::Malformed)),
    }
}

fn read_signed_amount(
    row: &RawRow,
    amount: &AmountCols,
    profile: &ImportProfile,
) -> Result<i64, RowErrorKind> {
    match amount {
        AmountCols::Single(col) => {
            read_amount(row, col, profile)?.ok_or_else(|| RowErrorKind::InvalidAmount {
                column: col.label.clone(),
                problem: AmountProblem::Empty,
            })
        }
        AmountCols::DebitCredit { debit, credit } => {
            let debit_value = read_amount(row, debit, profile)?.filter(|v| *v != 0);
            let credit_value = read_amount(row, credit, profile)?.filter(|v| *v != 0);
            let overflow = |col: &Col| RowErrorKind::InvalidAmount {
                column: col.label.clone(),
                problem: AmountProblem::Overflow,
            };
            match (debit_value, credit_value) {
                (Some(_), Some(_)) => Err(RowErrorKind::AmbiguousAmount),
                // Debit is always an outflow: take the magnitude, then negate.
                (Some(d), None) => d
                    .checked_abs()
                    .and_then(i64::checked_neg)
                    .ok_or_else(|| overflow(debit)),
                (None, Some(c)) => Ok(c),
                // Both empty or zero: a genuine 0.00 row is valid, a blank pair is not.
                (None, None) => {
                    let both_empty = matches!(cell_at(row, debit), Cell::Empty)
                        && matches!(cell_at(row, credit), Cell::Empty);
                    if both_empty {
                        Err(RowErrorKind::InvalidAmount {
                            column: debit.label.clone(),
                            problem: AmountProblem::Empty,
                        })
                    } else {
                        Ok(0)
                    }
                }
            }
        }
    }
}

fn convert_row(
    row: &RawRow,
    cols: &Resolved,
    profile: &ImportProfile,
) -> Result<NewTransaction, RowErrorKind> {
    let booking_date =
        read_date(row, &cols.booking_date, &profile.date_formats)?.ok_or_else(|| {
            RowErrorKind::MissingValue {
                column: cols.booking_date.label.clone(),
            }
        })?;
    let value_date = match &cols.value_date {
        Some(col) => read_date(row, col, &profile.date_formats)?,
        None => None,
    };
    let amount_cents = read_signed_amount(row, &cols.amount, profile)?;
    let description = clean_text(cell_at(row, &cols.description).text());
    if description.is_empty() {
        return Err(RowErrorKind::MissingValue {
            column: cols.description.label.clone(),
        });
    }
    Ok(NewTransaction {
        status: TxStatus::Booked,
        booking_date: Some(booking_date),
        value_date,
        amount_cents,
        currency: profile.currency.clone(),
        description,
        counterparty: optional_text(row, &cols.counterparty),
        remittance: optional_text(row, &cols.remittance),
        entry_reference: optional_text(row, &cols.entry_reference),
    })
}

/// Converts a whole table. `file_sha256` is filled in by the caller.
pub fn convert(rows: &[RawRow], profile: &ImportProfile) -> Result<ParsedImport, ImportError> {
    let (header, data_start): (Vec<String>, usize) = if profile.has_header {
        let header_index = match profile.skip_rows {
            Some(n) => n,
            None => detect_header(rows, profile)?,
        };
        let header_row = rows.get(header_index).ok_or(ImportError::HeaderNotFound)?;
        (folded_header(header_row), header_index + 1)
    } else {
        (Vec::new(), profile.skip_rows.unwrap_or(0))
    };

    let cols = resolve_all(profile, &header)?;

    let mut parsed = ParsedImport {
        rows: Vec::new(),
        errors: Vec::new(),
        file_sha256: String::new(),
    };
    for row in rows.iter().skip(data_start) {
        if row.is_blank() {
            continue;
        }
        match convert_row(row, &cols, profile) {
            Ok(tx) => parsed.rows.push(tx),
            Err(reason) => parsed.errors.push(RowError {
                row_number: row.number,
                reason,
            }),
        }
    }
    Ok(parsed)
}
