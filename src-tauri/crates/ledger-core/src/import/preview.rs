//! Read-only helpers behind the import screen: the raw grid of a file, format
//! sniffing, profile detection and the dry run.
//!
//! Nothing here writes to the database. The dry run runs the real ingest pipeline
//! inside a transaction that is always rolled back, so its counts are exactly what a
//! real import would produce. Row errors carry row numbers and kinds, never cell
//! contents; the file's IBAN is never returned.

use rusqlite::Connection;
use serde::Serialize;

use super::pipeline::{account_with_identity, check_identity};
use super::profile::{CsvEncoding, FileFormat, ImportProfile, StoredProfile};
use super::{parse_file, read_table, IdentityCheck, ImportError, RowError};
use crate::ingest::{ingest_in, IngestOrigin, RowDisposition};
use crate::model::TxSource;

/// Most rows `raw_rows` ever returns.
pub const MAX_RAW_ROWS: usize = 200;
/// Rows listed in the dry run.
pub const MAX_PREVIEW_ROWS: usize = 10;
/// Row errors listed in the dry run (the total is reported separately).
pub const MAX_PREVIEW_ERRORS: usize = 100;
/// Longest cell text returned by `raw_rows`, in characters.
const MAX_CELL_CHARS: usize = 120;
/// Bytes and lines looked at when sniffing a CSV.
const SNIFF_BYTES: usize = 64 * 1024;
const SNIFF_LINES: usize = 30;
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// The first rows of a file as plain text, for the column mapping screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawTable {
    pub rows: Vec<Vec<String>>,
    /// 1-based file row number of each returned row (CSV skips blank lines).
    pub row_numbers: Vec<usize>,
    /// Rows in the whole file.
    pub total_rows: usize,
    /// Widest returned row, to size the column letters A, B, C...
    pub column_count: usize,
}

/// Reads the first `max_rows` (at most [`MAX_RAW_ROWS`]) rows with `format`.
pub fn raw_rows(
    bytes: &[u8],
    file_name: &str,
    format: &FileFormat,
    max_rows: usize,
) -> Result<RawTable, ImportError> {
    let table = read_table(bytes, file_name, format)?;
    let shown = &table[..table.len().min(max_rows.min(MAX_RAW_ROWS))];
    let rows: Vec<Vec<String>> = shown
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.text().chars().take(MAX_CELL_CHARS).collect())
                .collect()
        })
        .collect();
    Ok(RawTable {
        column_count: rows.iter().map(Vec::len).max().unwrap_or(0),
        row_numbers: shown.iter().map(|r| r.number).collect(),
        total_rows: table.len(),
        rows,
    })
}

fn count_unquoted(line: &str, delimiter: char) -> usize {
    let mut quoted = false;
    let mut n = 0;
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if c == delimiter && !quoted {
            n += 1;
        }
    }
    n
}

/// Guesses delimiter (`; , \t |`) and encoding (UTF-8 or Windows-1252) of a CSV to
/// prefill the generic mapping. The quote is always `"`. The delimiter is the one that
/// splits the most lines into the same number of fields; ties go to `;` (the Spanish
/// default), and `;` is also the answer when nothing looks like a table.
pub fn sniff_csv_format(bytes: &[u8]) -> FileFormat {
    let sample = &bytes[..bytes.len().min(SNIFF_BYTES)];
    let sample = sample.strip_prefix(UTF8_BOM).unwrap_or(sample);
    let (text, encoding) = match std::str::from_utf8(sample) {
        Ok(t) => (t.to_string(), CsvEncoding::Utf8),
        // A multi-byte character cut by the sample limit is not an encoding error.
        Err(e) if e.error_len().is_none() => (
            String::from_utf8_lossy(&sample[..e.valid_up_to()]).into_owned(),
            CsvEncoding::Utf8,
        ),
        Err(_) => (
            encoding_rs::WINDOWS_1252.decode(sample).0.into_owned(),
            CsvEncoding::Windows1252,
        ),
    };
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(SNIFF_LINES)
        .collect();

    let mut best = (';', (0usize, 0usize));
    for delimiter in [';', ',', '\t', '|'] {
        let mut freq = std::collections::BTreeMap::<usize, usize>::new();
        for n in lines
            .iter()
            .map(|l| count_unquoted(l, delimiter))
            .filter(|n| *n > 0)
        {
            *freq.entry(n).or_default() += 1;
        }
        // (lines that agree, fields per line) of the most common split.
        let score = freq.iter().map(|(n, f)| (*f, *n)).max().unwrap_or((0, 0));
        if score > best.1 {
            best = (delimiter, score);
        }
    }
    FileFormat::Csv {
        delimiter: best.0,
        encoding,
        quote: '"',
    }
}

/// True when the bytes are valid UTF-8 with non-ASCII text: a Windows-1252 profile
/// would silently turn the accents into mojibake.
fn is_non_ascii_utf8(bytes: &[u8]) -> bool {
    !bytes.is_ascii() && std::str::from_utf8(bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes)).is_ok()
}

