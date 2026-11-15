//! Duplicate detection for incoming transactions.
//!
//! Every row gets a fingerprint (hash of account, status, date, amount,
//! counterparty and text) plus an occurrence index, so that two identical
//! coffees on the same day stay two rows while re-fetching the same window adds
//! nothing. `BatchDeduper::decide` returns what to do with each incoming row
//! (see the numbered steps there) and `apply` performs it. Pending rows that
//! the bank later books are promoted in place instead of duplicated.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension, Result};
use sha2::{Digest, Sha256};

use crate::model::{NewTransaction, TxSource, TxStatus};
use crate::normalize::normalize_text;

/// A pending row can become the booked row if its date is at most this many days away.
const PROMOTE_WINDOW_DAYS: i64 = 5;

/// What to do with one incoming row. The `i64` is the id of the existing row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupOutcome {
    /// New transaction.
    Insert,
    /// Already stored and identical: do nothing.
    Duplicate(i64),
    /// Same entry_reference but the bank changed some fields: rewrite them.
    Update(i64),
    /// A pending row became booked (matched by reference or by amount/date).
    Promote(i64),
    /// Same transaction stored without a reference: attach the incoming one.
    AdoptReference(i64),
}

/// The outcome plus the fingerprint and occurrence index computed for the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub outcome: DedupOutcome,
    pub fingerprint: String,
    pub occurrence_index: i64,
}

/// Hex sha256 of a versioned canonical string. The status is part of it, so a
/// pending row and its booked version never collide on the unique index.
pub fn fingerprint(account_id: i64, tx: &NewTransaction) -> String {
    let date = tx
        .booking_date
        .as_deref()
        .or(tx.value_date.as_deref())
        .unwrap_or("");
    let counterparty = normalize_text(tx.counterparty.as_deref().unwrap_or(""));
    let text = match tx.remittance.as_deref() {
        Some(r) if !r.trim().is_empty() => r,
        _ => tx.description.as_str(),
    };
    // normalize_text removes every '|', so fields cannot bleed into each other.
    let canonical = format!(
        "v1|{}|{}|{}|{}|{}|{}|{}",
        account_id,
        tx.status.as_str(),
        date,
        tx.amount_cents,
        tx.currency,
        counterparty,
        normalize_text(text)
    );
    hex::encode(Sha256::digest(canonical.as_bytes()))
}

/// Stored columns that dedup needs to look at.
struct Existing {
    id: i64,
    status: String,
    booking_date: Option<String>,
    value_date: Option<String>,
    amount_cents: i64,
    currency: String,
    description: String,
    counterparty: Option<String>,
    remittance: Option<String>,
    entry_reference: Option<String>,
}

const EXISTING_COLUMNS: &str = "id, status, booking_date, value_date, amount_cents, currency, \
                                description, counterparty, remittance, entry_reference";

fn read_existing(row: &rusqlite::Row<'_>) -> Result<Existing> {
    Ok(Existing {
        id: row.get(0)?,
        status: row.get(1)?,
        booking_date: row.get(2)?,
        value_date: row.get(3)?,
        amount_cents: row.get(4)?,
        currency: row.get(5)?,
        description: row.get(6)?,
        counterparty: row.get(7)?,
        remittance: row.get(8)?,
        entry_reference: row.get(9)?,
    })
}

/// Decides the fate of the rows of ONE batch for ONE account, in input order.
/// Remembers how many times each fingerprint appeared (occurrence index) and
/// which stored rows were already matched, so none is matched twice.
pub struct BatchDeduper {
    account_id: i64,
    seen_counts: HashMap<String, i64>,
    claimed: HashSet<i64>,
}

impl BatchDeduper {
    pub fn new(account_id: i64) -> Self {
        BatchDeduper {
            account_id,
            seen_counts: HashMap::new(),
            claimed: HashSet::new(),
        }
    }

