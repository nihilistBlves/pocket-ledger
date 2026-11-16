//! File import: CSV, Excel or HTML-table bytes in, `Vec<NewTransaction>` plus per-row errors out.
//!
//! This module only reads and validates files; it never writes transactions. A later
//! step hands `ParsedImport::rows` to the ingest pipeline. How a file is read is
//! described by an [`ImportProfile`] (stored as JSON in the database), so a new bank
//! layout is a new profile, not new code. Row errors never contain cell contents.

mod convert;
mod dates;
mod pipeline;
mod profile;
mod readers;
#[cfg(test)]
mod tests;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::identity::AccountIdentity;
use crate::model::NewTransaction;

pub use pipeline::{import_file, ImportOutcome};
pub use profile::{
    generic_csv_es, list_profiles, load_profile, openbank_excel, profiles_seed, save_profile,
    seed_builtin_profiles, AmountColumns, ColumnRef, Columns, CsvEncoding, DecimalSpec, FileFormat,
    ImportProfile, StoredProfile,
};

/// Why a whole file could not be read (as opposed to a single bad row).
#[derive(Debug, Error)]
pub enum ImportError {
    #[error("invalid import profile: {0}")]
    InvalidProfile(&'static str),
    #[error("file extension does not match the profile format")]
    FormatMismatch,
    #[error("file is not valid in the profile's text encoding")]
    Encoding,
    #[error("malformed CSV near line {line:?}")]
    Csv { line: Option<u64> },
    #[error("cannot open the spreadsheet")]
    Workbook,
    #[error("sheet not found")]
    SheetNotFound,
    #[error("header row not found")]
    HeaderNotFound,
    /// Carries the column name from the profile (not from the file).
    #[error("column {0:?} not found in the header")]
    MissingColumn(String),
    /// The file's preamble names an account whose identity is already on another
    /// local account (e.g. one linked by bank sync). Import into that account instead.
    #[error("this file belongs to another account (id {existing_account_id})")]
    AccountBelongsToOther { existing_account_id: i64 },
    /// The file's preamble names a different account than the import target.
    #[error("this file belongs to a different account than the selected one")]
    AccountMismatch,
    /// The import target account does not exist.
    #[error("account not found")]
    AccountNotFound,
    #[error("built-in profiles cannot be overwritten")]
    BuiltinProfile,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("profile JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// What was wrong with an amount cell. No cell contents, only the category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AmountProblem {
    Empty,
    Malformed,
    /// More than two significant decimals: rejected, never rounded.
    TooManyDecimals,
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RowErrorKind {
    MissingValue {
        column: String,
    },
    InvalidDate {
        column: String,
    },
    InvalidAmount {
        column: String,
        problem: AmountProblem,
    },
    /// Debit and credit columns both have a non-zero amount.
    AmbiguousAmount,
}

/// A row that could not be imported. `row_number` is 1-based, as in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RowError {
    pub row_number: usize,
    pub reason: RowErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImport {
    pub rows: Vec<NewTransaction>,
    pub errors: Vec<RowError>,
    /// Lowercase hex SHA-256 of the file bytes, to detect re-imports of the same file.
    pub file_sha256: String,
    /// Identity of the account the file belongs to, when the rows before the header
    /// (the preamble, e.g. Openbank's "Número de cuenta") hold exactly one IBAN.
    pub account_identity: Option<AccountIdentity>,
}

fn extension_matches(file_name: &str, format: &FileFormat) -> bool {
    let lower = file_name.to_lowercase();
    let is_csv = lower.ends_with(".csv") || lower.ends_with(".txt");
    let is_excel = lower.ends_with(".xlsx") || lower.ends_with(".xls");
    match format {
        FileFormat::Csv { .. } => !is_excel,
        FileFormat::Excel { .. } => !is_csv,
    }
}

/// Reads `bytes` according to `profile`.
///
/// `file_name` is only used to catch an obvious mismatch (a `.xlsx` with a CSV
/// profile, or the reverse).
pub fn parse_file(
    bytes: &[u8],
    file_name: &str,
    profile: &ImportProfile,
) -> Result<ParsedImport, ImportError> {
    profile.validate()?;
    if !extension_matches(file_name, &profile.format) {
        return Err(ImportError::FormatMismatch);
    }

    let table = match &profile.format {
        FileFormat::Csv {
            delimiter,
            encoding,
            quote,
        } => readers::read_csv(bytes, *delimiter, *quote, *encoding)?,
        // Content decides, not the extension: some banks send an HTML table as `.xls`.
        FileFormat::Excel { .. } if readers::looks_like_html(bytes) => {
            readers::read_html_table(bytes)?
        }
        FileFormat::Excel { sheet } => readers::read_excel(bytes, sheet.as_deref())?,
    };

    let mut parsed = convert::convert(&table, profile)?;
    parsed.file_sha256 = hex::encode(Sha256::digest(bytes));
    Ok(parsed)
}
