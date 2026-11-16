//! Tests of the Resumen / Movimientos layer. Fictitious data only.

use rusqlite::params;

use super::period::{periods, twelve_months, parse_month};
use super::transactions::{Direction, StatusFilter};
use super::*;
use crate::test_support::*;

/// Inserts a booked row with a category set by hand.
fn tx(f: &Fixture, account: i64, date: &str, cents: i64, text: &str, cat: Option<&str>) -> i64 {
    let id = raw_booked(&f.conn, account, date, cents, text);
    if let Some(key) = cat {
        let cid = category_id(&f.conn, key);
        f.conn
            .execute(
                "UPDATE transactions SET category_id = ?1, category_source = 'manual' WHERE id = ?2",
                params![cid, id],
            )
            .unwrap();
    }
    id
}

fn pending_tx(f: &Fixture, account: i64, date: &str, cents: i64, text: &str) -> i64 {
    let id = raw_booked(&f.conn, account, date, cents, text);
    f.conn
        .execute(
            "UPDATE transactions SET status = 'pending', value_date = booking_date, booking_date = NULL WHERE id = ?1",
            [id],
        )
        .unwrap();
    id
}

fn filter() -> TxFilter {
    TxFilter::default()
}

fn ids(page: &TxPage) -> Vec<i64> {
    page.rows.iter().map(|r| r.id).collect()
}

// ---- period logic ----

#[test]
fn current_month_compares_the_same_day_range() {
    let p = periods("2026-10", "2026-10-15").unwrap();
    assert!(p.is_current);
    assert_eq!(p.through_day, Some(15));
    assert_eq!(period::iso(p.from), "2026-10-01");
    assert_eq!(period::iso(p.to), "2026-10-15");
    assert_eq!(period::iso(p.prev_from), "2026-09-01");
    assert_eq!(period::iso(p.prev_to), "2026-09-15");
}

#[test]
fn comparison_day_is_clamped_to_the_previous_month_length() {
    // 31 Mar against February (28 days, then 29 in a leap year).
    let p = periods("2026-03", "2026-03-31").unwrap();
    assert_eq!(period::iso(p.prev_to), "2026-02-28");
    let leap = periods("2028-03", "2028-03-31").unwrap();
    assert_eq!(period::iso(leap.prev_to), "2028-02-29");
    // 31 Oct against September (30 days).
    assert_eq!(period::iso(periods("2026-10", "2026-10-31").unwrap().prev_to), "2026-09-30");
    // 29 Mar in a non-leap year is clamped too; 28 Mar is not.
    assert_eq!(period::iso(periods("2026-03", "2026-03-29").unwrap().prev_to), "2026-02-28");
    assert_eq!(period::iso(periods("2026-03", "2026-03-28").unwrap().prev_to), "2026-02-28");
}

#[test]
fn january_compares_against_december_of_the_previous_year() {
    let p = periods("2027-01", "2027-01-10").unwrap();
    assert_eq!(period::iso(p.prev_from), "2026-12-01");
    assert_eq!(period::iso(p.prev_to), "2026-12-10");
}

#[test]
fn past_month_compares_full_months() {
    let p = periods("2026-03", "2026-10-15").unwrap();
    assert!(!p.is_current);
    assert_eq!(p.through_day, None);
    assert_eq!(period::iso(p.to), "2026-03-31");
    assert_eq!(period::iso(p.prev_from), "2026-02-01");
    assert_eq!(period::iso(p.prev_to), "2026-02-28");
    let feb_leap = periods("2028-02", "2028-06-01").unwrap();
    assert_eq!(period::iso(feb_leap.to), "2028-02-29");
}

#[test]
fn bad_month_or_today_is_an_invalid_input() {
    for bad in ["2026-13", "2026-1", "26-10", "2026/10", "abcd-ef", "", "2026-10-01"] {
        assert!(matches!(periods(bad, "2026-10-15"), Err(LedgerError::InvalidInput(_))), "{bad}");
    }
    for bad in ["2026-02-30", "2026-10", "yesterday", "2026-10-1"] {
        assert!(matches!(periods("2026-10", bad), Err(LedgerError::InvalidInput(_))), "{bad}");
    }
}