/// Name of the stored profile that reads this file, if one clearly does.
///
/// A profile matches when its format fits the extension and content, the header row
/// is found with every column the profile names, and at least one row is valid.
/// Order: built-in Openbank, other built-ins, then user profiles (stored order).
/// The generic CSV built-in is not special: it matches only when its named columns
/// (`Fecha`, `Concepto`, `Importe`) are all in the header, like any profile.
/// Never auto-detected: profiles that name no header column (positional only), which
/// would match any file, and Windows-1252 CSV profiles on files that are valid UTF-8
/// with accents (they would read mojibake).
pub fn detect_profile(bytes: &[u8], file_name: &str, profiles: &[StoredProfile]) -> Option<String> {
    let rank = |p: &StoredProfile| match (p.builtin, p.name.as_str()) {
        (true, "Openbank Excel") => 0,
        (true, _) => 1,
        (false, _) => 2,
    };
    let mut ordered: Vec<&StoredProfile> = profiles.iter().collect();
    ordered.sort_by_key(|p| rank(p));

    ordered
        .into_iter()
        .find(|p| {
            let profile = &p.profile;
            if !profile.has_header || profile.required_header_names().is_empty() {
                return false;
            }
            if matches!(
                profile.format,
                FileFormat::Csv {
                    encoding: CsvEncoding::Windows1252,
                    ..
                }
            ) && is_non_ascii_utf8(bytes)
            {
                return false;
            }
            parse_file(bytes, file_name, profile).is_ok_and(|parsed| !parsed.rows.is_empty())
        })
        .map(|p| p.name.clone())
}

/// One parsed row of the dry run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    /// 1-based row of the file.
    pub row_number: usize,
    pub date: String,
    pub description: String,
    pub amount_cents: i64,
    pub currency: String,
    /// What the import would do with it; only known when an account was chosen.
    pub status: Option<RowDisposition>,
}

/// What the real import would do, from the dry run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCounts {
    pub would_insert: usize,
    /// Rows already stored (identical, or twins that would only receive a reference).
    pub duplicates: usize,
    /// Stored rows that would be rewritten.
    pub updated: usize,
    pub transfers_matched: usize,
    pub categorized: usize,
    /// Rows the pipeline itself refused (normally 0).
    pub rejected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    /// Data rows read: `valid_rows` plus the rejected ones.
    pub total_rows: usize,
    pub valid_rows: usize,
    /// The first [`MAX_PREVIEW_ERRORS`] rejected rows (row number and kind only).
    pub parse_errors: Vec<RowError>,
    pub parse_error_count: usize,
    /// The first [`MAX_PREVIEW_ROWS`] valid rows.
    pub rows: Vec<PreviewRow>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    /// The file's preamble names one account (the IBAN itself is never returned).
    pub has_identity: bool,
    /// The local account that already has the file's identity.
    pub matched_account_id: Option<i64>,
    /// Identity check against the chosen account; `None` when none was chosen.
    pub identity: Option<IdentityCheck>,
    /// Pipeline counts; `None` when no account was chosen or the identity check fails.
    pub counts: Option<ImportCounts>,
}

/// Dry run of `import_file`: parses the file and, when `account_id` is given, runs the
/// real ingest pipeline in a transaction that is rolled back. Writes nothing.
/// A file-level problem (encoding, missing column, ...) is an `Err`.
pub fn preview_import(
    conn: &mut Connection,
    account_id: Option<i64>,
    bytes: &[u8],
    file_name: &str,
    profile: &ImportProfile,
) -> Result<ImportPreview, ImportError> {
    let parsed = parse_file(bytes, file_name, profile)?;
    let identity = parsed.account_identity.as_ref();
    let matched_account_id = match identity {
        Some(i) => account_with_identity(conn, &i.hash)?,
        None => None,
    };
    let check = match account_id {
        Some(id) => Some(check_identity(conn, id, identity)?),
        None => None,
    };

    let mut statuses: Vec<Option<RowDisposition>> = Vec::new();
    let mut counts = None;
    let runnable = matches!(
        check,
        Some(IdentityCheck::NoIdentity | IdentityCheck::Match | IdentityCheck::WillAdopt)
    );
    if let (Some(id), true) = (account_id, runnable) {
        let tx = conn.transaction()?;
        let report = ingest_in(
            &tx,
            id,
            parsed.rows.clone(),
            TxSource::Import,
            IngestOrigin::None,
        )?;
        tx.rollback()?;
        counts = Some(ImportCounts {
            would_insert: report.inserted,
            duplicates: report.duplicates + report.adopted,
            updated: report.updated + report.promoted,
            transfers_matched: report.transfers_matched,
            categorized: report.categorized,
            rejected: report.errors.len(),
        });
        statuses = report.dispositions;
    }

    let rows = parsed
        .rows
        .iter()
        .zip(&parsed.row_numbers)
        .enumerate()
        .take(MAX_PREVIEW_ROWS)
        .map(|(i, (tx, number))| PreviewRow {
            row_number: *number,
            date: tx.booking_date.clone().unwrap_or_default(),
            description: tx.description.clone(),
            amount_cents: tx.amount_cents,
            currency: tx.currency.clone(),
            status: statuses.get(i).copied().flatten(),
        })
        .collect();
    let dates = || parsed.rows.iter().filter_map(|t| t.booking_date.as_deref());

    Ok(ImportPreview {
        total_rows: parsed.rows.len() + parsed.errors.len(),
        valid_rows: parsed.rows.len(),
        date_from: dates().min().map(str::to_string),
        date_to: dates().max().map(str::to_string),
        parse_error_count: parsed.errors.len(),
        parse_errors: parsed
            .errors
            .iter()
            .take(MAX_PREVIEW_ERRORS)
            .cloned()
            .collect(),
        rows,
        has_identity: identity.is_some(),
        matched_account_id,
        identity: check,
        counts,
    })
}
