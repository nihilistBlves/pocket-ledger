//! File readers: turn CSV or Excel bytes into a plain table of [`RawRow`]s.
//!
//! Readers know nothing about columns or profiles' meaning; they only decode the
//! file. Money is never converted here: Excel numbers are kept as text (see
//! [`excel_number_text`]) and parsed later with `money::parse_amount`.

use std::io::Cursor;

use calamine::{open_workbook_auto_from_rs, Data, Reader};

use super::profile::CsvEncoding;
use super::ImportError;

/// One cell, after decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    Empty,
    /// Text exactly as in the file.
    Text(String),
    /// A number cell as its shortest exact decimal text (always uses `.`).
    Number(String),
    /// A real date cell, already converted to `YYYY-MM-DD`.
    Date(String),
    /// A cell we cannot use as a value (error cell, boolean, duration, bad date).
    Unsupported,
}

impl Cell {
    /// The cell as text, or `""` when empty/unsupported.
    pub fn text(&self) -> &str {
        match self {
            Cell::Text(s) | Cell::Number(s) | Cell::Date(s) => s,
            Cell::Empty | Cell::Unsupported => "",
        }
    }

    pub fn is_blank(&self) -> bool {
        match self {
            Cell::Empty => true,
            Cell::Text(s) => s.trim().is_empty(),
            _ => false,
        }
    }
}

/// One row of the file with its 1-based number as the user sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRow {
    pub number: usize,
    pub cells: Vec<Cell>,
}

impl RawRow {
    pub fn is_blank(&self) -> bool {
        self.cells.iter().all(Cell::is_blank)
    }
}

/// Reads a CSV file. Row numbers are the line where each record starts.
pub fn read_csv(
    bytes: &[u8],
    delimiter: char,
    quote: char,
    encoding: CsvEncoding,
) -> Result<Vec<RawRow>, ImportError> {
    let encoder = match encoding {
        CsvEncoding::Utf8 => encoding_rs::UTF_8,
        CsvEncoding::Windows1252 => encoding_rs::WINDOWS_1252,
    };
    // Strips a leading UTF-8 BOM but never switches encoding on its own.
    let (text, had_errors) = encoder.decode_with_bom_removal(bytes);
    if had_errors {
        return Err(ImportError::Encoding);
    }

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true) // rows may have different lengths (title rows, summary rows)
        .delimiter(delimiter as u8) // validated as ASCII in ImportProfile::validate
        .quote(quote as u8)
        .from_reader(text.as_bytes());

    // The csv crate's own line counter drifts on blank lines, so the line number is
    // computed from the record's byte offset: 1 + number of '\n' before it. The crate
    // reports the offset where the previous record ended, i.e. before any blank
    // lines, so those line breaks are skipped first.
    let raw = text.as_bytes();
    let mut line = 1;
    let mut counted_up_to = 0;

    let mut rows = Vec::new();
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|e| ImportError::Csv {
            line: e.position().map(|p| p.line()),
        })?;
        let number = match record.position() {
            Some(p) => {
                let mut offset = (p.byte() as usize).min(raw.len());
                while offset < raw.len() && (raw[offset] == b'\r' || raw[offset] == b'\n') {
                    offset += 1;
                }
                if offset >= counted_up_to {
                    line += raw[counted_up_to..offset]
                        .iter()
                        .filter(|b| **b == b'\n')
                        .count();
                    counted_up_to = offset;
                }
                line
            }
            None => index + 1,
        };
        let cells = record
            .iter()
            .map(|field| {
                if field.is_empty() {
                    Cell::Empty
                } else {
                    Cell::Text(field.to_string())
                }
            })
            .collect();
        rows.push(RawRow { number, cells });
    }
    Ok(rows)
}

/// Text of a float cell for later exact parsing.
///
/// Rust prints an `f64` with the shortest digits that read back to the same value
/// (`12.34`, `-1234.5`, `0.30000000000000004`) and never uses exponent notation.
/// That is formatting, not arithmetic, so no money is ever computed with floats.
/// `parse_amount` then rejects anything with more than 2 significant decimals,
/// so a value such as 0.1 + 0.2 becomes a row error instead of being rounded.
fn excel_number_text(value: f64) -> String {
    value.to_string()
}

fn convert_excel_cell(data: &Data) -> Cell {
    match data {
        Data::Empty => Cell::Empty,
        Data::String(s) => Cell::Text(s.clone()),
        // ISO text such as "2024-03-05T00:00:00": keep only the date part.
        Data::DateTimeIso(s) => Cell::Text(s.split('T').next().unwrap_or("").to_string()),
        Data::Int(i) => Cell::Number(i.to_string()),
        Data::Float(f) => Cell::Number(excel_number_text(*f)),
        Data::DateTime(dt) => {
            if !dt.is_datetime() {
                return Cell::Unsupported; // durations (e.g. [hh]:mm) are not dates
            }
            // calamine converts the serial number to year/month/day with integer
            // math and handles both epochs (1900/1904) and Excel's fake 1900-02-29.
            // We drop the time of day on purpose: bank dates are whole days.
            let (year, month, day, ..) = dt.to_ymd_hms_milli();
            match super::dates::make_date(u32::from(year), u32::from(month), u32::from(day)) {
                Some(date) => Cell::Date(date),
                None => Cell::Unsupported,
            }
        }
        Data::Bool(_) | Data::DurationIso(_) | Data::Error(_) => Cell::Unsupported,
    }
}

/// Reads one sheet of an .xlsx/.xls workbook. Leading empty rows are kept so row
/// numbers and `skip_rows` match what Excel shows.
pub fn read_excel(bytes: &[u8], sheet: Option<&str>) -> Result<Vec<RawRow>, ImportError> {
    let mut workbook =
        open_workbook_auto_from_rs(Cursor::new(bytes)).map_err(|_| ImportError::Workbook)?;
    let sheet_name = match sheet {
        Some(name) => name.to_string(),
        None => workbook
            .sheet_names()
            .first()
            .cloned()
            .ok_or(ImportError::SheetNotFound)?,
    };
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|_| ImportError::SheetNotFound)?;

    let (start_row, start_col) = match range.start() {
        Some((r, c)) => (r as usize, c as usize),
        None => return Ok(Vec::new()),
    };

    let mut rows = Vec::new();
    for number in 1..=start_row {
        rows.push(RawRow {
            number,
            cells: Vec::new(),
        });
    }
    for (i, row) in range.rows().enumerate() {
        // Pad on the left so column indexes are absolute (column A = 0).
        let mut cells = vec![Cell::Empty; start_col];
        cells.extend(row.iter().map(convert_excel_cell));
        rows.push(RawRow {
            number: start_row + i + 1,
            cells,
        });
    }
    Ok(rows)
}
