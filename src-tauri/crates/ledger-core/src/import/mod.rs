//! File import: CSV, Excel or HTML-table bytes in, `Vec<NewTransaction>` plus per-row errors out.
//!
//! This module only reads and validates files; it never writes transactions. A later
//! step hands `ParsedImport::rows` to the ingest pipeline. How a file is read is
//! described by an [`ImportProfile`] (stored as JSON in the database), so a new bank
//! layout is a new profile, not new code. Row errors never contain cell contents.

mod convert;
mod dates;
mod history;
mod pipeline;
mod preview;
mod profile;
mod readers;
#[cfg(test)]
mod preview_tests;
#[cfg(test)]
mod tests;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::identity::AccountIdentity;
use crate::model::NewTransaction;

pub use history::{
    list_import_batches, undo_import_batch, ImportBatchRow, UndoReport, MAX_HISTORY_LIMIT,
};
pub use pipeline::{import_file, IdentityCheck, ImportOutcome};
pub use preview::{
    detect_profile, preview_import, raw_rows, sniff_csv_format, ImportCounts,
    ImportPreview, PreviewRow, RawTable, MAX_PREVIEW_ERRORS, MAX_PREVIEW_ROWS, MAX_RAW_ROWS,
};
pub use profile::{
    generic_csv_es, list_profiles, load_profile, openbank_excel, profiles_seed, save_profile,
    seed_builtin_profiles, AmountColumns, ColumnRef, Columns, CsvEncoding, DecimalSpec, FileFormat,
    ImportProfile, StoredProfile,
};

/// Largest file accepted for import (bank exports are a few hundred KB).
pub const MAX_FILE_BYTES: usize = 20 * 1024 * 1024;

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

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
    #[error("file is too large to import")]
    TooLarge,
    /// The file was read but holds no valid row (empty, or every row is invalid).
    #[error("the file has no rows to import")]
    NothingToImport,
    #[error("import batch not found")]
    BatchNotFound,
    #[error("import batch was already undone")]
    BatchAlreadyUndone,
    /// Bank sync adopted or rewrote rows of this batch, so they are no longer only the
    /// file's: undoing would delete data the bank now owns.
    #[error("rows of this batch were adopted by bank sync")]
    BatchHasSyncedRows,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("profile JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// What was wrong with an amount cell. No cell contents, only the category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AmountProblem {
    Empty,
    Malformed,
    /// More than two significant decimals: rejected, never rounded.
    TooManyDecimals,
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
#[serde(rename_all = "camelCase")]
pub struct RowError {
    pub row_number: usize,
    pub reason: RowErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImport {
    pub rows: Vec<NewTransaction>,
    /// 1-based file row number of each entry of `rows` (same length and order).
    pub row_numbers: Vec<usize>,
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
    let table = read_table(bytes, file_name, &profile.format)?;
    let mut parsed = convert::convert(&table, profile)?;
    parsed.file_sha256 = sha256_hex(bytes);
    Ok(parsed)
}

/// Decodes the file into raw rows (no columns, no profile semantics).
fn read_table(
    bytes: &[u8],
    file_name: &str,
    format: &FileFormat,
) -> Result<Vec<readers::RawRow>, ImportError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    if !extension_matches(file_name, format) {
        return Err(ImportError::FormatMismatch);
    }
    match format {
        FileFormat::Csv {
            delimiter,
            encoding,
            quote,
        } => {
            if !delimiter.is_ascii() || !quote.is_ascii() {
                return Err(ImportError::InvalidProfile(
                    "delimiter and quote must be ASCII",
                ));
            }
            readers::read_csv(bytes, *delimiter, *quote, *encoding)
        }
        // Content decides, not the extension: some banks send an HTML table as `.xls`.
        FileFormat::Excel { .. } if readers::looks_like_html(bytes) => {
            readers::read_html_table(bytes)
        }
        FileFormat::Excel { sheet } => readers::read_excel(bytes, sheet.as_deref()),
    }
}