#[test]
fn twelve_months_end_at_the_selected_one() {
    let months: Vec<String> =
        twelve_months(parse_month("2026-02").unwrap()).into_iter().map(period::month_key).collect();
    assert_eq!(months.len(), 12);
    assert_eq!(months.first().unwrap(), "2025-03");
    assert_eq!(months.last().unwrap(), "2026-02");
}

// ---- dashboard ----

fn sample(f: &Fixture) {
    // October 2026, own account A.
    tx(f, f.own_a, "2026-10-01", 250_000, "Nomina demo", Some("income.salary"));
    tx(f, f.own_a, "2026-10-03", -12_000, "Mercado Demo", Some("food.groceries"));
    tx(f, f.own_a, "2026-10-05", -3_050, "Cafeteria Demo", Some("food.cafes"));
    tx(f, f.own_a, "2026-10-07", -8_000, "Gasolinera Demo", Some("transport.fuel"));
    tx(f, f.own_a, "2026-10-09", -500, "Sin clasificar demo", None);
    // Manual transfer category: neither income nor expense.
    tx(f, f.own_a, "2026-10-10", -50_000, "Mover a ahorro", Some("transfer.internal"));
    tx(f, f.own_b, "2026-10-10", 50_000, "Mover a ahorro", Some("transfer.internal"));
    // A refund in an expense category counts as income (sign decides).
    tx(f, f.own_a, "2026-10-12", 2_000, "Devolucion demo", Some("food.groceries"));
    // Outside the current day range (after 15 Oct).
    tx(f, f.own_a, "2026-10-20", -99_900, "Futuro demo", Some("shopping"));
    // September, same day range and later.
    tx(f, f.own_a, "2026-09-02", -10_000, "Mercado Demo", Some("food.groceries"));
    tx(f, f.own_a, "2026-09-14", -5_000, "Cine demo", Some("leisure"));
    tx(f, f.own_a, "2026-09-28", -70_000, "Fuera de rango", Some("housing"));
    // Not the user's account: ignored everywhere.
    tx(f, f.external, "2026-10-04", -777_700, "Cuenta ajena", Some("shopping"));
}

#[test]
fn totals_exclude_transfers_and_follow_the_sign() {
    let f = fixture();
    sample(&f);
    let d = dashboard(&f.conn, "2026-10", "2026-10-15").unwrap();
    assert!(d.is_current_month);
    assert_eq!(d.through_day, Some(15));
    assert_eq!(d.totals.income_cents, 250_000 + 2_000);
    assert_eq!(d.totals.expense_cents, 12_000 + 3_050 + 8_000 + 500);
    assert_eq!(d.totals.balance_cents, d.totals.income_cents - d.totals.expense_cents);
    // Previous: 1..15 Sep only.
    assert_eq!(d.previous.expense_cents, 15_000);
    assert_eq!(d.previous.income_cents, 0);
    assert_eq!(d.previous_label_range.from, "2026-09-01");
    assert_eq!(d.previous_label_range.to, "2026-09-15");
}

#[test]
fn past_month_uses_the_whole_month() {
    let f = fixture();
    sample(&f);
    let d = dashboard(&f.conn, "2026-10", "2026-11-20").unwrap();
    assert!(!d.is_current_month);
    assert_eq!(d.through_day, None);
    assert_eq!(d.totals.expense_cents, 12_000 + 3_050 + 8_000 + 500 + 99_900);
    assert_eq!(d.previous.expense_cents, 85_000);
}

