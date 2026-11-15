use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};

use super::dates::{parse_date, parse_with_formats};
use super::*;
use crate::db::{open_in_memory, DbKey};

const FIXTURE_ES: &[u8] = include_bytes!("../../tests/fixtures/generic_es_1252.csv");
const FIXTURE_DEBIT_CREDIT: &[u8] = include_bytes!("../../tests/fixtures/debit_credit_utf8.csv");

fn name(n: &str) -> ColumnRef {
    ColumnRef::Name(n.to_string())
}

/// Plain UTF-8 CSV profile with comma delimiter, dot decimals and ISO dates.
fn simple_profile(columns: Columns) -> ImportProfile {
    ImportProfile {
        format: FileFormat::Csv {
            delimiter: ',',
            encoding: CsvEncoding::Utf8,
            quote: '"',
        },
        skip_rows: None,
        has_header: true,
        columns,
        date_formats: vec!["%Y-%m-%d".to_string(), "%d/%m/%y".to_string()],
        decimal: DecimalSpec {
            decimal: '.',
            thousands: None,
        },
        currency: "EUR".to_string(),
    }
}

fn single_columns(date: ColumnRef, desc: ColumnRef, amount: ColumnRef) -> Columns {
    Columns {
        booking_date: date,
        value_date: None,
        description: desc,
        counterparty: None,
        remittance: None,
        entry_reference: None,
        amount: AmountColumns::Single { column: amount },
    }
}

fn parse_csv(text: &str, profile: &ImportProfile) -> ParsedImport {
    parse_file(text.as_bytes(), "test.csv", profile).expect("parse")
}

// ---------- dates ----------

#[test]
fn dates_validate_the_real_calendar() {
    assert_eq!(
        parse_date("29/02/2024", "%d/%m/%Y").as_deref(),
        Some("2024-02-29")
    );
    assert_eq!(parse_date("29/02/2023", "%d/%m/%Y"), None); // not a leap year
    assert_eq!(parse_date("29/02/1900", "%d/%m/%Y"), None); // century, not leap
    assert_eq!(
        parse_date("29/02/2000", "%d/%m/%Y").as_deref(),
        Some("2000-02-29")
    );
    assert_eq!(parse_date("31/04/2024", "%d/%m/%Y"), None);
    assert_eq!(parse_date("00/04/2024", "%d/%m/%Y"), None);
    assert_eq!(parse_date("10/13/2024", "%d/%m/%Y"), None);
}

#[test]
fn dates_support_all_listed_formats() {
    assert_eq!(
        parse_date("05-03-2024", "%d-%m-%Y").as_deref(),
        Some("2024-03-05")
    );
    assert_eq!(
        parse_date("2024-03-05", "%Y-%m-%d").as_deref(),
        Some("2024-03-05")
    );
    assert_eq!(
        parse_date("5/3/24", "%d/%m/%y").as_deref(),
        Some("2024-03-05")
    );
    assert_eq!(
        parse_date(" 05/03/2024 ", "%d/%m/%Y").as_deref(),
        Some("2024-03-05")
    );
}

#[test]
fn dates_reject_wrong_shapes() {
    assert_eq!(parse_date("05/03/24", "%d/%m/%Y"), None); // 2-digit year for %Y
    assert_eq!(parse_date("05/03/2024x", "%d/%m/%Y"), None);
    assert_eq!(parse_date("05.03.2024", "%d/%m/%Y"), None);
    assert_eq!(parse_date("", "%d/%m/%Y"), None);
    assert_eq!(parse_date("05/03/2024", "%d/%H/%Y"), None); // unsupported specifier
}

#[test]
fn dates_try_formats_in_order() {
    let formats = vec!["%d/%m/%Y".to_string(), "%Y-%m-%d".to_string()];
    assert_eq!(
        parse_with_formats("2024-03-05", &formats).as_deref(),
        Some("2024-03-05")
    );
    assert_eq!(
        parse_with_formats("05/03/2024", &formats).as_deref(),
        Some("2024-03-05")
    );
}

