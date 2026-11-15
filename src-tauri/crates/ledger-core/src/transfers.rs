//! Detection of transfers between the user's own accounts.
//!
//! An outflow and an inflow with exactly opposite amounts on two different own
//! accounts, booked within 2 days of each other, are linked as a pair and both
//! get the "Transferencias internas" category, so they never count as income or
//! expense. Matching is greedy and deterministic (see `detect_internal_transfers`).
//! Day distances are computed inside SQLite; Rust only compares whole numbers.

use rusqlite::{params, Connection, OptionalExtension, Result};

use crate::rules;

/// Maximum difference in booking date, in days.
const MAX_DAYS_APART: i64 = 2;

/// SQL expression: whole days between booking date `b.booking_date` and parameter ?N.
fn days_apart(param: &str) -> String {
    format!("CAST(round(abs(julianday(b.booking_date) - julianday({param}))) AS INTEGER)")
}

fn transfer_category_id(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT id FROM categories WHERE key = 'transfer.internal'",
        [],
        |r| r.get(0),
    )
}

/// What we need to know about a candidate.
struct Candidate {
    account_id: i64,
    booking_date: String,
    amount_cents: i64,
    currency: String,
}

/// The candidate row if it can still be paired: booked, own account, non-zero
/// amount, not linked yet, and not manually categorized (unless the manual
/// category is already the internal-transfer one).
fn eligible_candidate(conn: &Connection, id: i64, transfer_cat: i64) -> Result<Option<Candidate>> {
    conn.query_row(
        "SELECT t.account_id, t.booking_date, t.amount_cents, t.currency
         FROM transactions t JOIN accounts a ON a.id = t.account_id
         WHERE t.id = ?1 AND t.status = 'booked' AND t.booking_date IS NOT NULL
           AND t.transfer_peer_id IS NULL AND a.is_own = 1 AND t.amount_cents != 0
           AND (t.category_source != 'manual' OR t.category_id = ?2)",
        params![id, transfer_cat],
        |r| {
            Ok(Candidate {
                account_id: r.get(0)?,
                booking_date: r.get(1)?,
                amount_cents: r.get(2)?,
                currency: r.get(3)?,
            })
        },
    )
    .optional()
}