#[test]
fn auto_linked_transfers_are_excluded_too() {
    let f = fixture();
    let out = tx(&f, f.own_a, "2026-10-02", -10_000, "TRASPASO", None);
    let inn = tx(&f, f.own_b, "2026-10-02", 10_000, "TRASPASO", None);
    crate::transfers::detect_internal_transfers(&f.conn, &[out]).unwrap();
    let d = dashboard(&f.conn, "2026-10", "2026-10-31").unwrap();
    assert_eq!((d.totals.income_cents, d.totals.expense_cents), (0, 0));
    let page = list_transactions(&f.conn, &filter()).unwrap();
    assert_eq!(page.summary.transfer_count, 2);
    assert_eq!((page.summary.income_cents, page.summary.expense_cents), (0, 0));
    let row = page.rows.iter().find(|r| r.id == inn).unwrap();
    assert!(row.is_transfer);
    assert_eq!(row.transfer_peer_id, Some(out));
    assert_eq!(row.transfer_peer_account_name.as_deref(), Some("Cuenta demo"));
}

#[test]
fn pending_rows_count_in_the_month_of_their_effective_date() {
    let f = fixture();
    pending_tx(&f, f.own_a, "2026-10-14", -4_000, "Pendiente demo");
    let d = dashboard(&f.conn, "2026-10", "2026-10-15").unwrap();
    assert_eq!(d.totals.expense_cents, 4_000);
    assert_eq!(d.recent[0].status, "pending");
    assert_eq!(d.recent[0].date, "2026-10-14");
    assert_eq!(d.recent[0].booking_date, None);
    assert_eq!(list_transactions(&f.conn, &filter()).unwrap().summary.pending_count, 1);
}

#[test]
fn categories_group_by_top_level_and_keep_uncategorized_apart() {
    let f = fixture();
    sample(&f);
    let d = dashboard(&f.conn, "2026-10", "2026-10-15").unwrap();
    let got: Vec<(&str, i64, i64)> =
        d.categories.iter().map(|c| (c.name.as_str(), c.expense_cents, c.tx_count)).collect();
    // Food = groceries 12_000 + cafes 3_050 (the refund is a positive amount and not a spend).
    assert_eq!(
        got,
        vec![("Alimentación", 15_050, 2), ("Transporte", 8_000, 1), ("Sin categoría", 500, 1)]
    );
    assert_eq!(d.categories[0].key.as_deref(), Some("food"));
    assert_eq!(d.categories[2].category_id, None);
    assert_eq!(d.categories[2].key, None);
}

#[test]
fn evolution_has_twelve_months_and_the_last_matches_the_headline() {
    let f = fixture();
    sample(&f);
    let d = dashboard(&f.conn, "2026-10", "2026-10-15").unwrap();
    assert_eq!(d.evolution.len(), 12);
    assert_eq!(d.evolution[0].month, "2025-11");
    let last = d.evolution.last().unwrap();
    assert_eq!(last.month, "2026-10");
    assert_eq!(last.totals, d.totals);
    let sep = d.evolution.iter().find(|m| m.month == "2026-09").unwrap();
    assert_eq!(sep.totals.expense_cents, 85_000); // full month
    let aug = d.evolution.iter().find(|m| m.month == "2026-08").unwrap();
    assert_eq!(aug.totals, PeriodTotals { income_cents: 0, expense_cents: 0, balance_cents: 0 });
    // Past month: full month in the last point.
    let past = dashboard(&f.conn, "2026-10", "2026-12-01").unwrap();
    assert_eq!(past.evolution.last().unwrap().totals.expense_cents, past.totals.expense_cents);
}

#[test]
fn accounts_report_the_bank_balance_not_a_transaction_sum() {
    let f = fixture();
    tx(&f, f.own_a, "2026-10-01", -1_000, "Algo demo", None);
    f.conn
        .execute(
            "INSERT INTO account_balances (account_id, amount_cents, currency, balance_type, reference_date, fetched_at)
             VALUES (?1, 123456, 'EUR', 'closingBooked', '2026-10-14', '2026-10-14T08:00:00Z')",
            [f.own_a],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO account_sync_state (account_id, last_synced_to, last_success_at)
             VALUES (?1, '2026-10-14', '2026-10-14T08:00:00Z')",
            [f.own_a],
        )
        .unwrap();
    let d = dashboard(&f.conn, "2026-10", "2026-10-15").unwrap();
    // Own accounts only: the external one is not listed.
    assert_eq!(d.accounts.len(), 2);
    let a = &d.accounts[0];
    assert_eq!(a.balance_cents, Some(123_456));
    assert_eq!(a.balance_date.as_deref(), Some("2026-10-14"));
    assert_eq!(a.last_synced_at.as_deref(), Some("2026-10-14T08:00:00Z"));
    assert_eq!(d.accounts[1].balance_cents, None);
}

