//! Runs a parsed file through the same ingest pipeline as bank sync.
//!
//! `import_file` records an `import_batches` row, parses the file with the given
//! profile and hands the valid rows to `ingest::ingest` with source `import`, so
//! dedup, internal-transfer detection and categorization are identical to sync.
//! Re-importing the same (or an overlapping) file only adds rows that are new.
//!
//! Account identity: when the file's preamble names an IBAN, the target account
//! must be that account. A target without an identity adopts the file's, unless
//! another account (e.g. one linked by bank sync) already has it; then the import is
//! refused instead of creating a second copy of the same bank account.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::{parse_file, ImportError, ImportProfile, RowError};
use crate::identity::AccountIdentity;
use crate::ingest::{ingest_in, IngestOrigin, IngestReport};
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

/// How the file's account identity relates to the chosen target account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum IdentityCheck {
    /// The file names no account (no IBAN in its preamble): nothing to compare.
    NoIdentity,
    /// The target account has the file's identity.
    Match,
    /// The target has no identity yet and would adopt the file's on import.
    WillAdopt,
    /// The target has a different identity.
    Mismatch,
    /// Another local account (e.g. one linked by bank sync) already has the file's identity.
    BelongsToOther { existing_account_id: i64 },
    /// The target account does not exist.
    AccountNotFound,
}

/// Compares the file's identity with the target account. Reads only.
pub(super) fn check_identity(
    conn: &Connection,
    account_id: i64,
    identity: Option<&AccountIdentity>,
) -> Result<IdentityCheck, ImportError> {
    let current: Option<Option<String>> = conn
        .query_row("SELECT identity_hash FROM accounts WHERE id = ?1", [account_id], |r| r.get(0))
        .optional()?;
    let Some(current) = current else {
        return Ok(IdentityCheck::AccountNotFound);
    };
    let Some(identity) = identity else {
        return Ok(IdentityCheck::NoIdentity);
    };
    match current {
        Some(hash) if hash == identity.hash => Ok(IdentityCheck::Match),
        Some(_) => Ok(IdentityCheck::Mismatch),
        None => match account_with_identity(conn, &identity.hash)? {
            Some(existing_account_id) => Ok(IdentityCheck::BelongsToOther { existing_account_id }),
            None => Ok(IdentityCheck::WillAdopt),
        },
    }
}

/// The account whose `identity_hash` equals `hash`, if any.
pub(super) fn account_with_identity(
    conn: &Connection,
    hash: &str,
) -> Result<Option<i64>, ImportError> {
    Ok(conn
        .query_row("SELECT id FROM accounts WHERE identity_hash = ?1", [hash], |r| r.get(0))
        .optional()?)
}

/// Checks the file's identity against the target account and, when the target has
/// none yet, stores it there. Writes nothing on error.
fn reconcile_identity(
    conn: &Connection,
    account_id: i64,
    identity: &AccountIdentity,
) -> Result<(), ImportError> {
    match check_identity(conn, account_id, Some(identity))? {
        IdentityCheck::AccountNotFound => Err(ImportError::AccountNotFound),
        IdentityCheck::Mismatch => Err(ImportError::AccountMismatch),
        IdentityCheck::BelongsToOther { existing_account_id } => {
            Err(ImportError::AccountBelongsToOther { existing_account_id })
        }
        IdentityCheck::Match | IdentityCheck::NoIdentity => Ok(()),
        IdentityCheck::WillAdopt => {
            conn.execute(
                "UPDATE accounts SET identity_hash = ?1, display_suffix = COALESCE(display_suffix, ?2)
                 WHERE id = ?3",
                params![identity.hash, identity.suffix, account_id],
            )?;
            Ok(())
        }
    }
}