// ---------- CSV ----------

#[test]
fn spanish_windows_1252_csv_with_accents_and_quoted_delimiter() {
    let parsed = parse_file(FIXTURE_ES, "extracto.csv", &generic_csv_es()).expect("parse");

    assert_eq!(parsed.rows.len(), 3);
    let first = &parsed.rows[0];
    assert_eq!(first.description, "CAFETERÍA LUNA"); // decoded from 0xCD
    assert_eq!(first.amount_cents, -340);
    assert_eq!(first.booking_date.as_deref(), Some("2024-03-01"));
    assert_eq!(first.value_date.as_deref(), Some("2024-03-01"));
    assert_eq!(first.status, crate::model::TxStatus::Booked);
    assert_eq!(first.currency, "EUR");

    // The ';' inside quotes stays in the description, and 1.234,56 parses exactly.
    assert_eq!(parsed.rows[1].description, "SUPERMERCADOS NORTE; tienda 4");
    assert_eq!(parsed.rows[1].amount_cents, -123_456);
    assert_eq!(parsed.rows[2].amount_cents, 150_000);
}

#[test]
fn errors_have_one_based_file_row_numbers_and_no_content() {
    let parsed = parse_file(FIXTURE_ES, "extracto.csv", &generic_csv_es()).expect("parse");

    // Lines: 1 title, 2 blank, 3 header, 4-5 data, 6 blank (skipped), 7 data,
    // 8 impossible date, 9 bad amount, 10 summary row.
    let found: Vec<(usize, &RowErrorKind)> = parsed
        .errors
        .iter()
        .map(|e| (e.row_number, &e.reason))
        .collect();
    assert_eq!(found.len(), 3);
    assert_eq!(found[0].0, 8);
    assert_eq!(found[1].0, 9);
    assert_eq!(found[2].0, 10);
    assert_eq!(
        *found[0].1,
        RowErrorKind::InvalidDate {
            column: "Fecha".to_string()
        }
    );
    assert_eq!(
        *found[1].1,
        RowErrorKind::InvalidAmount {
            column: "Importe".to_string(),
            problem: AmountProblem::Malformed
        }
    );
    assert_eq!(
        *found[2].1,
        RowErrorKind::InvalidDate {
            column: "Fecha".to_string()
        }
    );

    let dump = format!("{:?}", parsed.errors);
    for secret in ["31/02", "abc", "Saldo", "COMPRA", "1600"] {
        assert!(!dump.contains(secret), "error leaks cell content: {secret}");
    }
}

#[test]
fn debit_credit_columns_treat_debit_as_outflow_whatever_the_sign() {
    let mut profile = simple_profile(Columns {
        booking_date: name("Date"),
        value_date: None,
        description: name("Description"),
        counterparty: None,
        remittance: None,
        entry_reference: None,
        amount: AmountColumns::DebitCredit {
            debit: name("Debit"),
            credit: name("Credit"),
        },
    });
    profile.skip_rows = Some(0);
    let parsed = parse_file(FIXTURE_DEBIT_CREDIT, "x.csv", &profile).expect("parse");

    assert!(parsed.errors.is_empty());
    let amounts: Vec<i64> = parsed.rows.iter().map(|r| r.amount_cents).collect();
    assert_eq!(amounts, vec![-340, 150_000, -1_250]); // -12.50 in debit is still an outflow
}

#[test]
fn debit_and_credit_both_filled_is_ambiguous() {
    let profile = simple_profile(Columns {
        booking_date: name("d"),
        value_date: None,
        description: name("t"),
        counterparty: None,
        remittance: None,
        entry_reference: None,
        amount: AmountColumns::DebitCredit {
            debit: name("out"),
            credit: name("in"),
        },
    });
    let parsed = parse_csv(
        "d,t,out,in\n2024-03-01,X,1.00,2.00\n2024-03-02,Y,,\n",
        &profile,
    );
    assert!(parsed.rows.is_empty());
    assert_eq!(parsed.errors[0].reason, RowErrorKind::AmbiguousAmount);
    assert_eq!(parsed.errors[0].row_number, 2);
    assert!(matches!(
        parsed.errors[1].reason,
        RowErrorKind::InvalidAmount {
            problem: AmountProblem::Empty,
            ..
        }
    ));
}