#[test]
fn recent_rows_and_count_cover_the_calendar_month() {
    let f = fixture();
    for day in 1..=8 {
        tx(&f, f.own_a, &format!("2026-10-{day:02}"), -100 * i64::from(day), "Fila demo", None);
    }
    tx(&f, f.own_a, "2026-09-30", -100, "Otro mes", None);
    let d = dashboard(&f.conn, "2026-10", "2026-10-15").unwrap();
    assert_eq!(d.month_tx_count, 8);
    assert_eq!(d.recent.len(), 6);
    assert_eq!(d.recent[0].date, "2026-10-08");
    assert_eq!(d.recent[5].date, "2026-10-03");
}

#[test]
fn effective_date_falls_back_to_value_date() {
    let f = fixture();
    let id = pending_tx(&f, f.own_a, "2026-10-03", -100, "Solo valor");
    let page = list_transactions(&f.conn, &TxFilter { from: Some("2026-10-01".into()), to: Some("2026-10-03".into()), ..filter() }).unwrap();
    assert_eq!(ids(&page), vec![id]);
}

#[test]
fn dashboard_json_is_camel_case() {
    let f = fixture();
    sample(&f);
    let v = serde_json::to_value(dashboard(&f.conn, "2026-10", "2026-10-15").unwrap()).unwrap();
    for key in [
        "isCurrentMonth", "throughDay", "previousLabelRange", "monthTxCount", "evolution", "categories", "accounts", "recent",
    ] {
        assert!(v.get(key).is_some(), "{key}");
    }
    assert!(v["totals"].get("incomeCents").is_some());
    assert!(v["evolution"][0].get("month").is_some() && v["evolution"][0].get("balanceCents").is_some());
    assert!(v["accounts"][0].get("displaySuffix").is_some());
    assert!(v["recent"][0].get("categorySource").is_some());
    assert!(v["recent"][0]["tags"].is_array());
}

// ---- transaction list ----

#[test]
fn list_is_newest_first_and_paginates() {
    let f = fixture();
    let mut all = Vec::new();
    for day in 1..=7 {
        all.push(tx(&f, f.own_a, &format!("2026-10-{day:02}"), -100, "Fila demo", None));
    }
    let same_day = tx(&f, f.own_a, "2026-10-07", -100, "Mismo dia", None);
    let first = list_transactions(&f.conn, &TxFilter { limit: Some(3), ..filter() }).unwrap();
    assert_eq!(first.total, 8);
    assert_eq!(ids(&first), vec![same_day, all[6], all[5]]);
    let second = list_transactions(&f.conn, &TxFilter { limit: Some(3), offset: Some(3), ..filter() }).unwrap();
    assert_eq!(ids(&second), vec![all[4], all[3], all[2]]);
    let tail = list_transactions(&f.conn, &TxFilter { limit: Some(3), offset: Some(6), ..filter() }).unwrap();
    assert_eq!(ids(&tail), vec![all[1], all[0]]);
    // Summary covers the whole filter, not the page.
    assert_eq!(first.summary.expense_cents, 800);
}

