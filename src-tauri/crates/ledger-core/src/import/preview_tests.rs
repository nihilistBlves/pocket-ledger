//! Tests of the import screen's backend: raw preview, sniffing, detection, dry run,
//! batch bookkeeping, history and undo. Fictitious data only.

use rusqlite::params;

use super::*;
use crate::identity::AccountIdentity;
use crate::ingest::{ingest, IngestOrigin, RowDisposition};
use crate::model::{NewTransaction, TxSource};
use crate::test_support::*;

const GENERIC_1252: &[u8] = include_bytes!("../../tests/fixtures/generic_es_1252.csv");
const DEBIT_CREDIT: &[u8] = include_bytes!("../../tests/fixtures/debit_credit_utf8.csv");
const OPENBANK: &[u8] = include_bytes!("../../tests/fixtures/openbank_demo.html");
/// Fictitious IBAN of the Openbank fixture (invalid check digits on purpose).
const FIXTURE_IBAN: &str = "ES00 0000 0000 0000 0000 0001";

fn stored_builtins() -> Vec<StoredProfile> {
    profiles_seed()
        .into_iter()
        .map(|(name, profile)| StoredProfile {
            name,
            builtin: true,
            profile,
        })
        .collect()
}

fn csv_format(delimiter: char, encoding: CsvEncoding) -> FileFormat {
    FileFormat::Csv {
        delimiter,
        encoding,
        quote: '"',
    }
}

fn import_generic(f: &mut Fixture, account: i64) -> ImportOutcome {
    import_file(
        &mut f.conn,
        account,
        GENERIC_1252,
        "extracto-demo.csv",
        "Generic CSV (es-ES)",
        &generic_csv_es(),
    )
    .expect("import")
}

fn preview_generic(f: &mut Fixture, account: Option<i64>) -> ImportPreview {
    preview_import(
        &mut f.conn,
        account,
        GENERIC_1252,
        "extracto-demo.csv",
        &generic_csv_es(),
    )
    .expect("preview")
}

fn tx_count(f: &Fixture) -> i64 {
    count(&f.conn, "SELECT count(*) FROM transactions")
}

// ---------- raw rows ----------

#[test]
fn raw_rows_of_a_csv_keep_file_row_numbers_and_skip_blank_lines() {
    let format = csv_format(';', CsvEncoding::Windows1252);
    let table = raw_rows(GENERIC_1252, "extracto-demo.csv", &format, 4).unwrap();
    assert_eq!(table.rows.len(), 4);
    assert_eq!(table.row_numbers, vec![1, 3, 4, 5]);
    assert_eq!(table.rows[1][0], "Fecha");
    assert_eq!(table.rows[3][2], "SUPERMERCADOS NORTE; tienda 4");
    assert_eq!(table.rows[2][2], "CAFETERÍA LUNA"); // Windows-1252 decoded
    assert_eq!(table.column_count, 5);
    assert_eq!(table.total_rows, 8);

    let all = raw_rows(GENERIC_1252, "extracto-demo.csv", &format, 10_000).unwrap();
    assert_eq!(all.rows.len(), 8);
}

#[test]
fn raw_rows_reads_the_html_xls_by_content() {
    let format = FileFormat::Excel { sheet: None };
    let table = raw_rows(OPENBANK, "movimientos.xls", &format, 50).unwrap();
    assert_eq!(table.total_rows, 7);
    assert_eq!(table.rows[0][0], "Cuentas - Movimientos");
    assert_eq!(table.rows[3][0], "Fecha de operación");
    assert_eq!(table.rows[4][2], "CAFETERIA LUNA");
    assert_eq!(table.row_numbers[4], 5);
}

#[test]
fn raw_rows_rejects_wrong_extension_bad_encoding_and_huge_cells_are_cut() {
    let csv = csv_format(';', CsvEncoding::Utf8);
    assert!(matches!(
        raw_rows(GENERIC_1252, "extracto.xlsx", &csv, 5),
        Err(ImportError::FormatMismatch)
    ));
    assert!(matches!(
        raw_rows(GENERIC_1252, "extracto.csv", &csv, 5),
        Err(ImportError::Encoding)
    ));
    let long = format!("{}\n", "x".repeat(500));
    let table = raw_rows(long.as_bytes(), "a.csv", &csv, 5).unwrap();
    assert_eq!(table.rows[0][0].chars().count(), 120);
}

