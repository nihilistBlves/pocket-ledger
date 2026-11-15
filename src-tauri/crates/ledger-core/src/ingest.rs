//! The ingest pipeline: validate -> dedup -> store -> link transfers -> categorize.
//!
//! Bank sync and file imports both call `ingest` with a batch of
//! `NewTransaction` for one account. Everything runs in ONE SQLite transaction:
//! if anything fails nothing is stored. Internal-transfer detection runs before
//! categorization, so a transfer is never categorized by a rule. Invalid rows
//! are reported by index and reason only, never with their content.

use std::collections::HashSet;

use rusqlite::{params, Connection, Result};

use crate::dedup::{apply, BatchDeduper, DedupOutcome, Provenance};
use crate::model::{NewTransaction, TxSource, TxStatus};
use crate::{rules, transfers};

/// Which bookkeeping row (if any) gets its counters updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOrigin {
    ImportBatch(i64),
    SyncRun(i64),
    None,
}

/// A rejected row: its 0-based position in the input and why (a fixed message,
/// never part of the row's data).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowError {
    pub row: usize,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestReport {
    pub inserted: usize,
    /// Rows rewritten because the bank changed them (same entry_reference).
    pub updated: usize,
    /// Pending rows that became booked.
    pub promoted: usize,
    /// Existing rows that received a bank reference.
    pub adopted: usize,
    pub duplicates: usize,
    /// Internal-transfer pairs linked.
    pub transfers_matched: usize,
    /// Rows given a category by a rule.
    pub categorized: usize,
    pub errors: Vec<RowError>,
    /// Ids of rows inserted, updated or promoted (the ones that were processed further).
    pub touched_ids: Vec<i64>,
    /// Ids of every valid row of the batch, including duplicates: what sync
    /// passes to `dedup::reconcile_stale_pending`.
    pub seen_ids: Vec<i64>,
}

/// Runs the whole pipeline for one batch of one account.
pub fn ingest(
    conn: &mut Connection,
    account_id: i64,
    rows: Vec<NewTransaction>,
    source: TxSource,
    origin: IngestOrigin,
) -> Result<IngestReport> {
    let db_tx = conn.transaction()?;
    let provenance = Provenance {
        source,
        import_batch_id: match origin {
            IngestOrigin::ImportBatch(id) => Some(id),
            _ => None,
        },
        sync_run_id: match origin {
            IngestOrigin::SyncRun(id) => Some(id),
            _ => None,
        },
    };

    let mut report = IngestReport::default();
    let mut deduper = BatchDeduper::new(account_id);

    for (index, mut row) in rows.iter().cloned().enumerate() {
        if let Err(reason) = validate(&mut row) {
            report.errors.push(RowError { row: index, reason });
            continue;
        }
        let decision = deduper.decide(&db_tx, &row)?;
        let id = apply(&db_tx, account_id, &row, &decision, &provenance)?;
        report.seen_ids.push(id);
        match decision.outcome {
            DedupOutcome::Insert => {
                report.inserted += 1;
                report.touched_ids.push(id);
            }
            DedupOutcome::Update(_) => {
                report.updated += 1;
                report.touched_ids.push(id);
            }
            DedupOutcome::Promote(_) => {
                report.promoted += 1;
                report.touched_ids.push(id);
            }
            DedupOutcome::AdoptReference(_) => report.adopted += 1,
            DedupOutcome::Duplicate(_) => report.duplicates += 1,
        }
    }

    // Transfers first, so that linked rows are skipped by the rules below.
    let pairs = transfers::detect_internal_transfers(&db_tx, &report.touched_ids)?;
    report.transfers_matched = pairs.len();
    let linked: HashSet<i64> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let remaining: Vec<i64> = report
        .touched_ids
        .iter()
        .copied()
        .filter(|id| !linked.contains(id))
        .collect();
    report.categorized = rules::categorize(&db_tx, &remaining)?;

    update_counters(&db_tx, origin, rows.len(), &report)?;
    db_tx.commit()?;
    Ok(report)
}