#[test]
fn each_filter_field_narrows_the_list() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", 100_000, "Nomina demo", Some("income.salary"));
    let b = tx(&f, f.own_a, "2026-10-05", -2_000, "Mercado demo", Some("food.groceries"));
    let c = tx(&f, f.own_b, "2026-10-09", -9_000, "Cafe demo", Some("food.cafes"));
    let d = tx(&f, f.own_b, "2026-10-12", -300, "Sin clase demo", None);
    let p = pending_tx(&f, f.own_a, "2026-10-13", -700, "Pendiente demo");
    let run = |flt: TxFilter| ids(&list_transactions(&f.conn, &flt).unwrap());
    let sorted = |mut v: Vec<i64>| {
        v.sort_unstable();
        v
    };

    assert_eq!(run(TxFilter { from: Some("2026-10-05".into()), to: Some("2026-10-12".into()), ..filter() }), vec![d, c, b]);
    assert_eq!(sorted(run(TxFilter { account_id: Some(f.own_b), ..filter() })), sorted(vec![c, d]));
    // A group matches its children.
    let food = category_id(&f.conn, "food");
    assert_eq!(sorted(run(TxFilter { category_id: Some(food), ..filter() })), sorted(vec![b, c]));
    let cafes = category_id(&f.conn, "food.cafes");
    assert_eq!(run(TxFilter { category_id: Some(cafes), ..filter() }), vec![c]);
    assert_eq!(sorted(run(TxFilter { uncategorized: Some(true), ..filter() })), sorted(vec![d, p]));
    assert_eq!(sorted(run(TxFilter { min_cents: Some(2_000), max_cents: Some(9_000), ..filter() })), sorted(vec![b, c]));
    assert_eq!(run(TxFilter { direction: Some(Direction::In), ..filter() }), vec![a]);
    assert_eq!(run(TxFilter { direction: Some(Direction::Out), limit: Some(500), ..filter() }).len(), 4);
    assert_eq!(run(TxFilter { status: Some(StatusFilter::Pending), ..filter() }), vec![p]);
    assert_eq!(run(TxFilter { status: Some(StatusFilter::Booked), ..filter() }).len(), 4);
    // Combined.
    assert_eq!(
        run(TxFilter {
            account_id: Some(f.own_a),
            direction: Some(Direction::Out),
            status: Some(StatusFilter::Booked),
            ..filter()
        }),
        vec![b]
    );
}

#[test]
fn summary_has_income_expense_net_transfers_and_pending() {
    let f = fixture();
    tx(&f, f.own_a, "2026-10-01", 100_000, "Nomina demo", Some("income.salary"));
    tx(&f, f.own_a, "2026-10-02", -30_000, "Alquiler demo", Some("housing"));
    tx(&f, f.own_a, "2026-10-03", -5_000, "Mover", Some("transfer.internal"));
    pending_tx(&f, f.own_a, "2026-10-04", -1_000, "Pendiente demo");
    tx(&f, f.external, "2026-10-04", -1_234, "Ajena", None);
    let page = list_transactions(&f.conn, &filter()).unwrap();
    assert_eq!(page.total, 4);
    assert_eq!(
        page.summary,
        transactions::TxSummary {
            income_cents: 100_000,
            expense_cents: 31_000,
            net_cents: 69_000,
            transfer_count: 1,
            pending_count: 1
        }
    );
}

#[test]
fn text_search_ignores_case_and_accents_across_fields() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -300, "Cafetería Luna", None);
    let b = tx(&f, f.own_a, "2026-10-02", -400, "Pago tarjeta", None);
    f.conn.execute("UPDATE transactions SET counterparty = 'PANADERÍA SOL' WHERE id = ?1", [b]).unwrap();
    let c = tx(&f, f.own_a, "2026-10-03", -500, "Otro", None);
    set_notes(&f.conn, c, Some("compra para el cumpleaños")).unwrap();
    let run = |t: &str| ids(&list_transactions(&f.conn, &TxFilter { text: Some(t.into()), ..filter() }).unwrap());
    assert_eq!(run("cafeteria"), vec![a]);
    assert_eq!(run("CAFETERÍA luna"), vec![a]);
    assert_eq!(run("panaderia"), vec![b]);
    assert_eq!(run("cumpleanos"), vec![c]);
    assert!(run("zzz").is_empty());
    // Words may come in any order; `%` and quotes are plain text, never SQL.
    assert_eq!(run("luna cafe"), vec![a]);
    assert!(run("'; DROP TABLE transactions; --").is_empty());
    // Only punctuation: no text filter at all.
    assert_eq!(run("  ...  ").len(), 3);
    // Combined with another filter and counted as a total.
    let page = list_transactions(&f.conn, &TxFilter { text: Some("luna".into()), min_cents: Some(450), ..filter() }).unwrap();
    assert_eq!(page.total, 0);
}

