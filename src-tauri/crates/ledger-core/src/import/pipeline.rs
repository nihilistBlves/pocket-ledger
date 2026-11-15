//! Runs a parsed file through the same ingest pipeline as bank sync.
//!
//! `import_file` records an `import_batches` row, parses the file with the given
//! profile and hands the valid rows to `ingest::ingest` with source `import`, so
//! dedup, internal-transfer detection and categorization are identical to sync.
//! Re-importing the same (or an overlapping) file only adds rows that are new.

use rusqlite::{params, Connection};

use super::{parse_file, ImportError, ImportProfile, RowError};
use crate::ingest::{ingest, IngestOrigin, IngestReport};
use crate::model::TxSource;

/// Result of importing one file into one account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    pub batch_id: i64,
    /// Rows the parser rejected (1-based file row numbers, no cell contents).
    pub parse_errors: Vec<RowError>,
    /// What the ingest pipeline did with the valid rows.
    pub report: IngestReport,
}

/// Parses `bytes` with `profile` and ingests the rows into `account_id`.
/// A file-level parse error aborts before anything is written.
pub fn import_file(
    conn: &mut Connection,
    account_id: i64,
    bytes: &[u8],
    file_name: &str,
    profile_name: &str,
    profile: &ImportProfile,
) -> Result<ImportOutcome, ImportError> {
    let parsed = parse_file(bytes, file_name, profile)?;

    conn.execute(
        "INSERT INTO import_batches (account_id, profile, file_name, file_sha256, rows_total, errors)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
        params![
            account_id,
            profile_name,
            file_name,
            parsed.file_sha256,
            parsed.errors.len() as i64
        ],
    )?;
    let batch_id = conn.last_insert_rowid();

    let report = ingest(
        conn,
        account_id,
        parsed.rows,
        TxSource::Import,
        IngestOrigin::ImportBatch(batch_id),
    )?;

    Ok(ImportOutcome {
        batch_id,
        parse_errors: parsed.errors,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::generic_csv_es;
    use crate::model::{NewTransaction, TxStatus};
    use crate::rules::{create_manual_rule, NewRule};
    use crate::test_support::*;

    const FIXTURE_ES: &[u8] = include_bytes!("../../tests/fixtures/generic_es_1252.csv");

    fn run(f: &mut Fixture) -> ImportOutcome {
        let account = f.own_a;
        import_file(
            &mut f.conn,
            account,
            FIXTURE_ES,
            "extracto-demo.csv",
            "Generic CSV (es-ES)",
            &generic_csv_es(),
        )
        .expect("import")
    }

    #[test]
    fn import_inserts_valid_rows_and_reimport_adds_nothing() {
        let mut f = fixture();
        let first = run(&mut f);
        assert_eq!(first.report.inserted, 3);
        assert_eq!(first.parse_errors.len(), 3); // bad date, bad amount, "Saldo final"

        let total: i64 = count(
            &f.conn,
            "SELECT sum(amount_cents) FROM transactions WHERE source = 'import'",
        );
        assert_eq!(total, -340 - 123_456 + 150_000);

        let second = run(&mut f);
        assert_eq!(second.report.inserted, 0);
        assert_eq!(second.report.duplicates, 3);
        assert_eq!(count(&f.conn, "SELECT count(*) FROM transactions"), 3);

        let (rows_total, inserted, duplicates, errors): (i64, i64, i64, i64) = f
            .conn
            .query_row(
                "SELECT rows_total, inserted, duplicates, errors FROM import_batches WHERE id = ?1",
                [second.batch_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((rows_total, inserted, duplicates, errors), (6, 0, 3, 3));
    }

    #[test]
    fn imported_rows_are_categorized_by_rules() {
        let mut f = fixture();
        let groceries = category_id(&f.conn, "food.groceries");
        create_manual_rule(
            &f.conn,
            &NewRule::contains("supermercados norte", groceries),
        )
        .unwrap();
        let out = run(&mut f);
        assert_eq!(out.report.categorized, 1);
        assert_eq!(
            count(
                &f.conn,
                &format!("SELECT count(*) FROM transactions WHERE category_id = {groceries}")
            ),
            1
        );
    }

    #[test]
    fn imported_row_matches_bank_transfer_on_other_own_account() {
        let mut f = fixture();
        // A bank-synced outflow on own_b that mirrors the imported 1.500,00
        // inflow on own_a one day later: the two must be linked as a transfer.
        let mirror = NewTransaction {
            entry_reference: Some("REF-DEMO-1".into()),
            ..booked("2024-03-06", -150_000, "TRASPASO A CUENTA DEMO")
        };
        ingest(
            &mut f.conn,
            f.own_b,
            vec![mirror],
            TxSource::Bank,
            IngestOrigin::None,
        )
        .unwrap();

        let out = run(&mut f);
        assert_eq!(out.report.transfers_matched, 1);
        assert_eq!(
            count(
                &f.conn,
                "SELECT count(*) FROM transactions WHERE category_source = 'transfer'"
            ),
            2
        );
    }

    #[test]
    fn bank_row_adopts_reference_of_unique_imported_twin() {
        let mut f = fixture();
        run(&mut f);
        let bank = NewTransaction {
            status: TxStatus::Booked,
            entry_reference: Some("REF-DEMO-2".into()),
            counterparty: Some("Cafeteria Luna SL".into()),
            ..booked("2024-03-01", -340, "COMPRA TARJ CAFETERIA LUNA")
        };
        let report = ingest(
            &mut f.conn,
            f.own_a,
            vec![bank],
            TxSource::Bank,
            IngestOrigin::None,
        )
        .unwrap();
        assert_eq!(report.adopted, 1);
        assert_eq!(report.inserted, 0);
        assert_eq!(count(&f.conn, "SELECT count(*) FROM transactions"), 3);
    }
}