    /// Steps, in order; the first that applies wins:
    /// a) the bank reference already exists on this account -> Update/Promote/Duplicate;
    /// b) (fingerprint, occurrence index) already exists -> Duplicate (or AdoptReference);
    /// c) incoming is booked and a matching pending row exists -> Promote;
    /// d) incoming has a reference and exactly one reference-less imported row
    ///    looks like it -> AdoptReference;
    /// e) otherwise Insert.
    pub fn decide(&mut self, conn: &Connection, tx: &NewTransaction) -> Result<Decision> {
        let fp = fingerprint(self.account_id, tx);
        let counter = self.seen_counts.entry(fp.clone()).or_insert(0);
        let idx = *counter;
        *counter += 1;

        let outcome = self.pick_outcome(conn, tx, &fp, idx)?;
        match outcome {
            DedupOutcome::Insert => {}
            DedupOutcome::Duplicate(id)
            | DedupOutcome::Update(id)
            | DedupOutcome::Promote(id)
            | DedupOutcome::AdoptReference(id) => {
                self.claimed.insert(id);
            }
        }
        Ok(Decision {
            outcome,
            fingerprint: fp,
            occurrence_index: idx,
        })
    }

    fn pick_outcome(
        &self,
        conn: &Connection,
        tx: &NewTransaction,
        fp: &str,
        idx: i64,
    ) -> Result<DedupOutcome> {
        // a) same bank reference.
        if let Some(reference) = &tx.entry_reference {
            if let Some(ex) = find_by_reference(conn, self.account_id, reference)? {
                return Ok(outcome_for_same_reference(&ex, tx));
            }
        }

        // b) same fingerprint and occurrence index. If both rows carry different
        // bank references the bank says they are different entries, so no match.
        if let Some(ex) = find_by_fingerprint(conn, self.account_id, fp, idx)? {
            let refs_conflict = matches!(
                (&tx.entry_reference, &ex.entry_reference),
                (Some(a), Some(b)) if a != b
            );
            if !refs_conflict {
                let adopt = tx.entry_reference.is_some() && ex.entry_reference.is_none();
                return Ok(if adopt {
                    DedupOutcome::AdoptReference(ex.id)
                } else {
                    DedupOutcome::Duplicate(ex.id)
                });
            }
        }

        // c) a booked row may be the final version of a pending one.
        // (A pending row with the same reference was already handled in a).)
        if tx.status == TxStatus::Booked {
            if let Some(id) = self.find_pending_to_promote(conn, tx)? {
                return Ok(DedupOutcome::Promote(id));
            }
        }

        // d) cross-source adoption: an imported file row without reference.
        if tx.entry_reference.is_some() && tx.status == TxStatus::Booked {
            if let Some(id) = self.find_unique_import_twin(conn, tx)? {
                return Ok(DedupOutcome::AdoptReference(id));
            }
        }

        // e)
        Ok(DedupOutcome::Insert)
    }

    /// Closest pending row (same account, currency and amount, not claimed,
    /// within the date window). Ties on distance go to the smallest id.
    fn find_pending_to_promote(
        &self,
        conn: &Connection,
        tx: &NewTransaction,
    ) -> Result<Option<i64>> {
        let Some(date) = tx.booking_date.as_deref().or(tx.value_date.as_deref()) else {
            return Ok(None);
        };
        // The day distance is computed by SQLite and comes back as a whole number.
        // A pending row without any date gives NULL: allowed, but ranked last.
        let mut stmt = conn.prepare(
            "SELECT id,
                    CAST(round(abs(julianday(?1) - julianday(COALESCE(booking_date, value_date)))) AS INTEGER)
             FROM transactions
             WHERE account_id = ?2 AND status = 'pending' AND currency = ?3 AND amount_cents = ?4",
        )?;
        let rows = stmt.query_map(
            params![date, self.account_id, tx.currency, tx.amount_cents],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?)),
        )?;

        let mut best: Option<(i64, i64)> = None; // (distance, id)
        for row in rows {
            let (id, distance) = row?;
            if self.claimed.contains(&id) {
                continue;
            }
            if matches!(distance, Some(d) if d > PROMOTE_WINDOW_DAYS) {
                continue;
            }
            let candidate = (distance.unwrap_or(i64::MAX), id);
            if best.is_none_or(|b| candidate < b) {
                best = Some(candidate);
            }
        }
        Ok(best.map(|(_, id)| id))
    }

    /// The single imported, reference-less, booked row with the same date and
    /// amount. Zero or several candidates -> None (too risky to guess).
    fn find_unique_import_twin(
        &self,
        conn: &Connection,
        tx: &NewTransaction,
    ) -> Result<Option<i64>> {
        let mut stmt = conn.prepare(
            "SELECT id FROM transactions
             WHERE account_id = ?1 AND entry_reference IS NULL AND source = 'import'
               AND status = 'booked' AND booking_date = ?2 AND amount_cents = ?3
               AND currency = ?4
             ORDER BY id",
        )?;
        let ids = stmt
            .query_map(
                params![
                    self.account_id,
                    tx.booking_date,
                    tx.amount_cents,
                    tx.currency
                ],
                |r| r.get::<_, i64>(0),
            )?
            .collect::<Result<Vec<i64>>>()?;
        let free: Vec<i64> = ids
            .into_iter()
            .filter(|id| !self.claimed.contains(id))
            .collect();
        Ok(if free.len() == 1 { Some(free[0]) } else { None })
    }
}