#[test]
fn files_over_the_size_cap_are_refused() {
    let big = vec![b'a'; MAX_FILE_BYTES + 1];
    let csv = csv_format(';', CsvEncoding::Utf8);
    assert!(matches!(
        raw_rows(&big, "a.csv", &csv, 5),
        Err(ImportError::TooLarge)
    ));
    assert!(matches!(
        parse_file(&big, "a.csv", &generic_csv_es()),
        Err(ImportError::TooLarge)
    ));
}

// ---------- sniffing ----------

#[test]
fn sniff_finds_delimiter_and_encoding() {
    assert_eq!(
        sniff_csv_format(GENERIC_1252),
        csv_format(';', CsvEncoding::Windows1252)
    );
    assert_eq!(
        sniff_csv_format(DEBIT_CREDIT),
        csv_format(',', CsvEncoding::Utf8)
    );
    let tab = "Fecha\tConcepto\tImporte\n01/03/2024\tCafé\t-3,40\n02/03/2024\tLuz\t-9,00\n";
    assert_eq!(
        sniff_csv_format(tab.as_bytes()),
        csv_format('\t', CsvEncoding::Utf8)
    );
    let pipe = "a|b|c\n1|2|3\n4|5|6\n";
    assert_eq!(
        sniff_csv_format(pipe.as_bytes()),
        csv_format('|', CsvEncoding::Utf8)
    );
}

#[test]
fn sniff_ignores_delimiters_inside_quotes_and_defaults_to_semicolon() {
    let quoted = "\"a,b,c,d\";\"x\";\"y\"\n\"e,f,g,h\";\"1\";\"2\"\n";
    assert_eq!(
        sniff_csv_format(quoted.as_bytes()),
        csv_format(';', CsvEncoding::Utf8)
    );
    assert_eq!(
        sniff_csv_format(b"just text"),
        csv_format(';', CsvEncoding::Utf8)
    );
    assert_eq!(sniff_csv_format(b""), csv_format(';', CsvEncoding::Utf8));
    // A BOM does not turn UTF-8 into something else.
    let mut bom = b"\xEF\xBB\xBF".to_vec();
    bom.extend_from_slice("Fecha;Concepto\n1;Caf\u{e9}\n".as_bytes());
    assert_eq!(sniff_csv_format(&bom), csv_format(';', CsvEncoding::Utf8));
}

// ---------- detection ----------

#[test]
fn openbank_file_is_detected_first() {
    let found = detect_profile(OPENBANK, "movimientos.xls", &stored_builtins());
    assert_eq!(found.as_deref(), Some("Openbank Excel"));
}

#[test]
fn generic_csv_is_detected_only_when_its_named_columns_are_present() {
    let profiles = stored_builtins();
    assert_eq!(
        detect_profile(GENERIC_1252, "extracto.csv", &profiles).as_deref(),
        Some("Generic CSV (es-ES)")
    );
    // Other layouts (English headers) are not guessed, and never read as Openbank.
    assert_eq!(detect_profile(DEBIT_CREDIT, "otro.csv", &profiles), None);
    // The Openbank document is not a CSV and the CSV is not Excel.
    assert_eq!(detect_profile(OPENBANK, "movimientos.csv", &profiles), None);
    assert_eq!(
        detect_profile(b"nada;util\n1;2\n", "a.csv", &profiles),
        None
    );
}

#[test]
fn utf8_file_is_not_detected_by_a_windows_1252_profile() {
    let text = "Fecha;Fecha valor;Concepto;Importe\n01/03/2024;01/03/2024;Cafeter\u{ed}a;-3,40\n";
    assert_eq!(
        detect_profile(text.as_bytes(), "a.csv", &stored_builtins()),
        None
    );
    // Pure ASCII is fine either way.
    let ascii = "Fecha;Fecha valor;Concepto;Importe\n01/03/2024;01/03/2024;Cafeteria;-3,40\n";
    assert!(detect_profile(ascii.as_bytes(), "a.csv", &stored_builtins()).is_some());
}