/// Adds this batch's numbers to the import batch or sync run row.
fn update_counters(
    conn: &Connection,
    origin: IngestOrigin,
    total: usize,
    r: &IngestReport,
) -> Result<()> {
    let count = |n: usize| n as i64;
    match origin {
        IngestOrigin::ImportBatch(id) => {
            conn.execute(
                "UPDATE import_batches
                 SET rows_total = rows_total + ?1, inserted = inserted + ?2,
                     updated = updated + ?3, duplicates = duplicates + ?4, errors = errors + ?5
                 WHERE id = ?6",
                params![
                    count(total),
                    count(r.inserted),
                    count(r.updated + r.promoted),
                    count(r.duplicates + r.adopted),
                    count(r.errors.len()),
                    id
                ],
            )?;
        }
        IngestOrigin::SyncRun(id) => {
            conn.execute(
                "UPDATE sync_runs
                 SET fetched = fetched + ?1, inserted = inserted + ?2, updated = updated + ?3
                 WHERE id = ?4",
                params![
                    count(total),
                    count(r.inserted),
                    count(r.updated + r.promoted),
                    id
                ],
            )?;
        }
        IngestOrigin::None => {}
    }
    Ok(())
}

/// Checks one row (and upper-cases its currency). The error is a fixed message.
fn validate(row: &mut NewTransaction) -> std::result::Result<(), &'static str> {
    if row.currency.len() != 3 || !row.currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err("currency must be 3 letters");
    }
    row.currency = row.currency.to_ascii_uppercase();
    for date in [&row.booking_date, &row.value_date].into_iter().flatten() {
        if !is_valid_date(date) {
            return Err("date must be a valid YYYY-MM-DD");
        }
    }
    if row.status == TxStatus::Booked && row.booking_date.is_none() {
        return Err("booked row needs a booking date");
    }
    Ok(())
}