#[test]
fn list_rejects_bad_input() {
    let f = fixture();
    let bad = |flt: TxFilter| matches!(list_transactions(&f.conn, &flt), Err(LedgerError::InvalidInput(_)));
    assert!(bad(TxFilter { limit: Some(501), ..filter() }));
    assert!(bad(TxFilter { limit: Some(0), ..filter() }));
    assert!(bad(TxFilter { offset: Some(-1), ..filter() }));
    assert!(bad(TxFilter { from: Some("01/10/2026".into()), ..filter() }));
    assert!(bad(TxFilter { to: Some("2026-02-30".into()), ..filter() }));
    assert!(bad(TxFilter { min_cents: Some(-1), ..filter() }));
    assert!(bad(TxFilter { text: Some("x".repeat(MAX_TEXT_CHARS + 1)), ..filter() }));
    assert!(list_transactions(&f.conn, &TxFilter { limit: Some(500), ..filter() }).is_ok());
}

#[test]
fn filter_deserializes_from_camel_case() {
    let flt: TxFilter = serde_json::from_str(
        r#"{"accountId":3,"categoryId":4,"minCents":10,"direction":"out","status":"pending","limit":50}"#,
    )
    .unwrap();
    assert_eq!((flt.account_id, flt.category_id, flt.min_cents, flt.limit), (Some(3), Some(4), Some(10), Some(50)));
    assert_eq!(flt.direction, Some(Direction::Out));
    assert_eq!(flt.status, Some(StatusFilter::Pending));
    assert!(serde_json::from_str::<TxFilter>("{}").is_ok());
}

#[test]
fn categories_come_as_groups_followed_by_their_children() {
    let f = fixture();
    let list = categories(&f.conn).unwrap();
    assert_eq!(list.first().unwrap().key.as_deref(), Some("income"));
    assert_eq!(list[1].parent_id, Some(list[0].id));
    let food_pos = list.iter().position(|c| c.key.as_deref() == Some("food")).unwrap();
    assert!(list[food_pos].parent_id.is_none());
    assert_eq!(list[food_pos + 1].parent_id, Some(list[food_pos].id));
    // Every child sits right after its group or a sibling.
    let transfer = list.iter().find(|c| c.key.as_deref() == Some("transfer.internal")).unwrap();
    assert_eq!(transfer.kind, "transfer");
}

// ---- mutations ----

#[test]
fn set_category_learns_and_recategorizes_similar_rows() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -1_000, "Cafeteria Luna 1234", None);
    let b = tx(&f, f.own_a, "2026-10-02", -1_200, "CAFETERIA LUNA", None);
    let other = tx(&f, f.own_a, "2026-10-03", -900, "Gasolinera demo", None);
    let cafes = category_id(&f.conn, "food.cafes");
    let res = set_category(&f.conn, a, cafes, false).unwrap();
    assert_eq!(res.row.category_id, Some(cafes));
    assert_eq!(res.row.category_source, "manual");
    assert_eq!(res.row.parent_category_name.as_deref(), Some("Alimentación"));
    assert_eq!(res.recategorized, 1);
    assert_eq!(res.rule_id, None);
    let rows = rows_by_ids_for_test(&f, &[b, other]);
    assert_eq!(rows[0].category_id, Some(cafes));
    assert_eq!(rows[1].category_id, None);
    assert_eq!(count(&f.conn, "SELECT count(*) FROM rules WHERE origin = 'learned'"), 1);
}