/// Pairs each candidate with the best partner. Candidates are processed ordered
/// by (booking date, id); for each one the partner is the row with the smallest
/// day distance, then the oldest (booking date, id). A row is paired at most
/// once. Returns the pairs as (candidate id, partner id).
pub fn detect_internal_transfers(
    conn: &Connection,
    candidate_ids: &[i64],
) -> Result<Vec<(i64, i64)>> {
    let transfer_cat = transfer_category_id(conn)?;

    // Order candidates by (booking date, id), dropping duplicates and ineligible rows.
    let mut ordered: Vec<(String, i64)> = Vec::new();
    for &id in candidate_ids {
        if let Some(c) = eligible_candidate(conn, id, transfer_cat)? {
            ordered.push((c.booking_date, id));
        }
    }
    ordered.sort();
    ordered.dedup();

    let partner_sql = format!(
        "SELECT b.id
         FROM transactions b JOIN accounts ab ON ab.id = b.account_id
         WHERE b.status = 'booked' AND b.booking_date IS NOT NULL
           AND b.transfer_peer_id IS NULL AND ab.is_own = 1
           AND b.account_id != ?1 AND b.currency = ?2 AND b.amount_cents = ?3
           AND (b.category_source != 'manual' OR b.category_id = ?4)
           AND {dist} <= {MAX_DAYS_APART}
         ORDER BY {dist}, b.booking_date, b.id
         LIMIT 1",
        dist = days_apart("?5")
    );

    let mut pairs = Vec::new();
    for (_, id) in ordered {
        // Re-check: an earlier iteration may have linked this row as a partner.
        let Some(c) = eligible_candidate(conn, id, transfer_cat)? else {
            continue;
        };
        let Some(opposite) = c.amount_cents.checked_neg() else {
            continue;
        };
        let partner: Option<i64> = conn
            .query_row(
                &partner_sql,
                params![
                    c.account_id,
                    c.currency,
                    opposite,
                    transfer_cat,
                    c.booking_date
                ],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(partner_id) = partner {
            link(conn, id, partner_id, transfer_cat)?;
            pairs.push((id, partner_id));
        }
    }
    Ok(pairs)
}

fn link(conn: &Connection, a: i64, b: i64, transfer_cat: i64) -> Result<()> {
    for (id, peer) in [(a, b), (b, a)] {
        conn.execute(
            "UPDATE transactions
             SET transfer_peer_id = ?1, category_id = ?2, category_source = 'transfer', rule_id = NULL
             WHERE id = ?3",
            params![peer, transfer_cat, id],
        )?;
    }
    Ok(())
}

/// Undoes a link (given either side). Both rows go back to "no category" and are
/// then run through the rules again. Returns the ids of the rows that were reset.
pub fn unlink_transfer(conn: &Connection, tx_id: i64) -> Result<Vec<i64>> {
    let peer: Option<i64> = conn
        .query_row(
            "SELECT transfer_peer_id FROM transactions WHERE id = ?1",
            [tx_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let mut ids = vec![tx_id];
    ids.extend(peer);
    for id in &ids {
        conn.execute(
            "UPDATE transactions
             SET transfer_peer_id = NULL,
                 category_id = CASE WHEN category_source = 'transfer' THEN NULL ELSE category_id END,
                 category_source = CASE WHEN category_source = 'transfer' THEN 'none' ELSE category_source END,
                 rule_id = NULL
             WHERE id = ?1",
            [id],
        )?;
    }
    rules::categorize(conn, &ids)?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn source_of(f: &Fixture, id: i64) -> String {
        text_of(
            &f.conn,
            "SELECT category_source FROM transactions WHERE id = ?1",
            id,
        )
        .unwrap()
    }

    #[test]
    fn basic_pair_is_linked_both_ways() {
        let f = fixture();
        let out = raw_booked(&f.conn, f.own_a, "2026-03-01", -10_000, "TRASPASO");
        let inn = raw_booked(&f.conn, f.own_b, "2026-03-02", 10_000, "TRASPASO");
        let pairs = detect_internal_transfers(&f.conn, &[out]).unwrap();
        assert_eq!(pairs, vec![(out, inn)]);
        let cat = category_id(&f.conn, "transfer.internal");
        for (id, peer) in [(out, inn), (inn, out)] {
            assert_eq!(
                int_of(
                    &f.conn,
                    "SELECT transfer_peer_id FROM transactions WHERE id = ?1",
                    id
                ),
                Some(peer)
            );
            assert_eq!(
                int_of(
                    &f.conn,
                    "SELECT category_id FROM transactions WHERE id = ?1",
                    id
                ),
                Some(cat)
            );
            assert_eq!(source_of(&f, id), "transfer");
        }
    }

    #[test]
    fn window_is_two_days_inclusive() {
        let f = fixture();
        let out = raw_booked(&f.conn, f.own_a, "2026-03-10", -5_000, "X");
        raw_booked(&f.conn, f.own_b, "2026-03-13", 5_000, "X"); // 3 days
        assert!(detect_internal_transfers(&f.conn, &[out])
            .unwrap()
            .is_empty());
        let near = raw_booked(&f.conn, f.own_b, "2026-03-12", 5_000, "X"); // 2 days
        assert_eq!(
            detect_internal_transfers(&f.conn, &[out]).unwrap(),
            vec![(out, near)]
        );
    }

    #[test]
    fn tie_break_closest_date_then_oldest() {
        let f = fixture();
        let out = raw_booked(&f.conn, f.own_a, "2026-03-10", -5_000, "X");
        raw_booked(&f.conn, f.own_b, "2026-03-12", 5_000, "X"); // distance 2
        let before = raw_booked(&f.conn, f.own_b, "2026-03-09", 5_000, "X"); // distance 1, older
        let after = raw_booked(&f.conn, f.own_b, "2026-03-11", 5_000, "X"); // distance 1, newer
        let pairs = detect_internal_transfers(&f.conn, &[out]).unwrap();
        assert_eq!(pairs, vec![(out, before)]);
        assert_eq!(source_of(&f, after), "none");
    }

    #[test]
    fn a_row_is_never_matched_twice_and_oldest_candidate_goes_first() {
        let f = fixture();
        let late = raw_booked(&f.conn, f.own_a, "2026-03-02", -5_000, "X");
        let early = raw_booked(&f.conn, f.own_a, "2026-03-01", -5_000, "X");
        let inn = raw_booked(&f.conn, f.own_b, "2026-03-02", 5_000, "X");
        // Passed in "wrong" order on purpose: candidates are sorted by (date, id).
        let pairs = detect_internal_transfers(&f.conn, &[late, early]).unwrap();
        assert_eq!(pairs, vec![(early, inn)]);
        assert_eq!(source_of(&f, late), "none");
    }

    #[test]
    fn same_account_non_own_zero_pending_and_other_currency_are_never_paired() {
        let f = fixture();
        let out = raw_booked(&f.conn, f.own_a, "2026-03-01", -5_000, "X");
        raw_booked(&f.conn, f.own_a, "2026-03-01", 5_000, "X"); // same account
        raw_booked(&f.conn, f.external, "2026-03-01", 5_000, "X"); // not own
        let usd = raw_booked(&f.conn, f.own_b, "2026-03-01", 5_000, "X");
        f.conn
            .execute(
                "UPDATE transactions SET currency = 'USD' WHERE id = ?1",
                [usd],
            )
            .unwrap();
        let pend = raw_booked(&f.conn, f.own_b, "2026-03-01", 5_000, "X");
        f.conn
            .execute(
                "UPDATE transactions SET status = 'pending' WHERE id = ?1",
                [pend],
            )
            .unwrap();
        assert!(detect_internal_transfers(&f.conn, &[out])
            .unwrap()
            .is_empty());

        let z1 = raw_booked(&f.conn, f.own_a, "2026-03-05", 0, "X");
        raw_booked(&f.conn, f.own_b, "2026-03-05", 0, "X");
        assert!(detect_internal_transfers(&f.conn, &[z1])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn manual_category_is_respected_unless_it_is_the_transfer_category() {
        let f = fixture();
        let groceries = category_id(&f.conn, "food.groceries");
        let transfer = category_id(&f.conn, "transfer.internal");
        let out = raw_booked(&f.conn, f.own_a, "2026-03-01", -5_000, "X");
        let inn = raw_booked(&f.conn, f.own_b, "2026-03-01", 5_000, "X");
        f.conn
            .execute("UPDATE transactions SET category_id = ?1, category_source = 'manual' WHERE id = ?2", [groceries, inn])
            .unwrap();
        assert!(detect_internal_transfers(&f.conn, &[out])
            .unwrap()
            .is_empty());
        assert!(detect_internal_transfers(&f.conn, &[inn])
            .unwrap()
            .is_empty());

        f.conn
            .execute(
                "UPDATE transactions SET category_id = ?1 WHERE id = ?2",
                [transfer, inn],
            )
            .unwrap();
        assert_eq!(
            detect_internal_transfers(&f.conn, &[out]).unwrap(),
            vec![(out, inn)]
        );
        assert_eq!(source_of(&f, inn), "transfer");
    }

    #[test]
    fn unlink_resets_both_sides_and_recategorizes() {
        let f = fixture();
        let out = raw_booked(&f.conn, f.own_a, "2026-03-01", -5_000, "TRASPASO AHORRO");
        let inn = raw_booked(&f.conn, f.own_b, "2026-03-01", 5_000, "TRASPASO AHORRO");
        detect_internal_transfers(&f.conn, &[out]).unwrap();
        let savings = category_id(&f.conn, "finance.savings_investment");
        rules::create_manual_rule(
            &f.conn,
            &rules::NewRule::contains("traspaso ahorro", savings),
        )
        .unwrap();

        let reset = unlink_transfer(&f.conn, inn).unwrap();
        assert_eq!(reset.len(), 2);
        for id in [out, inn] {
            assert_eq!(
                int_of(
                    &f.conn,
                    "SELECT transfer_peer_id FROM transactions WHERE id = ?1",
                    id
                ),
                None
            );
            assert_eq!(source_of(&f, id), "rule");
            assert_eq!(
                int_of(
                    &f.conn,
                    "SELECT category_id FROM transactions WHERE id = ?1",
                    id
                ),
                Some(savings)
            );
        }
    }
}