/// `YYYY-MM-DD` with a real calendar day (leap years included).
fn is_valid_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits_only = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
    if !(digits_only(0..4) && digits_only(5..7) && digits_only(8..10)) {
        return false;
    }
    // All ten bytes are ASCII here, so slicing the text is safe.
    let (Ok(year), Ok(month), Ok(day)) = (
        text[0..4].parse::<u32>(),
        text[5..7].parse::<u32>(),
        text[8..10].parse::<u32>(),
    ) else {
        return false;
    };
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::NewRule;
    use crate::test_support::*;

    #[test]
    fn date_validation() {
        for ok in ["2026-03-01", "2024-02-29", "2000-02-29"] {
            assert!(is_valid_date(ok), "{ok}");
        }
        for bad in [
            "2026-13-01",
            "2026-02-29",
            "1900-02-29",
            "2026-04-31",
            "2026-3-01",
            "26-03-01",
            "2026/03/01",
            "2026-03-0x",
            "",
            "２０２６-03-01",
        ] {
            assert!(!is_valid_date(bad), "{bad}");
        }
    }

    #[test]
    fn invalid_rows_are_reported_without_their_content() {
        let mut f = fixture();
        let mut no_date = booked("2026-03-01", -100, "SECRETO UNO");
        no_date.booking_date = None;
        let mut bad_currency = booked("2026-03-01", -100, "SECRETO DOS");
        bad_currency.currency = "EU".into();
        let bad_date = booked("2026-13-01", -100, "SECRETO TRES");
        let good = booked("2026-03-01", -100, "CAFETERIA LUNA");

        let r = ingest(
            &mut f.conn,
            f.own_a,
            vec![no_date, bad_currency, good, bad_date],
            TxSource::Import,
            IngestOrigin::None,
        )
        .unwrap();
        assert_eq!(r.inserted, 1);
        assert_eq!(
            r.errors.iter().map(|e| e.row).collect::<Vec<_>>(),
            vec![0, 1, 3]
        );
        let shown = format!("{:?}", r.errors);
        assert!(!shown.contains("SECRETO"));
        assert_eq!(count(&f.conn, "SELECT count(*) FROM transactions"), 1);
    }

    #[test]
    fn lowercase_currency_is_accepted_and_stored_upper_case() {
        let mut f = fixture();
        let mut row = booked("2026-03-01", -100, "CAFETERIA LUNA");
        row.currency = "eur".into();
        let r = ingest(
            &mut f.conn,
            f.own_a,
            vec![row],
            TxSource::Import,
            IngestOrigin::None,
        )
        .unwrap();
        assert_eq!(r.inserted, 1);
        assert_eq!(
            count(
                &f.conn,
                "SELECT count(*) FROM transactions WHERE currency = 'EUR'"
            ),
            1
        );
    }

    #[test]
    fn transfers_are_linked_before_categorization() {
        let mut f = fixture();
        let groceries = category_id(&f.conn, "food.groceries");
        crate::rules::create_manual_rule(&f.conn, &NewRule::contains("traspaso", groceries))
            .unwrap();

        let first = ingest(
            &mut f.conn,
            f.own_a,
            vec![booked("2026-03-01", -20_000, "TRASPASO A AHORRO")],
            TxSource::Bank,
            IngestOrigin::None,
        )
        .unwrap();
        assert_eq!((first.transfers_matched, first.categorized), (0, 1)); // no partner yet
        let second = ingest(
            &mut f.conn,
            f.own_b,
            vec![booked("2026-03-02", 20_000, "TRASPASO DE CUENTA")],
            TxSource::Bank,
            IngestOrigin::None,
        )
        .unwrap();
        assert_eq!((second.transfers_matched, second.categorized), (1, 0));

        let transfer = category_id(&f.conn, "transfer.internal");
        assert_eq!(count(&f.conn, &format!("SELECT count(*) FROM transactions WHERE category_source = 'transfer' AND category_id = {transfer}")), 2);
        assert_eq!(
            count(
                &f.conn,
                "SELECT count(*) FROM transactions WHERE category_source = 'rule'"
            ),
            0
        );
    }

    #[test]
    fn counters_are_added_to_the_batch_and_sync_rows() {
        let mut f = fixture();
        f.conn
            .execute(
                "INSERT INTO import_batches (account_id, profile) VALUES (?1, 'demo')",
                [f.own_a],
            )
            .unwrap();
        let batch = f.conn.last_insert_rowid();
        let rows = vec![
            booked("2026-03-01", -100, "CAFETERIA LUNA"),
            booked("2026-03-01", -100, "CAFETERIA LUNA"),
            booked("2026-03-01", -100, ""),
        ];
        let mut bad = rows.clone();
        bad[2].currency = "X".into();
        ingest(
            &mut f.conn,
            f.own_a,
            bad.clone(),
            TxSource::Import,
            IngestOrigin::ImportBatch(batch),
        )
        .unwrap();
        ingest(
            &mut f.conn,
            f.own_a,
            bad,
            TxSource::Import,
            IngestOrigin::ImportBatch(batch),
        )
        .unwrap();
        let (total, inserted, dups, errs): (i64, i64, i64, i64) = f
            .conn
            .query_row(
                "SELECT rows_total, inserted, duplicates, errors FROM import_batches WHERE id = ?1",
                [batch],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((total, inserted, dups, errs), (6, 2, 2, 2));
        assert_eq!(
            count(
                &f.conn,
                &format!("SELECT count(*) FROM transactions WHERE import_batch_id = {batch}")
            ),
            2
        );

        f.conn
            .execute(
                "INSERT INTO sync_runs (account_id, provider) VALUES (?1, 'enable_banking')",
                [f.own_a],
            )
            .unwrap();
        let run = f.conn.last_insert_rowid();
        ingest(
            &mut f.conn,
            f.own_a,
            vec![booked("2026-04-01", -100, "SUPERMERCADOS NORTE")],
            TxSource::Bank,
            IngestOrigin::SyncRun(run),
        )
        .unwrap();
        assert_eq!(
            count(
                &f.conn,
                &format!("SELECT fetched * 10 + inserted FROM sync_runs WHERE id = {run}")
            ),
            11
        );
    }

    #[test]
    fn a_failure_rolls_the_whole_batch_back() {
        let mut f = fixture();
        // Unknown account: the foreign key fails on the first insert.
        let r = ingest(
            &mut f.conn,
            9999,
            vec![booked("2026-03-01", -100, "CAFETERIA LUNA")],
            TxSource::Bank,
            IngestOrigin::None,
        );
        assert!(r.is_err());
        assert_eq!(count(&f.conn, "SELECT count(*) FROM transactions"), 0);
    }
}