fn rows_by_ids_for_test(f: &Fixture, ids: &[i64]) -> Vec<TxRow> {
    transactions::rows_by_ids(&f.conn, ids).unwrap()
}

#[test]
fn set_category_with_create_rule_adds_a_manual_rule() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -1_000, "Mercado Sol 55", None);
    let income_like = tx(&f, f.own_a, "2026-10-02", 1_000, "Mercado Sol devolucion", None);
    let groceries = category_id(&f.conn, "food.groceries");
    let res = set_category(&f.conn, a, groceries, true).unwrap();
    let rule_id = res.rule_id.expect("manual rule");
    let (origin, field, dir): (String, String, String) = f
        .conn
        .query_row("SELECT origin, field, direction FROM rules WHERE id = ?1", [rule_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!((origin.as_str(), field.as_str(), dir.as_str()), ("manual", "any", "any"));
    // The broad manual rule (any direction) also moves the refund of the same merchant.
    assert_eq!(rows_by_ids_for_test(&f, &[income_like])[0].category_id, Some(groceries));
    assert!(res.recategorized >= 1);
    // Doing it again repoints the same rule instead of failing on the unique index.
    let cafes = category_id(&f.conn, "food.cafes");
    let again = set_category(&f.conn, a, cafes, true).unwrap();
    assert_eq!(again.rule_id, Some(rule_id));
    assert_eq!(count(&f.conn, "SELECT count(*) FROM rules WHERE origin = 'manual'"), 1);
}

#[test]
fn set_category_on_a_transfer_side_unlinks_the_pair() {
    let f = fixture();
    let out = tx(&f, f.own_a, "2026-10-02", -10_000, "TRASPASO", None);
    let inn = tx(&f, f.own_b, "2026-10-02", 10_000, "TRASPASO", None);
    crate::transfers::detect_internal_transfers(&f.conn, &[out]).unwrap();
    let rent = category_id(&f.conn, "housing");
    let res = set_category(&f.conn, out, rent, false).unwrap();
    assert!(!res.row.is_transfer);
    assert_eq!(res.row.transfer_peer_id, None);
    let peer = &rows_by_ids_for_test(&f, &[inn])[0];
    assert!(!peer.is_transfer);
    assert_eq!(peer.transfer_peer_id, None);
}

#[test]
fn set_category_validates_ids() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -1_000, "Algo demo", None);
    let cat = category_id(&f.conn, "food");
    assert!(matches!(set_category(&f.conn, 9_999, cat, false), Err(LedgerError::NotFound(_))));
    assert!(matches!(set_category(&f.conn, a, 9_999, false), Err(LedgerError::NotFound(_))));
    assert_eq!(count(&f.conn, "SELECT count(*) FROM rules"), 0);
}

#[test]
fn notes_are_trimmed_cleared_and_limited() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -1_000, "Algo demo", None);
    assert_eq!(set_notes(&f.conn, a, Some("  recordar factura \n")).unwrap().notes.as_deref(), Some("recordar factura"));
    assert_eq!(set_notes(&f.conn, a, Some("   ")).unwrap().notes, None);
    set_notes(&f.conn, a, Some("x")).unwrap();
    assert_eq!(set_notes(&f.conn, a, None).unwrap().notes, None);
    assert!(set_notes(&f.conn, a, Some(&"n".repeat(MAX_NOTES_CHARS))).is_ok());
    assert!(matches!(set_notes(&f.conn, a, Some(&"n".repeat(MAX_NOTES_CHARS + 1))), Err(LedgerError::InvalidInput(_))));
    assert!(matches!(set_notes(&f.conn, 9_999, Some("x")), Err(LedgerError::NotFound(_))));
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn tags_are_cleaned_deduped_and_kept_when_unused() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -1_000, "Algo demo", None);
    let b = tx(&f, f.own_a, "2026-10-02", -1_000, "Otro demo", None);
    let row = set_tags(&f.conn, a, &strings(&["  Viaje ", "viaje", "", "Regalo"])).unwrap();
    assert_eq!(row.tags, vec!["Regalo", "Viaje"]);
    // Another spelling reuses the stored tag.
    let row_b = set_tags(&f.conn, b, &strings(&["VIAJE"])).unwrap();
    assert_eq!(row_b.tags, vec!["Viaje"]);
    assert_eq!(count(&f.conn, "SELECT count(*) FROM tags"), 2);
    // Removing tags keeps the names for suggestions.
    assert!(set_tags(&f.conn, a, &[]).unwrap().tags.is_empty());
    assert_eq!(tag_names(&f.conn).unwrap(), vec!["Regalo", "Viaje"]);
    // The list shows the tags.
    let page = list_transactions(&f.conn, &filter()).unwrap();
    assert_eq!(page.rows.iter().find(|r| r.id == b).unwrap().tags, vec!["Viaje"]);
}