/// Step a): what to do when the stored row has the same bank reference.
fn outcome_for_same_reference(ex: &Existing, tx: &NewTransaction) -> DedupOutcome {
    // Never turn a booked row back into a pending one.
    if ex.status == "booked" && tx.status == TxStatus::Pending {
        return DedupOutcome::Duplicate(ex.id);
    }
    let changed = ex.status != tx.status.as_str()
        || ex.booking_date != tx.booking_date
        || ex.value_date != tx.value_date
        || ex.amount_cents != tx.amount_cents
        || ex.currency != tx.currency
        || ex.description != tx.description
        || ex.counterparty != tx.counterparty
        || ex.remittance != tx.remittance;
    if !changed {
        DedupOutcome::Duplicate(ex.id)
    } else if ex.status == "pending" && tx.status == TxStatus::Booked {
        DedupOutcome::Promote(ex.id)
    } else {
        DedupOutcome::Update(ex.id)
    }
}

fn find_by_reference(
    conn: &Connection,
    account_id: i64,
    reference: &str,
) -> Result<Option<Existing>> {
    conn.query_row(
        &format!(
            "SELECT {EXISTING_COLUMNS} FROM transactions
             WHERE account_id = ?1 AND entry_reference = ?2"
        ),
        params![account_id, reference],
        read_existing,
    )
    .optional()
}

fn find_by_fingerprint(
    conn: &Connection,
    account_id: i64,
    fp: &str,
    idx: i64,
) -> Result<Option<Existing>> {
    conn.query_row(
        &format!(
            "SELECT {EXISTING_COLUMNS} FROM transactions
             WHERE account_id = ?1 AND fingerprint = ?2 AND occurrence_index = ?3"
        ),
        params![account_id, fp, idx],
        read_existing,
    )
    .optional()
}

/// First occurrence index >= `start` not used by another row with this fingerprint.
fn free_occurrence(
    conn: &Connection,
    account_id: i64,
    fp: &str,
    start: i64,
    except_id: i64,
) -> Result<i64> {
    let mut idx = start;
    loop {
        let taken: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM transactions
                           WHERE account_id = ?1 AND fingerprint = ?2
                             AND occurrence_index = ?3 AND id != ?4)",
            params![account_id, fp, idx, except_id],
            |r| r.get(0),
        )?;
        if !taken {
            return Ok(idx);
        }
        idx += 1;
    }
}

/// Where a stored row came from; filled in on insert only.
#[derive(Debug, Clone, Copy)]
pub struct Provenance {
    pub source: TxSource,
    pub import_batch_id: Option<i64>,
    pub sync_run_id: Option<i64>,
}