/// Parses `bytes` with `profile` and ingests the rows into `account_id`.
/// A file-level parse error, a file without any valid row or an account identity
/// conflict (see the module docs) aborts before anything is written.
///
/// The batch row and the ingest share one transaction, so a failure leaves no
/// half-filled batch behind. The batch stores `rows_total` (valid rows plus rejected
/// ones), `inserted`, `updated`, `duplicates` and `errors` (rejected rows).
pub fn import_file(
    conn: &mut Connection,
    account_id: i64,
    bytes: &[u8],
    file_name: &str,
    profile_name: &str,
    profile: &ImportProfile,
) -> Result<ImportOutcome, ImportError> {
    let parsed = parse_file(bytes, file_name, profile)?;
    if parsed.rows.is_empty() {
        return Err(ImportError::NothingToImport);
    }
    if let Some(identity) = &parsed.account_identity {
        reconcile_identity(conn, account_id, identity)?;
    } else if check_identity(conn, account_id, None)? == IdentityCheck::AccountNotFound {
        return Err(ImportError::AccountNotFound);
    }

    let db_tx = conn.transaction()?;
    db_tx.execute(
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
    let batch_id = db_tx.last_insert_rowid();

    // `ingest_in` adds its own counts on top of the rejected rows stored above.
    let report = ingest_in(
        &db_tx,
        account_id,
        parsed.rows,
        TxSource::Import,
        IngestOrigin::ImportBatch(batch_id),
    )?;
    db_tx.commit()?;

    Ok(ImportOutcome {
        batch_id,
        parse_errors: parsed.errors,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::{generic_csv_es, openbank_excel};
    use crate::ingest::ingest;
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

    // ---------- account identity ----------

    /// Fictitious IBANs (invalid check digits on purpose).
    const FAKE_IBAN: &str = "ES00 0000 0000 0000 0000 0001";
    const OTHER_IBAN: &str = "ES00 0000 0000 0000 0000 0002";

    fn openbank_doc(iban: &str) -> Vec<u8> {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><table><tr><td>Cuentas - Movimientos</td></tr><tr><td>N&uacute;mero de cuenta</td><td>{iban}</td></tr><tr><td>Titular</td><td>Persona Ficticia</td></tr><tr><th>Fecha de operaci&oacute;n</th><th>Fecha valor</th><th>Concepto</th><th>Importe</th><th>Saldo</th></tr><tr><td>05/03/2024</td><td>05/03/2024</td><td>CAFETERIA LUNA</td><td>-3,40</td><td>996,60</td></tr></table></body></html>"
        )
        .into_bytes()
    }

    fn import_openbank(f: &mut Fixture, account: i64, iban: &str) -> Result<ImportOutcome, ImportError> {
        import_file(
            &mut f.conn,
            account,
            &openbank_doc(iban),
            "movimientos.xls",
            "Openbank Excel",
            &openbank_excel(),
        )
    }

    fn identity_hash(f: &Fixture, account: i64) -> Option<String> {
        f.conn
            .query_row("SELECT identity_hash FROM accounts WHERE id = ?1", [account], |r| r.get(0))
            .unwrap()
    }

    fn set_identity(f: &Fixture, account: i64, iban: &str) {
        let id = AccountIdentity::from_iban(iban).unwrap();
        f.conn
            .execute(
                "UPDATE accounts SET identity_hash = ?1, display_suffix = ?2 WHERE id = ?3",
                params![id.hash, id.suffix, account],
            )
            .unwrap();
    }

    #[test]
    fn import_adopts_the_preamble_identity_and_reimport_matches_it() {
        let mut f = fixture();
        let account = f.own_a;
        let out = import_openbank(&mut f, account, FAKE_IBAN).expect("import");
        assert_eq!(out.report.inserted, 1);

        let expected = AccountIdentity::from_iban(FAKE_IBAN).unwrap();
        assert_eq!(identity_hash(&f, account), Some(expected.hash));
        let suffix: String = f
            .conn
            .query_row("SELECT display_suffix FROM accounts WHERE id = ?1", [account], |r| r.get(0))
            .unwrap();
        assert_eq!(suffix, "0001");
        // The IBAN itself is stored nowhere.
        let dump: String = f
            .conn
            .query_row(
                "SELECT group_concat(coalesce(name,'') || coalesce(identity_hash,'') || coalesce(display_suffix,'')) FROM accounts",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!dump.contains("ES0000"));

        let again = import_openbank(&mut f, account, FAKE_IBAN).expect("reimport");
        assert_eq!(again.report.duplicates, 1);
    }

    #[test]
    fn import_into_a_new_account_is_refused_when_a_synced_account_has_the_iban() {
        let mut f = fixture();
        let (target, synced) = (f.own_a, f.own_b);
        set_identity(&f, synced, FAKE_IBAN); // as bank sync would have done

        let err = import_openbank(&mut f, target, FAKE_IBAN).unwrap_err();
        assert!(
            matches!(err, ImportError::AccountBelongsToOther { existing_account_id } if existing_account_id == synced),
            "{err:?}"
        );
        assert!(!err.to_string().contains("0001"));
        assert_eq!(identity_hash(&f, target), None);
        assert_eq!(count(&f.conn, "SELECT count(*) FROM transactions"), 0);
        assert_eq!(count(&f.conn, "SELECT count(*) FROM import_batches"), 0);

        // Importing into the synced account itself works.
        let out = import_openbank(&mut f, synced, FAKE_IBAN).expect("import into synced");
        assert_eq!(out.report.inserted, 1);
    }

    #[test]
    fn import_is_refused_when_the_file_names_a_different_account() {
        let mut f = fixture();
        let account = f.own_a;
        set_identity(&f, account, OTHER_IBAN);
        assert!(matches!(
            import_openbank(&mut f, account, FAKE_IBAN),
            Err(ImportError::AccountMismatch)
        ));
        assert_eq!(count(&f.conn, "SELECT count(*) FROM transactions"), 0);
    }

    #[test]
    fn import_into_a_missing_account_is_refused() {
        let mut f = fixture();
        assert!(matches!(
            import_openbank(&mut f, 9_999, FAKE_IBAN),
            Err(ImportError::AccountNotFound)
        ));
    }

    #[test]
    fn file_without_preamble_iban_leaves_identity_alone() {
        let mut f = fixture();
        run(&mut f);
        assert_eq!(identity_hash(&f, f.own_a), None);
    }
}