#[test]
fn header_names_ignore_case_accents_and_spaces() {
    let profile = simple_profile(single_columns(
        name("Fecha Operación"),
        name("descripcion"),
        name("IMPORTE"),
    ));
    let text = "  FECHA OPERACION ,DESCRIPCIÓN, importe \n2024-03-01,Café,2.50\n";
    let parsed = parse_csv(text, &profile);
    assert!(parsed.errors.is_empty());
    assert_eq!(parsed.rows[0].description, "Café");
    assert_eq!(parsed.rows[0].amount_cents, 250);
}

#[test]
fn index_columns_without_header_and_with_skip_rows() {
    let mut profile = simple_profile(single_columns(
        ColumnRef::Index(2),
        ColumnRef::Index(0),
        ColumnRef::Index(1),
    ));
    profile.has_header = false;
    profile.skip_rows = Some(1);
    let parsed = parse_csv(
        "ignored title,,\nTienda A,-5.00,2024-03-01\nTienda B,7.25,01/03/24\n",
        &profile,
    );
    assert!(parsed.errors.is_empty());
    assert_eq!(parsed.rows.len(), 2);
    assert_eq!(parsed.rows[0].amount_cents, -500);
    assert_eq!(parsed.rows[1].amount_cents, 725);
    assert_eq!(parsed.rows[1].booking_date.as_deref(), Some("2024-03-01"));
}

#[test]
fn explicit_skip_rows_with_header() {
    let mut profile = simple_profile(single_columns(name("d"), name("t"), name("a")));
    profile.skip_rows = Some(2);
    let parsed = parse_csv("title\nsubtitle\nd,t,a\n2024-03-01,X,1.00\n", &profile);
    assert_eq!(parsed.rows.len(), 1);
}

#[test]
fn header_is_auto_detected_within_first_20_rows() {
    let profile = simple_profile(single_columns(name("d"), name("t"), name("a")));
    let mut text = String::new();
    for _ in 0..5 {
        text.push_str("noise,,\n");
    }
    text.push_str("d,t,a\n2024-03-01,X,1.00\n");
    let parsed = parse_csv(&text, &profile);
    assert_eq!(parsed.rows.len(), 1);

    let mut far = "noise,,\n".repeat(25);
    far.push_str("d,t,a\n2024-03-01,X,1.00\n");
    let result = parse_file(far.as_bytes(), "a.csv", &profile);
    assert!(matches!(result, Err(ImportError::HeaderNotFound)));
}

#[test]
fn missing_named_column_is_a_file_level_error() {
    let profile = simple_profile(single_columns(name("d"), name("t"), name("amount")));
    let mut explicit = profile.clone();
    explicit.skip_rows = Some(0);
    let result = parse_file(b"d,t,a\n2024-03-01,X,1.00\n", "a.csv", &explicit);
    assert!(matches!(result, Err(ImportError::MissingColumn(c)) if c == "amount"));
}

#[test]
fn blank_rows_are_skipped_and_empty_description_is_an_error() {
    let profile = simple_profile(single_columns(name("d"), name("t"), name("a")));
    let parsed = parse_csv(
        "d,t,a\n,,\n2024-03-01,   ,1.00\n   ,  ,  \n2024-03-02,Tienda,2.00\n",
        &profile,
    );
    assert_eq!(parsed.rows.len(), 1);
    assert_eq!(parsed.errors.len(), 1);
    assert_eq!(parsed.errors[0].row_number, 3);
    assert_eq!(
        parsed.errors[0].reason,
        RowErrorKind::MissingValue {
            column: "t".to_string()
        }
    );
}