/// Performs the decision. Returns the id of the row that was inserted or matched.
/// Category, rule, notes and tags of existing rows are never touched.
pub fn apply(
    conn: &Connection,
    account_id: i64,
    tx: &NewTransaction,
    decision: &Decision,
    provenance: &Provenance,
) -> Result<i64> {
    match decision.outcome {
        DedupOutcome::Duplicate(id) => Ok(id),
        DedupOutcome::AdoptReference(id) => {
            conn.execute(
                "UPDATE transactions SET entry_reference = ?1 WHERE id = ?2 AND entry_reference IS NULL",
                params![tx.entry_reference, id],
            )?;
            Ok(id)
        }
        DedupOutcome::Update(id) | DedupOutcome::Promote(id) => {
            rewrite_row(conn, account_id, id, tx, decision)?;
            Ok(id)
        }
        DedupOutcome::Insert => {
            let idx = free_occurrence(
                conn,
                account_id,
                &decision.fingerprint,
                decision.occurrence_index,
                -1,
            )?;
            conn.execute(
                "INSERT INTO transactions
                   (account_id, status, booking_date, value_date, amount_cents, currency,
                    description, counterparty, remittance, entry_reference,
                    fingerprint, occurrence_index, source, import_batch_id, sync_run_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    account_id,
                    tx.status.as_str(),
                    tx.booking_date,
                    tx.value_date,
                    tx.amount_cents,
                    tx.currency,
                    tx.description,
                    tx.counterparty,
                    tx.remittance,
                    tx.entry_reference,
                    decision.fingerprint,
                    idx,
                    provenance.source.as_str(),
                    provenance.import_batch_id,
                    provenance.sync_run_id,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        }
    }
}

/// Overwrites the bank-provided fields of a stored row with the incoming ones.
fn rewrite_row(
    conn: &Connection,
    account_id: i64,
    id: i64,
    tx: &NewTransaction,
    decision: &Decision,
) -> Result<()> {
    let (old_fp, old_idx): (String, i64) = conn.query_row(
        "SELECT fingerprint, occurrence_index FROM transactions WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    // Same fingerprint: keep the stored index. Changed: find a free slot.
    let idx = if old_fp == decision.fingerprint {
        old_idx
    } else {
        free_occurrence(
            conn,
            account_id,
            &decision.fingerprint,
            decision.occurrence_index,
            id,
        )?
    };
    conn.execute(
        "UPDATE transactions SET
            status = ?1, booking_date = ?2, value_date = ?3, amount_cents = ?4, currency = ?5,
            description = ?6, counterparty = ?7, remittance = ?8,
            entry_reference = COALESCE(?9, entry_reference),
            fingerprint = ?10, occurrence_index = ?11
         WHERE id = ?12",
        params![
            tx.status.as_str(),
            tx.booking_date,
            tx.value_date,
            tx.amount_cents,
            tx.currency,
            tx.description,
            tx.counterparty,
            tx.remittance,
            tx.entry_reference,
            decision.fingerprint,
            idx,
            id,
        ],
    )?;
    Ok(())
}

/// Deletes pending rows of `account_id` dated within `[date_from, date_to]`
/// that the latest bank fetch no longer returned (`seen_ids` are the ids of
/// every row of that fetch). Rows the user touched (manual category, notes,
/// tags) are kept. Only bank sync calls this, never file imports.
pub fn reconcile_stale_pending(
    conn: &Connection,
    account_id: i64,
    date_from: &str,
    date_to: &str,
    seen_ids: &[i64],
) -> Result<usize> {
    let seen: HashSet<i64> = seen_ids.iter().copied().collect();
    let mut stmt = conn.prepare(
        "SELECT t.id FROM transactions t
         WHERE t.account_id = ?1 AND t.status = 'pending'
           AND COALESCE(t.booking_date, t.value_date) BETWEEN ?2 AND ?3
           AND t.category_source != 'manual'
           AND (t.notes IS NULL OR t.notes = '')
           AND NOT EXISTS (SELECT 1 FROM transaction_tags g WHERE g.transaction_id = t.id)",
    )?;
    let stale: Vec<i64> = stmt
        .query_map(params![account_id, date_from, date_to], |r| {
            r.get::<_, i64>(0)
        })?
        .collect::<Result<Vec<i64>>>()?
        .into_iter()
        .filter(|id| !seen.contains(id))
        .collect();
    for id in &stale {
        conn.execute("DELETE FROM transactions WHERE id = ?1", [id])?;
    }
    Ok(stale.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{ingest, IngestOrigin};
    use crate::test_support::*;

    fn run(f: &mut Fixture, rows: Vec<NewTransaction>) -> crate::ingest::IngestReport {
        ingest(
            &mut f.conn,
            f.own_a,
            rows,
            TxSource::Bank,
            IngestOrigin::None,
        )
        .expect("ingest")
    }

    fn rows_in_db(f: &Fixture) -> i64 {
        count(&f.conn, "SELECT count(*) FROM transactions")
    }

    #[test]
    fn fingerprint_is_stable_and_status_sensitive() {
        let a = booked("2026-03-01", -350, "Cafetería Luna");
        let b = booked("2026-03-01", -350, "CAFETERIA  LUNA!");
        assert_eq!(fingerprint(1, &a), fingerprint(1, &b));
        assert_ne!(fingerprint(1, &a), fingerprint(2, &a));
        let p = pending("2026-03-01", -350, "Cafetería Luna");
        assert_ne!(fingerprint(1, &a), fingerprint(1, &p));
        assert_eq!(fingerprint(1, &a).len(), 64);
    }

    #[test]
    fn reingesting_same_batch_is_all_duplicates() {
        let mut f = fixture();
        let batch = vec![
            booked("2026-03-01", -350, "CAFETERIA LUNA"),
            booked("2026-03-02", -4210, "SUPERMERCADOS NORTE"),
        ];
        let first = run(&mut f, batch.clone());
        assert_eq!((first.inserted, first.duplicates), (2, 0));
        let second = run(&mut f, batch);
        assert_eq!((second.inserted, second.duplicates), (0, 2));
        assert_eq!(rows_in_db(&f), 2);
    }

    #[test]
    fn identical_same_day_rows_are_kept_and_idempotent() {
        let mut f = fixture();
        let coffee = booked("2026-03-01", -350, "CAFETERIA LUNA");
        let two = vec![coffee.clone(), coffee.clone()];
        assert_eq!(run(&mut f, two.clone()).inserted, 2);
        let again = run(&mut f, two);
        assert_eq!((again.inserted, again.duplicates), (0, 2));
        assert_eq!(rows_in_db(&f), 2);

        // A later fetch shows a third identical coffee: it gets index 2.
        let three = vec![coffee.clone(), coffee.clone(), coffee];
        let later = run(&mut f, three);
        assert_eq!((later.inserted, later.duplicates), (1, 2));
        assert_eq!(rows_in_db(&f), 3);
        assert_eq!(
            count(&f.conn, "SELECT max(occurrence_index) FROM transactions"),
            2
        );
    }

    #[test]
    fn same_reference_with_changed_description_updates_in_place() {
        let mut f = fixture();
        let first = run(
            &mut f,
            vec![with_ref(booked("2026-03-01", -350, "CAFETERIA LUNA"), "R1")],
        );
        let id = first.touched_ids[0];
        let travel = category_id(&f.conn, "leisure.travel");
        f.conn
            .execute(
                "UPDATE transactions SET category_id = ?1, category_source = 'manual' WHERE id = ?2",
                params![travel, id],
            )
            .unwrap();

        let second = run(
            &mut f,
            vec![with_ref(
                booked("2026-03-01", -350, "CAFETERIA LUNA MADRID"),
                "R1",
            )],
        );
        assert_eq!(
            (second.updated, second.inserted, second.duplicates),
            (1, 0, 0)
        );
        assert_eq!(rows_in_db(&f), 1);
        assert_eq!(
            text_of(
                &f.conn,
                "SELECT description FROM transactions WHERE id = ?1",
                id
            )
            .as_deref(),
            Some("CAFETERIA LUNA MADRID")
        );
        // Manual data survives and the fingerprint was recomputed.
        assert_eq!(
            int_of(
                &f.conn,
                "SELECT category_id FROM transactions WHERE id = ?1",
                id
            ),
            Some(travel)
        );
        let expected = fingerprint(
            f.own_a,
            &with_ref(booked("2026-03-01", -350, "CAFETERIA LUNA MADRID"), "R1"),
        );
        assert_eq!(
            text_of(
                &f.conn,
                "SELECT fingerprint FROM transactions WHERE id = ?1",
                id
            ),
            Some(expected)
        );
        // Identical third time: duplicate.
        let third = run(
            &mut f,
            vec![with_ref(
                booked("2026-03-01", -350, "CAFETERIA LUNA MADRID"),
                "R1",
            )],
        );
        assert_eq!(third.duplicates, 1);
    }

    #[test]
    fn pending_is_promoted_with_other_date_and_description() {
        let mut f = fixture();
        let p = run(
            &mut f,
            vec![pending("2026-03-01", -1500, "RESERVA CAFETERIA LUNA")],
        );
        let id = p.touched_ids[0];
        let b = run(
            &mut f,
            vec![booked("2026-03-04", -1500, "CAFETERIA LUNA MADRID")],
        );
        assert_eq!((b.promoted, b.inserted), (1, 0));
        assert_eq!(rows_in_db(&f), 1);
        assert_eq!(
            text_of(&f.conn, "SELECT status FROM transactions WHERE id = ?1", id).as_deref(),
            Some("booked")
        );
        assert_eq!(
            text_of(
                &f.conn,
                "SELECT booking_date FROM transactions WHERE id = ?1",
                id
            )
            .as_deref(),
            Some("2026-03-04")
        );
        // Fetching the booked row again is now a plain duplicate.
        let again = run(
            &mut f,
            vec![booked("2026-03-04", -1500, "CAFETERIA LUNA MADRID")],
        );
        assert_eq!(again.duplicates, 1);
    }

    #[test]
    fn pending_not_promoted_when_too_far_or_amount_differs() {
        let mut f = fixture();
        run(
            &mut f,
            vec![pending("2026-03-01", -1500, "RESERVA CAFETERIA LUNA")],
        );
        // 6 days away: inserted as a separate row. 5 days would still promote.
        let far = run(&mut f, vec![booked("2026-03-07", -1500, "CAFETERIA LUNA")]);
        assert_eq!((far.promoted, far.inserted), (0, 1));
        let other_amount = run(&mut f, vec![booked("2026-03-02", -1600, "CAFETERIA LUNA")]);
        assert_eq!((other_amount.promoted, other_amount.inserted), (0, 1));
        assert_eq!(
            count(
                &f.conn,
                "SELECT count(*) FROM transactions WHERE status = 'pending'"
            ),
            1
        );

        let edge = run(
            &mut f,
            vec![booked("2026-03-06", -1500, "CAFETERIA LUNA OTRA")],
        );
        assert_eq!(edge.promoted, 1);
    }

    #[test]
    fn two_pendings_same_amount_closest_date_is_promoted() {
        let mut f = fixture();
        let p = run(
            &mut f,
            vec![
                pending("2026-03-01", -1500, "RESERVA UNO"),
                pending("2026-03-03", -1500, "RESERVA DOS"),
            ],
        );
        let (far_id, near_id) = (p.touched_ids[0], p.touched_ids[1]);
        let b = run(&mut f, vec![booked("2026-03-04", -1500, "CAFETERIA LUNA")]);
        assert_eq!(b.promoted, 1);
        assert_eq!(
            text_of(
                &f.conn,
                "SELECT status FROM transactions WHERE id = ?1",
                near_id
            )
            .as_deref(),
            Some("booked")
        );
        assert_eq!(
            text_of(
                &f.conn,
                "SELECT status FROM transactions WHERE id = ?1",
                far_id
            )
            .as_deref(),
            Some("pending")
        );
    }

    #[test]
    fn a_pending_row_is_claimed_only_once_per_batch() {
        let mut f = fixture();
        run(&mut f, vec![pending("2026-03-01", -1500, "RESERVA UNO")]);
        let b = run(
            &mut f,
            vec![
                booked("2026-03-02", -1500, "CAFETERIA LUNA"),
                booked("2026-03-02", -1500, "CAFETERIA SOL"),
            ],
        );
        assert_eq!((b.promoted, b.inserted), (1, 1));
    }

    #[test]
    fn reference_is_adopted_only_when_exactly_one_candidate() {
        let mut f = fixture();
        // One imported row without reference.
        ingest(
            &mut f.conn,
            f.own_a,
            vec![booked(
                "2026-03-02",
                -500,
                "COMPRA TARJ 1234 CAFETERIA LUNA",
            )],
            TxSource::Import,
            IngestOrigin::None,
        )
        .unwrap();
        let r = run(
            &mut f,
            vec![with_ref(booked("2026-03-02", -500, "CAFETERIA LUNA"), "R9")],
        );
        assert_eq!((r.adopted, r.inserted), (1, 0));
        assert_eq!(rows_in_db(&f), 1);
        assert_eq!(
            count(
                &f.conn,
                "SELECT count(*) FROM transactions WHERE entry_reference = 'R9'"
            ),
            1
        );
        // Description of the imported row is kept.
        assert_eq!(
            count(
                &f.conn,
                "SELECT count(*) FROM transactions WHERE description LIKE 'COMPRA TARJ%'"
            ),
            1
        );

        // Two imported twins: ambiguous, so the bank row is inserted.
        ingest(
            &mut f.conn,
            f.own_a,
            vec![
                booked("2026-04-02", -700, "PAGO PANADERIA TRIGO"),
                booked("2026-04-02", -700, "PAGO PANADERIA TRIGO"),
            ],
            TxSource::Import,
            IngestOrigin::None,
        )
        .unwrap();
        let amb = run(
            &mut f,
            vec![with_ref(
                booked("2026-04-02", -700, "PANADERIA TRIGO"),
                "R10",
            )],
        );
        assert_eq!((amb.adopted, amb.inserted), (0, 1));
    }

    #[test]
    fn same_fingerprint_adopts_a_missing_reference() {
        let mut f = fixture();
        run(&mut f, vec![booked("2026-03-02", -500, "CAFETERIA LUNA")]);
        let r = run(
            &mut f,
            vec![with_ref(booked("2026-03-02", -500, "CAFETERIA LUNA"), "R5")],
        );
        assert_eq!((r.adopted, r.inserted, r.duplicates), (1, 0, 0));
        assert_eq!(
            count(
                &f.conn,
                "SELECT count(*) FROM transactions WHERE entry_reference = 'R5'"
            ),
            1
        );
    }

    #[test]
    fn different_references_never_collapse_into_one_row() {
        let mut f = fixture();
        run(
            &mut f,
            vec![with_ref(booked("2026-03-02", -500, "CAFETERIA LUNA"), "R1")],
        );
        let r = run(
            &mut f,
            vec![with_ref(booked("2026-03-02", -500, "CAFETERIA LUNA"), "R2")],
        );
        assert_eq!((r.inserted, r.duplicates), (1, 0));
        assert_eq!(rows_in_db(&f), 2);
    }

    #[test]
    fn reconcile_deletes_only_unseen_unedited_pending_rows_in_range() {
        let mut f = fixture();
        let created = run(
            &mut f,
            vec![
                pending("2026-03-02", -100, "VISTO"),
                pending("2026-03-03", -200, "NO VISTO"),
                pending("2026-03-04", -300, "CON NOTA"),
                pending("2026-03-05", -400, "CON ETIQUETA"),
                pending("2026-03-06", -500, "CATEGORIA MANUAL"),
                pending("2026-02-01", -600, "FUERA DE RANGO"),
                pending("2026-03-07", -700, "ES BOOKED LUEGO"),
            ],
        );
        let ids = created.touched_ids.clone();
        f.conn
            .execute(
                "UPDATE transactions SET notes = 'mi nota' WHERE id = ?1",
                [ids[2]],
            )
            .unwrap();
        f.conn
            .execute("INSERT INTO tags (name) VALUES ('viaje')", [])
            .unwrap();
        f.conn
            .execute(
                "INSERT INTO transaction_tags VALUES (?1, last_insert_rowid())",
                [ids[3]],
            )
            .unwrap();
        f.conn
            .execute(
                "UPDATE transactions SET category_source = 'manual' WHERE id = ?1",
                [ids[4]],
            )
            .unwrap();
        f.conn
            .execute("UPDATE transactions SET status = 'booked', booking_date = '2026-03-07' WHERE id = ?1", [ids[6]])
            .unwrap();

        let deleted =
            reconcile_stale_pending(&f.conn, f.own_a, "2026-03-01", "2026-03-31", &[ids[0]])
                .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(rows_in_db(&f), 6);
        assert_eq!(
            count(
                &f.conn,
                &format!("SELECT count(*) FROM transactions WHERE id = {}", ids[1])
            ),
            0
        );
    }
}