#[test]
fn tag_limits_are_enforced_and_leave_the_row_untouched() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -1_000, "Algo demo", None);
    set_tags(&f.conn, a, &strings(&["uno"])).unwrap();
    let too_many: Vec<String> = (0..=MAX_TAGS).map(|i| format!("t{i}")).collect();
    assert!(matches!(set_tags(&f.conn, a, &too_many), Err(LedgerError::InvalidInput(_))));
    let exactly: Vec<String> = (0..MAX_TAGS).map(|i| format!("t{i}")).collect();
    assert!(set_tags(&f.conn, a, &exactly).is_ok());
    assert!(matches!(set_tags(&f.conn, a, &["x".repeat(MAX_TAG_CHARS + 1)]), Err(LedgerError::InvalidInput(_))));
    assert!(set_tags(&f.conn, a, &["x".repeat(MAX_TAG_CHARS)]).is_ok());
    assert!(matches!(set_tags(&f.conn, a, &strings(&["a\nb"])), Err(LedgerError::InvalidInput(_))));
    assert!(matches!(set_tags(&f.conn, 9_999, &[]), Err(LedgerError::NotFound(_))));
    // A refused call changed nothing.
    assert_eq!(rows_by_ids_for_test(&f, &[a])[0].tags.len(), 1);
}

#[test]
fn unlink_transfer_resets_both_sides() {
    let f = fixture();
    let out = tx(&f, f.own_a, "2026-10-02", -10_000, "TRASPASO", None);
    let inn = tx(&f, f.own_b, "2026-10-02", 10_000, "TRASPASO", None);
    crate::transfers::detect_internal_transfers(&f.conn, &[out]).unwrap();
    assert!(rows_by_ids_for_test(&f, &[out])[0].is_transfer);
    let rows = unlink_transfer(&f.conn, inn).unwrap();
    assert_eq!(rows.len(), 2);
    for r in &rows {
        assert!(!r.is_transfer);
        assert_eq!(r.transfer_peer_id, None);
        assert_eq!(r.category_id, None);
        assert_eq!(r.category_source, "none");
    }
    assert_eq!(rows[0].id, inn);
    // Totals now count both sides.
    let d = dashboard(&f.conn, "2026-10", "2026-10-31").unwrap();
    assert_eq!((d.totals.income_cents, d.totals.expense_cents), (10_000, 10_000));
    assert!(matches!(unlink_transfer(&f.conn, 9_999), Err(LedgerError::NotFound(_))));
}

#[test]
fn a_failed_mutation_rolls_back() {
    let f = fixture();
    let a = tx(&f, f.own_a, "2026-10-01", -1_000, "Algo demo", None);
    let b = tx(&f, f.own_a, "2026-10-02", -1_000, "Otra cosa", None);
    set_notes(&f.conn, a, Some("antes")).unwrap();
    // Unknown category: nothing is written, not even the manual category.
    assert!(set_category(&f.conn, a, 9_999, false).is_err());
    let row = &rows_by_ids_for_test(&f, &[a, b])[0];
    assert_eq!((row.category_id, row.notes.as_deref()), (None, Some("antes")));
}
