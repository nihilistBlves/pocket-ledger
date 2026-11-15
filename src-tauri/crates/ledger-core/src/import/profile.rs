//! Import profiles: how to read one kind of bank/CSV/Excel file.
//!
//! A profile is plain data (serde JSON, stored in `import_profiles.spec_json`), so
//! supporting a new bank means adding a profile, not code. Built-in profiles are
//! seeded into the database; user profiles are saved next to them.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::ImportError;
use crate::money::DecimalStyle;

/// Text encoding of a CSV file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsvEncoding {
    Utf8,
    Windows1252,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FileFormat {
    /// Delimiter and quote must be ASCII characters.
    Csv {
        delimiter: char,
        encoding: CsvEncoding,
        quote: char,
    },
    /// `sheet: None` reads the first sheet.
    Excel { sheet: Option<String> },
}

/// A column given by header name or by position (0-based).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnRef {
    /// Matched against the header ignoring case, accents and surrounding spaces.
    Name(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AmountColumns {
    /// One signed column (negative = outflow).
    Single { column: ColumnRef },
    /// Separate columns. The debit column is always an outflow, whatever its sign.
    DebitCredit { debit: ColumnRef, credit: ColumnRef },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Columns {
    pub booking_date: ColumnRef,
    pub value_date: Option<ColumnRef>,
    pub description: ColumnRef,
    pub counterparty: Option<ColumnRef>,
    pub remittance: Option<ColumnRef>,
    pub entry_reference: Option<ColumnRef>,
    pub amount: AmountColumns,
}

/// Serializable twin of [`DecimalStyle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecimalSpec {
    pub decimal: char,
    pub thousands: Option<char>,
}

impl DecimalSpec {
    pub fn style(self) -> DecimalStyle {
        DecimalStyle {
            decimal: self.decimal,
            thousands: self.thousands,
        }
    }
}

fn default_currency() -> String {
    "EUR".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportProfile {
    pub format: FileFormat,
    /// Rows before the header (or before the data if `has_header` is false).
    /// `None` = find the header row automatically in the first 20 rows.
    pub skip_rows: Option<usize>,
    pub has_header: bool,
    pub columns: Columns,
    /// Tried in order. Supported pieces: `%d %m %Y %y`.
    pub date_formats: Vec<String>,
    pub decimal: DecimalSpec,
    #[serde(default = "default_currency")]
    pub currency: String,
}

impl ImportProfile {
    /// Checks the profile itself (not a file) before any parsing.
    pub fn validate(&self) -> Result<(), ImportError> {
        if let FileFormat::Csv {
            delimiter, quote, ..
        } = &self.format
        {
            if !delimiter.is_ascii() || !quote.is_ascii() {
                return Err(ImportError::InvalidProfile(
                    "delimiter and quote must be ASCII",
                ));
            }
        }
        if self.date_formats.is_empty()
            || !self
                .date_formats
                .iter()
                .all(|f| super::dates::is_supported_format(f))
        {
            return Err(ImportError::InvalidProfile("unsupported date format"));
        }
        if self.decimal.thousands == Some(self.decimal.decimal) {
            return Err(ImportError::InvalidProfile(
                "decimal and thousands separators must differ",
            ));
        }
        if !self.has_header && self.uses_header_names() {
            return Err(ImportError::InvalidProfile(
                "columns by name need has_header = true",
            ));
        }
        Ok(())
    }

    fn uses_header_names(&self) -> bool {
        let c = &self.columns;
        let mut refs = vec![&c.booking_date, &c.description];
        refs.extend(
            [
                &c.value_date,
                &c.counterparty,
                &c.remittance,
                &c.entry_reference,
            ]
            .into_iter()
            .flatten(),
        );
        match &c.amount {
            AmountColumns::Single { column } => refs.push(column),
            AmountColumns::DebitCredit { debit, credit } => {
                refs.push(debit);
                refs.push(credit);
            }
        }
        refs.iter().any(|r| matches!(r, ColumnRef::Name(_)))
    }

    /// Header names the file must contain (optional columns are not included).
    pub(super) fn required_header_names(&self) -> Vec<&str> {
        let c = &self.columns;
        let mut refs = vec![&c.booking_date, &c.description];
        match &c.amount {
            AmountColumns::Single { column } => refs.push(column),
            AmountColumns::DebitCredit { debit, credit } => {
                refs.push(debit);
                refs.push(credit);
            }
        }
        refs.into_iter()
            .filter_map(|r| match r {
                ColumnRef::Name(n) => Some(n.as_str()),
                ColumnRef::Index(_) => None,
            })
            .collect()
    }
}

fn name(n: &str) -> ColumnRef {
    ColumnRef::Name(n.to_string())
}

/// Semicolon CSV as exported by many Spanish banks: Windows-1252, `1.234,56`, `dd/mm/yyyy`.
pub fn generic_csv_es() -> ImportProfile {
    ImportProfile {
        format: FileFormat::Csv {
            delimiter: ';',
            encoding: CsvEncoding::Windows1252,
            quote: '"',
        },
        skip_rows: None,
        has_header: true,
        columns: Columns {
            booking_date: name("Fecha"),
            value_date: Some(name("Fecha valor")),
            description: name("Concepto"),
            counterparty: None,
            remittance: None,
            entry_reference: None,
            amount: AmountColumns::Single {
                column: name("Importe"),
            },
        },
        date_formats: vec!["%d/%m/%Y".to_string()],
        decimal: DecimalSpec {
            decimal: ',',
            thousands: Some('.'),
        },
        currency: default_currency(),
    }
}

/// Openbank Excel export ("Cuentas - Movimientos").
///
/// Layout confirmed by the owner on 2026-11-15 (header names and order, preamble,
/// date format, no footer): a title row, preamble rows (account number, description,
/// holder, balance, download date), then the header "Fecha de operación",
/// "Fecha valor", "Concepto", "Importe", "Saldo". The balance column is ignored.
/// Header matching is case- and accent-insensitive and the header row is detected
/// automatically, so the preamble length does not matter.
// The export is a legacy .xls (confirmed), read by calamine's BIFF reader.
// "Importe" is one signed column: charges negative, income positive (confirmed).
// UNVERIFIED: whether "Importe" is a numeric cell or "1.234,56" text (both are
// supported), and that the .xls is real BIFF and not an HTML table renamed .xls.
pub fn openbank_excel() -> ImportProfile {
    ImportProfile {
        format: FileFormat::Excel { sheet: None },
        skip_rows: None,
        has_header: true,
        columns: Columns {
            booking_date: name("Fecha de operación"),
            value_date: Some(name("Fecha valor")),
            description: name("Concepto"),
            counterparty: None,
            remittance: None,
            entry_reference: None,
            amount: AmountColumns::Single {
                column: name("Importe"),
            },
        },
        date_formats: vec!["%d/%m/%Y".to_string(), "%Y-%m-%d".to_string()],
        decimal: DecimalSpec {
            decimal: ',',
            thousands: Some('.'),
        },
        currency: default_currency(),
    }
}

/// A profile together with the name it is stored under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProfile {
    pub name: String,
    pub builtin: bool,
    pub profile: ImportProfile,
}

/// The built-in profiles, in a stable order.
pub fn profiles_seed() -> Vec<(String, ImportProfile)> {
    vec![
        ("Generic CSV (es-ES)".to_string(), generic_csv_es()),
        ("Openbank Excel".to_string(), openbank_excel()),
    ]
}

/// Inserts the built-in profiles. Safe to call on every start: existing built-ins
/// are refreshed, user profiles are never touched.
pub fn seed_builtin_profiles(conn: &Connection) -> Result<(), ImportError> {
    for (name, profile) in profiles_seed() {
        let json = serde_json::to_string(&profile)?;
        conn.execute(
            "INSERT INTO import_profiles (name, spec_json, builtin) VALUES (?1, ?2, 1)
             ON CONFLICT(name) DO UPDATE SET spec_json = excluded.spec_json
             WHERE import_profiles.builtin = 1",
            params![name, json],
        )?;
    }
    Ok(())
}

/// Saves (or replaces) a user profile. Built-in profiles cannot be overwritten.
pub fn save_profile(
    conn: &Connection,
    name: &str,
    profile: &ImportProfile,
) -> Result<(), ImportError> {
    profile.validate()?;
    let json = serde_json::to_string(profile)?;
    let changed = conn.execute(
        "INSERT INTO import_profiles (name, spec_json, builtin) VALUES (?1, ?2, 0)
         ON CONFLICT(name) DO UPDATE SET spec_json = excluded.spec_json
         WHERE import_profiles.builtin = 0",
        params![name, json],
    )?;
    if changed == 0 {
        return Err(ImportError::BuiltinProfile);
    }
    Ok(())
}

/// Loads one profile by name.
pub fn load_profile(conn: &Connection, name: &str) -> Result<Option<ImportProfile>, ImportError> {
    let mut stmt = conn.prepare("SELECT spec_json FROM import_profiles WHERE name = ?1")?;
    let mut rows = stmt.query(params![name])?;
    match rows.next()? {
        Some(row) => {
            let json: String = row.get(0)?;
            Ok(Some(serde_json::from_str(&json)?))
        }
        None => Ok(None),
    }
}

/// All stored profiles, built-ins first, then by name.
pub fn list_profiles(conn: &Connection) -> Result<Vec<StoredProfile>, ImportError> {
    let mut stmt = conn.prepare(
        "SELECT name, spec_json, builtin FROM import_profiles ORDER BY builtin DESC, name",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let json: String = row.get(1)?;
        out.push(StoredProfile {
            name: row.get(0)?,
            builtin: row.get::<_, i64>(2)? == 1,
            profile: serde_json::from_str(&json)?,
        });
    }
    Ok(out)
}