#[test]
fn user_profiles_are_detected_after_built_ins_and_positional_ones_never() {
    let mut user = generic_csv_es();
    user.format = csv_format(',', CsvEncoding::Utf8);
    user.columns.booking_date = ColumnRef::Name("Date".into());
    user.columns.value_date = None;
    user.columns.description = ColumnRef::Name("Description".into());
    user.columns.amount = AmountColumns::DebitCredit {
        debit: ColumnRef::Name("Debit".into()),
        credit: ColumnRef::Name("Credit".into()),
    };
    user.date_formats = vec!["%Y-%m-%d".into()];
    user.decimal = DecimalSpec {
        decimal: '.',
        thousands: None,
    };
    let positional = ImportProfile {
        has_header: false,
        skip_rows: Some(0),
        columns: Columns {
            booking_date: ColumnRef::Index(0),
            value_date: None,
            description: ColumnRef::Index(1),
            counterparty: None,
            remittance: None,
            entry_reference: None,
            amount: AmountColumns::Single {
                column: ColumnRef::Index(2),
            },
        },
        ..user.clone()
    };
    let mut profiles = stored_builtins();
    profiles.push(StoredProfile {
        name: "Posicional".into(),
        builtin: false,
        profile: positional,
    });
    assert_eq!(detect_profile(DEBIT_CREDIT, "otro.csv", &profiles), None);
    profiles.push(StoredProfile {
        name: "Mi banco".into(),
        builtin: false,
        profile: user,
    });
    assert_eq!(
        detect_profile(DEBIT_CREDIT, "otro.csv", &profiles).as_deref(),
        Some("Mi banco")
    );
}

// ---------- dry run ----------

#[test]
fn preview_writes_nothing_and_reports_the_real_counts() {
    let mut f = fixture();
    let account = f.own_a;
    let before = (
        tx_count(&f),
        count(&f.conn, "SELECT count(*) FROM import_batches"),
    );

    let p = preview_generic(&mut f, Some(account));
    assert_eq!((p.total_rows, p.valid_rows, p.parse_error_count), (6, 3, 3));
    assert_eq!(p.parse_errors.len(), 3);
    assert_eq!(p.parse_errors[0].row_number, 8);
    let counts = p.counts.unwrap();
    assert_eq!(
        (
            counts.would_insert,
            counts.duplicates,
            counts.updated,
            counts.rejected
        ),
        (3, 0, 0, 0)
    );
    assert_eq!(p.identity, Some(IdentityCheck::NoIdentity));
    assert_eq!(
        (p.date_from.as_deref(), p.date_to.as_deref()),
        (Some("2024-03-01"), Some("2024-03-05"))
    );
    assert_eq!(p.rows.len(), 3);
    assert_eq!(p.rows[0].row_number, 4);
    assert_eq!(p.rows[1].description, "SUPERMERCADOS NORTE; tienda 4");
    assert_eq!(p.rows[1].amount_cents, -123_456);
    assert!(p.rows.iter().all(|r| r.status == Some(RowDisposition::New)));

    // Nothing was written.
    assert_eq!(
        (
            tx_count(&f),
            count(&f.conn, "SELECT count(*) FROM import_batches")
        ),
        before
    );

    // After a real import the same preview reports duplicates, per row too.
    import_generic(&mut f, account);
    let again = preview_generic(&mut f, Some(account));
    let counts = again.counts.unwrap();
    assert_eq!((counts.would_insert, counts.duplicates), (0, 3));
    assert!(again
        .rows
        .iter()
        .all(|r| r.status == Some(RowDisposition::Duplicate)));
    assert_eq!(tx_count(&f), 3);
}

#[test]
fn preview_counts_transfers_and_rule_categories_without_keeping_them() {
    let mut f = fixture();
    let (a, b) = (f.own_a, f.own_b);
    let groceries = category_id(&f.conn, "food.groceries");
    crate::rules::create_manual_rule(
        &f.conn,
        &crate::rules::NewRule::contains("supermercados norte", groceries),
    )
    .unwrap();
    let mirror = NewTransaction {
        entry_reference: Some("REF-DEMO-1".into()),
        ..booked("2024-03-06", -150_000, "TRASPASO A CUENTA DEMO")
    };
    ingest(
        &mut f.conn,
        b,
        vec![mirror],
        TxSource::Bank,
        IngestOrigin::None,
    )
    .unwrap();

    let p = preview_generic(&mut f, Some(a));
    let c = p.counts.unwrap();
    assert_eq!((c.transfers_matched, c.categorized), (1, 1));
    assert_eq!(
        count(
            &f.conn,
            "SELECT count(*) FROM transactions WHERE category_source != 'none'"
        ),
        0
    );
    assert_eq!(tx_count(&f), 1);
}