#[test]
fn description_whitespace_is_collapsed_and_optional_fields_read() {
    let mut columns = single_columns(name("d"), name("t"), name("a"));
    columns.value_date = Some(name("vd"));
    columns.counterparty = Some(name("cp"));
    columns.remittance = Some(name("rm"));
    columns.entry_reference = Some(name("ref"));
    let profile = simple_profile(columns);
    let text = "d,vd,t,cp,rm,ref,a\n2024-03-01,2024-03-02,\"  CAFETERIA \t  LUNA \",  Luna SL ,,R-1 ,-3.40\n";
    let parsed = parse_csv(text, &profile);
    let tx = &parsed.rows[0];
    assert_eq!(tx.description, "CAFETERIA LUNA");
    assert_eq!(tx.counterparty.as_deref(), Some("Luna SL"));
    assert_eq!(tx.remittance, None);
    assert_eq!(tx.entry_reference.as_deref(), Some("R-1"));
    assert_eq!(tx.value_date.as_deref(), Some("2024-03-02"));
}

#[test]
fn amounts_with_too_many_decimals_are_errors_not_rounded() {
    let profile = simple_profile(single_columns(name("d"), name("t"), name("a")));
    let parsed = parse_csv("d,t,a\n2024-03-01,X,1.005\n2024-03-02,Y,1.50\n", &profile);
    assert_eq!(parsed.rows.len(), 1);
    assert_eq!(
        parsed.errors[0].reason,
        RowErrorKind::InvalidAmount {
            column: "a".to_string(),
            problem: AmountProblem::TooManyDecimals
        }
    );
}

#[test]
fn utf8_bom_is_ignored_and_invalid_utf8_is_rejected() {
    let profile = simple_profile(single_columns(name("d"), name("t"), name("a")));
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend_from_slice(b"d,t,a\n2024-03-01,X,1.00\n");
    assert_eq!(
        parse_file(&with_bom, "a.csv", &profile)
            .expect("bom")
            .rows
            .len(),
        1
    );

    let bad = b"d,t,a\n2024-03-01,Caf\xe9,1.00\n";
    assert!(matches!(
        parse_file(bad, "a.csv", &profile),
        Err(ImportError::Encoding)
    ));
}

#[test]
fn extension_mismatch_is_rejected() {
    let profile = generic_csv_es();
    assert!(matches!(
        parse_file(b"", "datos.xlsx", &profile),
        Err(ImportError::FormatMismatch)
    ));
    assert!(matches!(
        parse_file(b"", "datos.csv", &openbank_excel()),
        Err(ImportError::FormatMismatch)
    ));
}

#[test]
fn invalid_profiles_are_rejected() {
    let mut profile = generic_csv_es();
    profile.date_formats = vec!["%H:%M".to_string()];
    assert!(matches!(
        parse_file(b"", "a.csv", &profile),
        Err(ImportError::InvalidProfile(_))
    ));
    let mut by_name_no_header = generic_csv_es();
    by_name_no_header.has_header = false;
    assert!(matches!(
        parse_file(b"", "a.csv", &by_name_no_header),
        Err(ImportError::InvalidProfile(_))
    ));
}

