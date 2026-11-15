//! File readers: turn CSV, Excel or HTML-table bytes into a plain table of [`RawRow`]s.
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

/// How many leading bytes are inspected for sniffing and for a charset declaration.
const SNIFF_LIMIT: usize = 4096;
/// Guards against absurd `colspan` values creating huge rows.
const MAX_COLSPAN: usize = 256;
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

fn find_ci(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle))
}

/// True when the bytes are an HTML/XHTML document rather than a real workbook.
///
/// Some banks (Openbank) export an XHTML table named `.xls`. Detection is by
/// content: after an optional UTF-8 BOM and whitespace, the file must start with
/// `<!doctype html`, `<html` or `<table`, or with `<?xml` followed by an `<html`
/// element.
pub fn looks_like_html(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let rest = &bytes[start..];
    let starts = |p: &[u8]| rest.len() >= p.len() && rest[..p.len()].eq_ignore_ascii_case(p);
    if starts(b"<!doctype html") || starts(b"<html") || starts(b"<table") {
        return true;
    }
    starts(b"<?xml") && find_ci(&rest[..rest.len().min(SNIFF_LIMIT)], b"<html")
}

/// Finds `charset=<label>` in the head of the document (meta charset or
/// http-equiv content-type). Only ASCII is looked at.
fn declared_charset(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let head = &bytes[..bytes.len().min(SNIFF_LIMIT)];
    let lower: Vec<u8> = head.iter().map(u8::to_ascii_lowercase).collect();
    let key = b"charset";
    let mut from = 0;
    while let Some(pos) = lower[from..]
        .windows(key.len())
        .position(|w| w == key)
        .map(|p| p + from)
    {
        let mut i = pos + key.len();
        while i < lower.len() && lower[i] == b' ' {
            i += 1;
        }
        if i < lower.len() && lower[i] == b'=' {
            i += 1;
            while i < lower.len() && matches!(lower[i], b' ' | b'"' | b'\'') {
                i += 1;
            }
            let end = lower[i..]
                .iter()
                .position(|b| {
                    !(b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
                })
                .map_or(lower.len(), |e| e + i);
            if let Some(enc) = encoding_rs::Encoding::for_label(&lower[i..end]) {
                return Some(enc);
            }
        }
        from = pos + key.len();
    }
    None
}

fn collect_text(node: scraper::ElementRef<'_>, out: &mut String) {
    use scraper::Node;
    for child in node.children() {
        match child.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(el) => {
                let name = el.name();
                if name == "script" || name == "style" {
                    continue;
                }
                if name == "br" {
                    out.push(' ');
                }
                if let Some(e) = scraper::ElementRef::wrap(child) {
                    collect_text(e, out);
                }
            }
            _ => {}
        }
    }
}

/// Text content with all whitespace (including U+00A0) collapsed and trimmed.
fn cell_text(cell: scraper::ElementRef<'_>) -> String {
    let mut raw = String::new();
    collect_text(cell, &mut raw);
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads every `<tr>` of every table, in document order, as one flat list of rows.
///
/// `RawRow::number` is the 1-based index in that flattened list (the HTML has no
/// native row numbers); empty rows are kept so the numbers stay stable. Cells are
/// `<td>`/`<th>`; a `colspan=n` cell yields n cells (text in the first, the rest
/// empty). The charset comes from a BOM, then `<meta>`, then UTF-8, falling back
/// to windows-1252 when an undeclared file is not valid UTF-8.
/// Scripts are never run and nothing external is loaded.
pub fn read_html_table(bytes: &[u8]) -> Result<Vec<RawRow>, ImportError> {
    let has_bom = bytes.starts_with(UTF8_BOM)
        || bytes.starts_with(b"\xFF\xFE")
        || bytes.starts_with(b"\xFE\xFF");
    let declared = if has_bom {
        None
    } else {
        declared_charset(bytes)
    };
    // `decode` lets a BOM override the chosen encoding.
    let (mut text, _, mut had_errors) = declared.unwrap_or(encoding_rs::UTF_8).decode(bytes);
    if had_errors && !has_bom && declared.is_none() {
        // Undeclared and not valid UTF-8: Spanish bank exports are almost always
        // windows-1252 in that case (which never fails to decode).
        (text, _, had_errors) = encoding_rs::WINDOWS_1252.decode(bytes);
    }
    if had_errors {
        return Err(ImportError::Encoding);
    }

    let doc = scraper::Html::parse_document(&text);
    let tr_sel = scraper::Selector::parse("tr").map_err(|_| ImportError::Workbook)?;
    let mut rows = Vec::new();
    for (i, tr) in doc.select(&tr_sel).enumerate() {
        let mut cells = Vec::new();
        for node in tr.descendants() {
            let Some(el) = scraper::ElementRef::wrap(node) else {
                continue;
            };
            if !matches!(el.value().name(), "td" | "th") {
                continue;
            }
            // Skip cells that belong to a nested table's own row.
            let owner = el
                .ancestors()
                .filter_map(scraper::ElementRef::wrap)
                .find(|a| a.value().name() == "tr");
            if owner.map(|o| o.id()) != Some(tr.id()) {
                continue;
            }
            let text = cell_text(el);
            cells.push(if text.is_empty() {
                Cell::Empty
            } else {
                Cell::Text(text)
            });
            let span = el
                .value()
                .attr("colspan")
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, MAX_COLSPAN);
            cells.extend(std::iter::repeat_n(Cell::Empty, span - 1));
        }
        rows.push(RawRow {
            number: i + 1,
            cells,
        });
    }
    Ok(rows)
}