#[test]
fn preview_without_account_has_no_counts_and_finds_the_matching_account() {
    let mut f = fixture();
    let account = f.own_b;
    let id = AccountIdentity::from_iban(FIXTURE_IBAN).unwrap();
    f.conn
        .execute(
            "UPDATE accounts SET identity_hash = ?1, display_suffix = ?2 WHERE id = ?3",
            params![id.hash, id.suffix, account],
        )
        .unwrap();
    let profile = openbank_excel();
    let p = preview_import(&mut f.conn, None, OPENBANK, "movimientos.xls", &profile).unwrap();
    assert!(p.has_identity);
    assert_eq!(p.matched_account_id, Some(account));
    assert!(p.counts.is_none() && p.identity.is_none());
    assert_eq!(p.valid_rows, 3);
    assert!(p.rows.iter().all(|r| r.status.is_none()));
    // The IBAN is nowhere in what the UI gets.
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains("ES00") && !json.contains("0001"));
}

#[test]
fn preview_identity_outcomes() {
    let mut f = fixture();
    let (a, b) = (f.own_a, f.own_b);
    let profile = openbank_excel();
    let run = |f: &mut Fixture, account: i64| {
        preview_import(&mut f.conn, Some(account), OPENBANK, "m.xls", &profile).unwrap()
    };

    let p = run(&mut f, a);
    assert_eq!(p.identity, Some(IdentityCheck::WillAdopt));
    assert_eq!(p.counts.unwrap().would_insert, 3);
    // Previewing never adopts the identity.
    assert_eq!(
        count(
            &f.conn,
            "SELECT count(*) FROM accounts WHERE identity_hash IS NOT NULL"
        ),
        0
    );

    let id = AccountIdentity::from_iban(FIXTURE_IBAN).unwrap();
    f.conn
        .execute(
            "UPDATE accounts SET identity_hash = ?1 WHERE id = ?2",
            params![id.hash, b],
        )
        .unwrap();
    assert_eq!(run(&mut f, b).identity, Some(IdentityCheck::Match));
    let other = run(&mut f, a);
    assert_eq!(
        other.identity,
        Some(IdentityCheck::BelongsToOther {
            existing_account_id: b
        })
    );
    assert!(other.counts.is_none());

    f.conn
        .execute(
            "UPDATE accounts SET identity_hash = 'otra' WHERE id = ?1",
            [b],
        )
        .unwrap();
    let mismatch = run(&mut f, b);
    assert_eq!(mismatch.identity, Some(IdentityCheck::Mismatch));
    assert!(mismatch.counts.is_none());

    let missing = run(&mut f, 9_999);
    assert_eq!(missing.identity, Some(IdentityCheck::AccountNotFound));
    assert!(missing.counts.is_none());
    let json = serde_json::to_value(other.identity).unwrap();
    assert_eq!(json["status"], "belongs_to_other");
    assert_eq!(json["existingAccountId"], b);
}

#[test]
fn preview_reports_file_level_errors() {
    let mut f = fixture();
    let err = preview_import(&mut f.conn, None, b"nada\n", "a.csv", &generic_csv_es()).unwrap_err();
    assert!(matches!(err, ImportError::HeaderNotFound));
}

#[test]
fn row_errors_serialize_without_cell_contents() {
    let mut f = fixture();
    let p = preview_generic(&mut f, None);
    let json = serde_json::to_value(&p.parse_errors).unwrap();
    assert_eq!(json[0]["rowNumber"], 8);
    assert_eq!(json[0]["reason"]["kind"], "invalid_date");
    assert_eq!(json[1]["reason"]["kind"], "invalid_amount");
    assert_eq!(json[1]["reason"]["problem"], "malformed");
    let text = json.to_string();
    assert!(!text.contains("abc") && !text.contains("RARA"));
}