#[test]
fn file_sha256_is_stable_and_correct() {
    let mut profile = simple_profile(single_columns(
        ColumnRef::Index(0),
        ColumnRef::Index(1),
        ColumnRef::Index(2),
    ));
    profile.has_header = false;
    let a = parse_file(b"abc", "a.csv", &profile).expect("parse");
    let b = parse_file(b"abc", "other-name.csv", &profile).expect("parse");
    assert_eq!(a.file_sha256, b.file_sha256);
    assert_eq!(
        a.file_sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

// ---------- Excel ----------

struct SheetRows {
    bytes: Vec<u8>,
}

/// Builds an .xlsx like the assumed Openbank layout: title rows, header, data.
fn build_xlsx() -> SheetRows {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let date_format = Format::new().set_num_format("dd/mm/yyyy");

    sheet
        .write_string(0, 0, "Movimientos cuenta demo")
        .expect("write");
    // row index 1 stays empty
    for (col, title) in [
        "Fecha Operación",
        "Fecha Valor",
        "Concepto",
        "Importe",
        "Saldo",
    ]
    .iter()
    .enumerate()
    {
        sheet.write_string(2, col as u16, *title).expect("write");
    }

    // Excel row 4: string dates, numeric amount 12.34
    sheet.write_string(3, 0, "05/03/2024").expect("write");
    sheet.write_string(3, 1, "05/03/2024").expect("write");
    sheet
        .write_string(3, 2, "  CAFETERIA   LUNA ")
        .expect("write");
    sheet.write_number(3, 3, 12.34).expect("write");

    // Excel row 5: real date cells, -1234.5
    let d = ExcelDateTime::from_ymd(2024, 2, 29).expect("date");
    sheet
        .write_datetime_with_format(4, 0, &d, &date_format)
        .expect("write");
    sheet
        .write_datetime_with_format(4, 1, &d, &date_format)
        .expect("write");
    sheet
        .write_string(4, 2, "SUPERMERCADOS NORTE")
        .expect("write");
    sheet.write_number(4, 3, -1234.5).expect("write");

    // Excel row 6: amount that is not exactly representable in cents
    sheet.write_string(5, 0, "06/03/2024").expect("write");
    sheet
        .write_string(5, 2, "NOMINA EMPRESA DEMO")
        .expect("write");
    sheet
        .write_number(5, 3, 0.300_000_000_000_000_04)
        .expect("write");

    // Excel row 7: text amount in Spanish style
    sheet.write_string(6, 0, "07/03/2024").expect("write");
    sheet
        .write_string(6, 2, "TRANSFERENCIA DEMO")
        .expect("write");
    sheet.write_string(6, 3, "1.234,56").expect("write");

    // Excel row 8: summary row
    sheet.write_string(7, 0, "Saldo final").expect("write");
    sheet.write_number(7, 4, 99.0).expect("write");

    SheetRows {
        bytes: workbook.save_to_buffer().expect("xlsx bytes"),
    }
}

#[test]
fn xlsx_with_title_rows_string_dates_date_cells_and_numeric_amounts() {
    let file = build_xlsx();
    let parsed = parse_file(&file.bytes, "movimientos.xlsx", &openbank_excel()).expect("parse");

    assert_eq!(parsed.rows.len(), 3);
    assert_eq!(parsed.rows[0].amount_cents, 1_234);
    assert_eq!(parsed.rows[0].description, "CAFETERIA LUNA");
    assert_eq!(parsed.rows[0].booking_date.as_deref(), Some("2024-03-05"));

    // Real date cell (leap day) and a negative float amount.
    assert_eq!(parsed.rows[1].booking_date.as_deref(), Some("2024-02-29"));
    assert_eq!(parsed.rows[1].value_date.as_deref(), Some("2024-02-29"));
    assert_eq!(parsed.rows[1].amount_cents, -123_450);

    // Text amount follows the profile's decimal style.
    assert_eq!(parsed.rows[2].amount_cents, 123_456);
    assert_eq!(parsed.rows[2].booking_date.as_deref(), Some("2024-03-07"));
}

#[test]
fn xlsx_float_noise_and_summary_rows_are_row_errors() {
    let file = build_xlsx();
    let parsed = parse_file(&file.bytes, "movimientos.xlsx", &openbank_excel()).expect("parse");

    assert_eq!(parsed.errors.len(), 2);
    // 0.30000000000000004 is rejected, never rounded to 0.30.
    assert_eq!(parsed.errors[0].row_number, 6);
    assert_eq!(
        parsed.errors[0].reason,
        RowErrorKind::InvalidAmount {
            column: "Importe".to_string(),
            problem: AmountProblem::TooManyDecimals
        }
    );
    // "Saldo final" fails the date parse.
    assert_eq!(parsed.errors[1].row_number, 8);
    assert!(matches!(
        parsed.errors[1].reason,
        RowErrorKind::InvalidDate { .. }
    ));
}

#[test]
fn xlsx_named_sheet_and_missing_sheet() {
    let file = build_xlsx();
    let mut profile = openbank_excel();
    profile.format = FileFormat::Excel {
        sheet: Some("Sheet1".to_string()),
    };
    assert_eq!(
        parse_file(&file.bytes, "a.xlsx", &profile)
            .expect("parse")
            .rows
            .len(),
        3
    );
    profile.format = FileFormat::Excel {
        sheet: Some("Nope".to_string()),
    };
    assert!(matches!(
        parse_file(&file.bytes, "a.xlsx", &profile),
        Err(ImportError::SheetNotFound)
    ));
}

#[test]
fn garbage_bytes_are_not_a_workbook() {
    assert!(matches!(
        parse_file(b"not a spreadsheet", "a.xlsx", &openbank_excel()),
        Err(ImportError::Workbook)
    ));
}

// ---------- profiles ----------

#[test]
fn profile_json_round_trip() {
    for (_, profile) in profiles_seed() {
        let json = serde_json::to_string(&profile).expect("serialize");
        let back: ImportProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, profile);
    }
    let mut dc = generic_csv_es();
    dc.columns.amount = AmountColumns::DebitCredit {
        debit: ColumnRef::Index(3),
        credit: name("Abono"),
    };
    let json = serde_json::to_string(&dc).expect("serialize");
    assert_eq!(
        serde_json::from_str::<ImportProfile>(&json).expect("de"),
        dc
    );
}

#[test]
fn currency_defaults_to_eur_when_missing_from_json() {
    let mut value = serde_json::to_value(generic_csv_es()).expect("value");
    value.as_object_mut().expect("object").remove("currency");
    let profile: ImportProfile = serde_json::from_value(value).expect("deserialize");
    assert_eq!(profile.currency, "EUR");
}

fn mem() -> rusqlite::Connection {
    open_in_memory(&DbKey::from_bytes([0x42; 32])).expect("open")
}

#[test]
fn builtin_profiles_are_seeded_idempotently() {
    let conn = mem();
    seed_builtin_profiles(&conn).expect("seed 1");
    seed_builtin_profiles(&conn).expect("seed 2");

    let stored = list_profiles(&conn).expect("list");
    assert_eq!(stored.len(), profiles_seed().len());
    assert!(stored.iter().all(|p| p.builtin));
    for (name, profile) in profiles_seed() {
        assert_eq!(load_profile(&conn, &name).expect("load"), Some(profile));
    }
    assert_eq!(load_profile(&conn, "missing").expect("load"), None);
}

#[test]
fn user_profiles_save_replace_and_cannot_shadow_builtins() {
    let conn = mem();
    seed_builtin_profiles(&conn).expect("seed");

    let mut mine = generic_csv_es();
    save_profile(&conn, "Mi banco", &mine).expect("save");
    mine.skip_rows = Some(4);
    save_profile(&conn, "Mi banco", &mine).expect("replace");
    assert_eq!(load_profile(&conn, "Mi banco").expect("load"), Some(mine));

    let builtin_name = profiles_seed()[0].0.clone();
    assert!(matches!(
        save_profile(&conn, &builtin_name, &generic_csv_es()),
        Err(ImportError::BuiltinProfile)
    ));

    // Re-seeding leaves the user profile alone.
    seed_builtin_profiles(&conn).expect("reseed");
    let stored = list_profiles(&conn).expect("list");
    assert_eq!(stored.len(), profiles_seed().len() + 1);
    assert!(
        !stored
            .iter()
            .find(|p| p.name == "Mi banco")
            .expect("mine")
            .builtin
    );
}