// ---------- batch bookkeeping ----------

#[test]
fn import_stores_real_counts_in_the_batch() {
    let mut f = fixture();
    let account = f.own_a;
    let first = import_generic(&mut f, account);
    let row = |f: &Fixture, id: i64| -> (i64, i64, i64, i64, i64) {
        f.conn
            .query_row(
                "SELECT rows_total, inserted, updated, duplicates, errors FROM import_batches WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap()
    };
    assert_eq!(row(&f, first.batch_id), (6, 3, 0, 0, 3));
    let second = import_generic(&mut f, account);
    assert_eq!(row(&f, second.batch_id), (6, 0, 0, 3, 3));
}

#[test]
fn file_without_valid_rows_imports_nothing_and_leaves_no_batch() {
    let mut f = fixture();
    let account = f.own_a;
    let only_header = "Fecha;Fecha valor;Concepto;Importe\n";
    let err = import_file(
        &mut f.conn,
        account,
        only_header.as_bytes(),
        "vacio.csv",
        "Generic CSV (es-ES)",
        &generic_csv_es(),
    )
    .unwrap_err();
    assert!(matches!(err, ImportError::NothingToImport));
    assert_eq!(count(&f.conn, "SELECT count(*) FROM import_batches"), 0);
    assert!(matches!(
        import_file(
            &mut f.conn,
            9_999,
            GENERIC_1252,
            "e.csv",
            "p",
            &generic_csv_es()
        ),
        Err(ImportError::AccountNotFound)
    ));
}

// ---------- history ----------

#[test]
fn history_lists_newest_first_with_account_names_and_can_undo() {
    let mut f = fixture();
    let (a, b) = (f.own_a, f.own_b);
    let first = import_generic(&mut f, a);
    let second = import_generic(&mut f, a); // only duplicates
    let third = import_file(
        &mut f.conn,
        b,
        OPENBANK,
        "movimientos.xls",
        "Openbank Excel",
        &openbank_excel(),
    )
    .unwrap();
    f.conn
        .execute(
            "UPDATE import_batches SET imported_at = '2099-04-01T10:00:00.000Z' WHERE id = ?1",
            [first.batch_id],
        )
        .unwrap();

    let list = list_import_batches(&f.conn, 50).unwrap();
    assert_eq!(list.iter().map(|r| r.id).collect::<Vec<_>>().len(), 3);
    assert_eq!(list[0].id, first.batch_id); // explicit newest timestamp
    let by_id = |id: i64| list.iter().find(|r| r.id == id).unwrap();
    assert_eq!(by_id(first.batch_id).account_name, "Cuenta demo");
    assert_eq!(
        by_id(first.batch_id).file_name.as_deref(),
        Some("extracto-demo.csv")
    );
    assert!(by_id(first.batch_id).can_undo);
    assert!(!by_id(second.batch_id).can_undo); // inserted nothing
    assert_eq!(by_id(third.batch_id).account_name, "Ahorro demo");
    assert_eq!(by_id(third.batch_id).inserted, 3);
    assert_eq!(list_import_batches(&f.conn, 1).unwrap().len(), 1);

    let json = serde_json::to_value(by_id(first.batch_id)).unwrap();
    assert_eq!(json["canUndo"], true);
    assert_eq!(json["rowsTotal"], 6);
    assert!(json["undoneAt"].is_null());
}

// ---------- undo ----------

#[test]
fn undo_deletes_only_the_batch_rows() {
    let mut f = fixture();
    let a = f.own_a;
    // A manual row and a row of an earlier batch must survive.
    let manual = raw_booked(&f.conn, a, "2024-02-01", -500, "MANUAL");
    let first = import_generic(&mut f, a);
    assert_eq!(first.report.inserted, 3);
    let more = "Fecha;Fecha valor;Concepto;Importe\n01/03/2024;01/03/2024;CAFETERIA LUNA;-3,40\n09/03/2024;09/03/2024;GASOLINERA DEMO;-40,00\n";
    let second = import_file(
        &mut f.conn,
        a,
        more.as_bytes(),
        "mas.csv",
        "Generic CSV (es-ES)",
        &generic_csv_es(),
    )
    .unwrap();
    assert_eq!((second.report.inserted, second.report.duplicates), (1, 1));
    // Tag one of the doomed rows: the link must go with it.
    f.conn
        .execute("INSERT INTO tags (name) VALUES ('demo')", [])
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO transaction_tags (transaction_id, tag_id)
             SELECT id, 1 FROM transactions WHERE import_batch_id = ?1 LIMIT 1",
            [second.batch_id],
        )
        .unwrap();

    let report = undo_import_batch(&mut f.conn, second.batch_id).unwrap();
    assert_eq!((report.deleted, report.transfers_unlinked), (1, 0));
    assert_eq!(tx_count(&f), 4); // manual + 3 from the first batch
    assert_eq!(count(&f.conn, "SELECT count(*) FROM transaction_tags"), 0);
    assert_eq!(
        count(
            &f.conn,
            "SELECT count(*) FROM transactions WHERE description LIKE 'GASOLINERA%'"
        ),
        0
    );
    assert!(f
        .conn
        .query_row(
            "SELECT 1 FROM transactions WHERE id = ?1",
            [manual],
            |_| Ok(())
        )
        .is_ok());
    let undone: Option<String> = f
        .conn
        .query_row(
            "SELECT undone_at FROM import_batches WHERE id = ?1",
            [second.batch_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(undone.is_some());
    let row = list_import_batches(&f.conn, 10)
        .unwrap()
        .into_iter()
        .find(|r| r.id == second.batch_id)
        .unwrap();
    assert!(!row.can_undo && row.undone_at.is_some());

    // Undoing again is refused and changes nothing.
    assert!(matches!(
        undo_import_batch(&mut f.conn, second.batch_id),
        Err(ImportError::BatchAlreadyUndone)
    ));
    assert_eq!(tx_count(&f), 4);
    assert!(matches!(
        undo_import_batch(&mut f.conn, 9_999),
        Err(ImportError::BatchNotFound)
    ));
    // The earlier batch can still be undone.
    assert_eq!(
        undo_import_batch(&mut f.conn, first.batch_id)
            .unwrap()
            .deleted,
        3
    );
    assert_eq!(tx_count(&f), 1);
    // Re-importing after an undo inserts the rows again.
    let again = import_generic(&mut f, a);
    assert_eq!(again.report.inserted, 3);
}

#[test]
fn undo_unlinks_transfer_peers_and_resets_their_category() {
    let mut f = fixture();
    let (a, b) = (f.own_a, f.own_b);
    let mirror = NewTransaction {
        entry_reference: Some("REF-DEMO-1".into()),
        ..booked("2024-03-06", -150_000, "TRASPASO A CUENTA DEMO")
    };
    ingest(
        &mut f.conn,
        b,
        vec![mirror],
        TxSource::Bank,
        IngestOrigin::None,
    )
    .unwrap();
    let out = import_generic(&mut f, a);
    assert_eq!(out.report.transfers_matched, 1);
    let peer: i64 = f
        .conn
        .query_row(
            "SELECT id FROM transactions WHERE account_id = ?1",
            [b],
            |r| r.get(0),
        )
        .unwrap();
    // A rule that would match the peer once it is unlinked.
    let transport = category_id(&f.conn, "transport.fuel");
    crate::rules::create_manual_rule(
        &f.conn,
        &crate::rules::NewRule::contains("traspaso a cuenta demo", transport),
    )
    .unwrap();

    let report = undo_import_batch(&mut f.conn, out.batch_id).unwrap();
    assert_eq!((report.deleted, report.transfers_unlinked), (3, 1));
    let (peer_link, source, category): (Option<i64>, String, Option<i64>) = f
        .conn
        .query_row(
            "SELECT transfer_peer_id, category_source, category_id FROM transactions WHERE id = ?1",
            [peer],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(peer_link, None);
    // Same path as `transfers::unlink_transfer`: back to uncategorized, then the rules.
    assert_eq!(source, "rule");
    assert_eq!(category, Some(transport));
    assert_eq!(tx_count(&f), 1);
}

#[test]
fn undo_unlink_without_rules_leaves_the_peer_uncategorized() {
    let mut f = fixture();
    let (a, b) = (f.own_a, f.own_b);
    let mirror = NewTransaction {
        entry_reference: Some("REF-DEMO-1".into()),
        ..booked("2024-03-06", -150_000, "TRASPASO A CUENTA DEMO")
    };
    ingest(
        &mut f.conn,
        b,
        vec![mirror],
        TxSource::Bank,
        IngestOrigin::None,
    )
    .unwrap();
    let out = import_generic(&mut f, a);
    undo_import_batch(&mut f.conn, out.batch_id).unwrap();
    let (source, category): (String, Option<i64>) = f
        .conn
        .query_row(
            "SELECT category_source, category_id FROM transactions WHERE account_id = ?1",
            [b],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((source.as_str(), category), ("none", None));
}

#[test]
fn undo_is_refused_when_bank_sync_adopted_a_row() {
    let mut f = fixture();
    let a = f.own_a;
    let out = import_generic(&mut f, a);
    f.conn
        .execute(
            "INSERT INTO sync_runs (provider) VALUES ('enable_banking')",
            [],
        )
        .unwrap();
    let run = f.conn.last_insert_rowid();
    // The bank reports the imported coffee with a reference: the import row adopts it.
    let bank = NewTransaction {
        entry_reference: Some("REF-DEMO-2".into()),
        ..booked("2024-03-01", -340, "COMPRA TARJ CAFETERIA LUNA")
    };
    let report = ingest(
        &mut f.conn,
        a,
        vec![bank],
        TxSource::Bank,
        IngestOrigin::SyncRun(run),
    )
    .unwrap();
    assert_eq!(report.adopted, 1);

    assert!(matches!(
        undo_import_batch(&mut f.conn, out.batch_id),
        Err(ImportError::BatchHasSyncedRows)
    ));
    assert_eq!(tx_count(&f), 3);
    let batch = list_import_batches(&f.conn, 10).unwrap();
    assert!(!batch[0].can_undo && batch[0].undone_at.is_none());
}

#[test]
fn undo_is_refused_when_bank_sync_rewrote_a_row() {
    let mut f = fixture();
    let a = f.own_a;
    // A file that carries bank references, then the bank changes one description.
    let mut profile = generic_csv_es();
    profile.columns.entry_reference = Some(ColumnRef::Name("Ref".into()));
    let file =
        "Fecha;Fecha valor;Concepto;Importe;Ref\n01/03/2024;01/03/2024;CAFETERIA LUNA;-3,40;R1\n";
    let out = import_file(
        &mut f.conn,
        a,
        file.as_bytes(),
        "r.csv",
        "Con referencia",
        &profile,
    )
    .unwrap();
    f.conn
        .execute(
            "INSERT INTO sync_runs (provider) VALUES ('enable_banking')",
            [],
        )
        .unwrap();
    let run = f.conn.last_insert_rowid();
    let changed = NewTransaction {
        entry_reference: Some("R1".into()),
        ..booked("2024-03-01", -340, "CAFETERIA LUNA MADRID")
    };
    let report = ingest(
        &mut f.conn,
        a,
        vec![changed],
        TxSource::Bank,
        IngestOrigin::SyncRun(run),
    )
    .unwrap();
    assert_eq!(report.updated, 1);
    assert!(matches!(
        undo_import_batch(&mut f.conn, out.batch_id),
        Err(ImportError::BatchHasSyncedRows)
    ));
}

#[test]
fn unrelated_sync_rows_do_not_block_undo() {
    let mut f = fixture();
    let a = f.own_a;
    let out = import_generic(&mut f, a);
    f.conn
        .execute(
            "INSERT INTO sync_runs (provider) VALUES ('enable_banking')",
            [],
        )
        .unwrap();
    let run = f.conn.last_insert_rowid();
    let bank = NewTransaction {
        entry_reference: Some("REF-DEMO-9".into()),
        ..booked("2024-03-20", -1_000, "OTRA COSA")
    };
    ingest(
        &mut f.conn,
        a,
        vec![bank],
        TxSource::Bank,
        IngestOrigin::SyncRun(run),
    )
    .unwrap();
    let report = undo_import_batch(&mut f.conn, out.batch_id).unwrap();
    assert_eq!(report.deleted, 3);
    assert_eq!(tx_count(&f), 1);
}
