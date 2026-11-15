use chrono::TimeZone;
use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::bank::connect::register_session;
use crate::bank::test_support::*;
use crate::keystore::MemoryStore;

struct Env {
    f: Fixture,
    db: Mutex<Connection>,
    store: MemoryStore,
}

async fn env(accounts: &[(&str, &str)]) -> Env {
    let f = fixture().await;
    let mut conn = mem_db();
    let store = MemoryStore::new();
    register_session(
        &mut conn,
        &store,
        "Mock ASPSP",
        "ES",
        &authorized(FAKE_SESSION, accounts),
        now(),
    )
    .unwrap();
    Env { f, db: Mutex::new(conn), store }
}

impl Env {
    async fn run(&self, trigger: SyncTrigger, at: DateTime<Utc>) -> Result<SyncSummary, SyncError> {
        run_sync(&self.db, &self.f.client, &self.store, trigger, at).await
    }

    fn int(&self, sql: &str) -> i64 {
        self.db.lock().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn text(&self, sql: &str) -> String {
        self.db.lock().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    async fn requests(&self) -> usize {
        self.f.server.received_requests().await.unwrap().len()
    }
}

fn day_later(n: i64) -> DateTime<Utc> {
    now() + Days::days(n)
}

fn tx(reference: Option<&str>, status: &str, ind: &str, amount: &str, date: Option<&str>) -> Value {
    let mut v = json!({
        "transaction_amount": { "currency": "EUR", "amount": amount },
        "credit_debit_indicator": ind,
        "status": status,
        "creditor": { "name": "Tienda Ficticia" },
        "debtor": { "name": "Cliente Ficticio" },
        "remittance_information": ["Pago ficticio"]
    });
    if let Some(r) = reference {
        v["entry_reference"] = json!(r);
    }
    if let Some(d) = date {
        v[if status == "BOOK" { "booking_date" } else { "value_date" }] = json!(d);
    }
    v
}

fn balance(kind: &str, amount: &str) -> Value {
    json!({
        "balance_amount": { "currency": "EUR", "amount": amount },
        "balance_type": kind,
        "reference_date": "2026-11-14"
    })
}

fn parse_tx(v: Value) -> Transaction {
    serde_json::from_value(v).unwrap()
}

async fn mock_account(
    server: &MockServer,
    uid: &str,
    balances: Vec<Value>,
    txs: Vec<Value>,
    window: Option<(&str, &str)>,
) {
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{uid}/balances")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "balances": balances })))
        .mount(server)
        .await;
    let mut m = Mock::given(method("GET")).and(path(format!("/accounts/{uid}/transactions")));
    if let Some((from, to)) = window {
        m = m.and(query_param("date_from", from)).and(query_param("date_to", to));
    }
    m.respond_with(ResponseTemplate::new(200).set_body_json(json!({ "transactions": txs })))
        .mount(server)
        .await;
}

// ---- window ---------------------------------------------------------------------

#[test]
fn window_uses_overlap_or_initial_lookback() {
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    let today = d(2026, 11, 15);
    assert_eq!(sync_window(None, today), (d(2026, 8, 17), today));
    assert_eq!(sync_window(Some(d(2026, 11, 14)), today), (d(2026, 11, 9), today));
    assert_eq!(sync_window(Some(d(2026, 11, 15)), today), (d(2026, 11, 10), today));
    // A last sync "in the future" (clock change) never yields an inverted window.
    assert_eq!(sync_window(Some(d(2026, 12, 31)), today).0, today);
}

// ---- mapping --------------------------------------------------------------------

fn mapped(v: Value) -> NewTransaction {
    match map_transaction(&parse_tx(v)) {
        Mapped::Row(r) => r,
        Mapped::Skipped => panic!("expected a row"),
    }
}

#[test]
fn indicator_is_authoritative_over_the_amount_sign() {
    let cents = |ind: &str, amount: &str| {
        mapped(tx(Some("r"), "BOOK", ind, amount, Some("2026-11-10"))).amount_cents
    };
    assert_eq!(cents("DBIT", "20.00"), -2000);
    assert_eq!(cents("DBIT", "-20.00"), -2000);
    assert_eq!(cents("CRDT", "-20.00"), 2000);
    assert_eq!(cents("CRDT", "5"), 500);
    assert_eq!(cents("CRDT", "0.07"), 7);
}

#[test]
fn unusable_rows_are_skipped() {
    for status in ["CNCL", "RJCT", "SCHD", "OTHR"] {
        let t = tx(Some("r"), status, "DBIT", "1.00", Some("2026-11-10"));
        assert_eq!(map_transaction(&parse_tx(t)), Mapped::Skipped, "{status}");
    }
    for bad in ["abc", "", "--1", "1.234", "1,00"] {
        let t = tx(Some("r"), "BOOK", "DBIT", bad, Some("2026-11-10"));
        assert_eq!(map_transaction(&parse_tx(t)), Mapped::Skipped, "{bad}");
    }
    // Booked without any date; pending without any date.
    assert_eq!(map_transaction(&parse_tx(tx(None, "BOOK", "DBIT", "1", None))), Mapped::Skipped);
    assert_eq!(map_transaction(&parse_tx(tx(None, "PDNG", "DBIT", "1", None))), Mapped::Skipped);
}

#[test]
fn status_and_dates_follow_the_rules() {
    let hold = mapped(tx(Some("r"), "HOLD", "DBIT", "1.00", Some("2026-11-12")));
    assert_eq!(hold.status, TxStatus::Pending);
    assert_eq!((hold.booking_date.as_deref(), hold.value_date.as_deref()), (None, Some("2026-11-12")));

    // Booked: booking date, else value date, else transaction date.
    let mut v = tx(Some("r"), "BOOK", "DBIT", "1.00", None);
    v["transaction_date"] = json!("2026-11-01");
    assert_eq!(mapped(v.clone()).booking_date.as_deref(), Some("2026-11-01"));
    v["value_date"] = json!("2026-11-02");
    let row = mapped(v.clone());
    assert_eq!(row.booking_date.as_deref(), Some("2026-11-02"));
    v["booking_date"] = json!("2026-11-03");
    let row = mapped(v);
    assert_eq!(row.booking_date.as_deref(), Some("2026-11-03"));
    assert_eq!(row.value_date.as_deref(), Some("2026-11-02"));
}

#[test]
fn text_fields_come_from_the_right_party() {
    let out = mapped(tx(Some("ref-1"), "BOOK", "DBIT", "3.00", Some("2026-11-10")));
    assert_eq!(out.counterparty.as_deref(), Some("Tienda Ficticia"));
    assert_eq!(out.entry_reference.as_deref(), Some("ref-1"));
    let inc = mapped(tx(Some("ref-2"), "BOOK", "CRDT", "3.00", Some("2026-11-10")));
    assert_eq!(inc.counterparty.as_deref(), Some("Cliente Ficticio"));

    let mut v = tx(None, "BOOK", "DBIT", "3.00", Some("2026-11-10"));
    v["remittance_information"] = json!(["Linea uno", " ", "Linea dos"]);
    v["transaction_id"] = json!("tid-not-stable");
    let row = mapped(v.clone());
    assert_eq!(row.remittance.as_deref(), Some("Linea uno Linea dos"));
    assert_eq!(row.description, "Linea uno Linea dos");
    assert_eq!(row.entry_reference, None, "transaction_id must never be used");

    v["remittance_information"] = Value::Null;
    assert_eq!(mapped(v.clone()).description, "Tienda Ficticia");
    v["creditor"] = Value::Null;
    v["note"] = json!("Una nota");
    assert_eq!(mapped(v.clone()).description, "Una nota");
    v["note"] = Value::Null;
    let row = mapped(v);
    assert_eq!((row.description.as_str(), row.remittance, row.counterparty), ("", None, None));
}

#[test]
fn balance_preference_order() {
    let balances: Vec<Balance> = serde_json::from_value(json!([
        balance("XPCD", "5.00"),
        balance("CLBD", "100.00"),
        balance("ITAV", "90.00"),
        balance("ZZZZ", "1.00"),
    ]))
    .unwrap();
    assert_eq!(pick_balance(&balances).unwrap().balance_type, "ITAV");
    assert_eq!(pick_balance(&balances[..2]).unwrap().balance_type, "CLBD");
    let odd: Vec<Balance> = serde_json::from_value(json!([balance("QQQQ", "1.00"), balance("WWWW", "2.00")])).unwrap();
    assert_eq!(pick_balance(&odd).unwrap().balance_type, "QQQQ");
    assert!(pick_balance(&[]).is_none());
}

// ---- sync end to end ------------------------------------------------------------

#[tokio::test]
async fn first_sync_inserts_and_records_state() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    mock_account(
        &e.f.server,
        FAKE_UID_A,
        vec![balance("CLBD", "100.00"), balance("ITAV", "-12.50")],
        vec![
            tx(Some("r1"), "BOOK", "DBIT", "12.34", Some("2026-11-10")),
            tx(Some("r2"), "PDNG", "CRDT", "100.00", Some("2026-11-14")),
        ],
        Some(("2026-08-17", "2026-11-15")),
    )
    .await;

    let s = e.run(SyncTrigger::Manual, now()).await.unwrap();
    assert_eq!(
        (s.accounts, s.inserted, s.updated, s.pending, s.skipped, s.transfers_matched),
        (1, 2, 0, 1, 0, 0)
    );
    assert_eq!(e.int("SELECT amount_cents FROM transactions WHERE entry_reference = 'r1'"), -1234);
    assert_eq!(e.text("SELECT status FROM transactions WHERE entry_reference = 'r2'"), "pending");
    assert_eq!(e.text("SELECT counterparty FROM transactions WHERE entry_reference = 'r1'"), "Tienda Ficticia");
    assert_eq!(e.text("SELECT source FROM transactions WHERE entry_reference = 'r1'"), "bank");

    assert_eq!(e.text("SELECT last_synced_to FROM account_sync_state"), "2026-11-15");
    assert_eq!(e.int("SELECT amount_cents FROM account_balances"), -1250);
    assert_eq!(e.text("SELECT balance_type FROM account_balances"), "ITAV");
    assert_eq!(e.int("SELECT count(*) FROM account_balances"), 1);

    assert_eq!(e.int("SELECT count(*) FROM sync_runs"), 1);
    assert_eq!(e.text("SELECT status FROM sync_runs"), "ok");
    assert_eq!(e.text("SELECT \"trigger\" FROM sync_runs"), "manual");
    assert_eq!(e.int("SELECT account_id IS NULL FROM sync_runs"), 1);
    assert_eq!(e.int("SELECT fetched FROM sync_runs"), 2);
    assert_eq!(e.int("SELECT inserted FROM sync_runs"), 2);
    assert_eq!(e.int("SELECT finished_at IS NOT NULL FROM sync_runs"), 1);
    assert_eq!(e.int("SELECT count(*) FROM transactions WHERE sync_run_id IS NOT NULL"), 2);
}

#[tokio::test]
async fn second_sync_uses_the_overlap_window_and_inserts_nothing_new() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    let rows = vec![
        tx(Some("r1"), "BOOK", "DBIT", "12.34", Some("2026-11-10")),
        tx(Some("r2"), "BOOK", "CRDT", "3.00", Some("2026-11-12")),
    ];
    mock_account(&e.f.server, FAKE_UID_A, vec![], rows.clone(), Some(("2026-08-17", "2026-11-15"))).await;
    assert_eq!(e.run(SyncTrigger::Manual, now()).await.unwrap().inserted, 2);

    // Next day: 2026-11-15 minus 5 days = 2026-11-10. Any other window gets a 404.
    e.f.server.reset().await;
    mock_account(&e.f.server, FAKE_UID_A, vec![], rows, Some(("2026-11-10", "2026-11-16"))).await;
    let s = e.run(SyncTrigger::Manual, day_later(1)).await.unwrap();
    assert_eq!((s.inserted, s.updated), (0, 0));
    assert_eq!(e.int("SELECT count(*) FROM transactions"), 2);
    assert_eq!(e.text("SELECT last_synced_to FROM account_sync_state"), "2026-11-16");
    assert_eq!(e.int("SELECT count(*) FROM sync_runs"), 2);
}

#[tokio::test]
async fn pending_becomes_booked_through_the_entry_reference() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    mock_account(&e.f.server, FAKE_UID_A, vec![], vec![tx(Some("r1"), "PDNG", "DBIT", "20.00", Some("2026-11-14"))], None).await;
    e.run(SyncTrigger::Manual, now()).await.unwrap();
    assert_eq!(e.text("SELECT status FROM transactions"), "pending");

    e.f.server.reset().await;
    mock_account(&e.f.server, FAKE_UID_A, vec![], vec![tx(Some("r1"), "BOOK", "DBIT", "20.00", Some("2026-11-15"))], None).await;
    let s = e.run(SyncTrigger::Manual, day_later(1)).await.unwrap();
    assert_eq!((s.inserted, s.updated), (0, 1));
    assert_eq!(e.int("SELECT count(*) FROM transactions"), 1);
    assert_eq!(e.text("SELECT status FROM transactions"), "booked");
    assert_eq!(e.text("SELECT booking_date FROM transactions"), "2026-11-15");
}

#[tokio::test]
async fn stale_pending_rows_are_removed() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    mock_account(
        &e.f.server,
        FAKE_UID_A,
        vec![],
        vec![
            tx(None, "PDNG", "DBIT", "9.99", Some("2026-11-14")),
            tx(Some("r1"), "BOOK", "DBIT", "1.00", Some("2026-11-13")),
        ],
        None,
    )
    .await;
    e.run(SyncTrigger::Manual, now()).await.unwrap();
    assert_eq!(e.int("SELECT count(*) FROM transactions WHERE status = 'pending'"), 1);

    // The bank no longer reports the pending row.
    e.f.server.reset().await;
    mock_account(&e.f.server, FAKE_UID_A, vec![], vec![tx(Some("r1"), "BOOK", "DBIT", "1.00", Some("2026-11-13"))], None).await;
    e.run(SyncTrigger::Manual, day_later(1)).await.unwrap();
    assert_eq!(e.int("SELECT count(*) FROM transactions WHERE status = 'pending'"), 0);
    assert_eq!(e.int("SELECT count(*) FROM transactions"), 1);
}

#[tokio::test]
async fn internal_transfer_between_two_linked_accounts_is_detected() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A), (FAKE_IBAN_B, FAKE_UID_B)]).await;
    mock_account(&e.f.server, FAKE_UID_A, vec![], vec![tx(Some("a1"), "BOOK", "DBIT", "50.00", Some("2026-11-10"))], None).await;
    mock_account(&e.f.server, FAKE_UID_B, vec![], vec![tx(Some("b1"), "BOOK", "CRDT", "50.00", Some("2026-11-10"))], None).await;
    let s = e.run(SyncTrigger::Manual, now()).await.unwrap();
    assert_eq!((s.accounts, s.inserted, s.transfers_matched), (2, 2, 1));
    assert_eq!(e.int("SELECT count(*) FROM transactions WHERE transfer_peer_id IS NOT NULL"), 2);
    assert_eq!(e.int("SELECT count(*) FROM transactions WHERE category_source = 'transfer'"), 2);
}

#[tokio::test]
async fn cancelled_scheduled_and_unparsable_rows_are_counted_as_skipped() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    mock_account(
        &e.f.server,
        FAKE_UID_A,
        vec![],
        vec![
            tx(Some("ok"), "BOOK", "DBIT", "1.00", Some("2026-11-10")),
            tx(Some("c"), "CNCL", "DBIT", "1.00", Some("2026-11-10")),
            tx(Some("s"), "SCHD", "DBIT", "1.00", Some("2026-11-10")),
            tx(Some("b"), "BOOK", "DBIT", "not-a-number", Some("2026-11-10")),
        ],
        None,
    )
    .await;
    let s = e.run(SyncTrigger::Manual, now()).await.unwrap();
    assert_eq!((s.inserted, s.skipped), (1, 3));
    assert_eq!(e.int("SELECT count(*) FROM transactions"), 1);
}

#[tokio::test]
async fn expired_session_requires_reauthorization_and_marks_the_run() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    mock_account(&e.f.server, FAKE_UID_A, vec![], vec![], None).await;
    e.f.server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": "EXPIRED_SESSION",
            "message": format!("secret {FAKE_IBAN_A} {FAKE_NAME}")
        })))
        .mount(&e.f.server)
        .await;
    let err = e.run(SyncTrigger::Manual, now()).await.unwrap_err();
    assert!(matches!(err, SyncError::ReauthRequired));
    assert_eq!(e.text("SELECT status FROM sync_runs"), "error");
    assert_eq!(e.text("SELECT error_code FROM sync_runs"), "EXPIRED_SESSION");
    assert_eq!(e.int("SELECT finished_at IS NOT NULL FROM sync_runs"), 1);
    assert_eq!(e.int("SELECT count(*) FROM account_sync_state"), 0);

    // Nothing printed or stored carries the fake personal data.
    let shown = format!(
        "{err} {err:?} {}",
        e.text("SELECT group_concat(coalesce(error_code, '') || status) FROM sync_runs")
    );
    for secret in [FAKE_IBAN_A, FAKE_NAME, FAKE_SESSION, FAKE_UID_A] {
        assert!(!shown.contains(secret), "leaked {secret}");
    }
}

#[tokio::test]
async fn rate_limit_stops_the_run_but_keeps_finished_accounts() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A), (FAKE_IBAN_B, FAKE_UID_B)]).await;
    mock_account(&e.f.server, FAKE_UID_A, vec![], vec![tx(Some("a1"), "BOOK", "DBIT", "5.00", Some("2026-11-10"))], None).await;
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{FAKE_UID_B}/balances")))
        .respond_with(
            ResponseTemplate::new(429).set_body_json(json!({ "error": "ASPSP_RATE_LIMIT_EXCEEDED" })),
        )
        .mount(&e.f.server)
        .await;
    let err = e.run(SyncTrigger::Auto, now()).await.unwrap_err();
    assert!(matches!(err, SyncError::RateLimited));
    assert_eq!(e.int("SELECT count(*) FROM transactions"), 1, "account A stays committed");
    assert_eq!(e.int("SELECT count(*) FROM account_sync_state"), 1);
    assert_eq!(e.text("SELECT status FROM sync_runs"), "error");
    assert_eq!(e.text("SELECT error_code FROM sync_runs"), "ASPSP_RATE_LIMIT_EXCEEDED");
    assert_eq!(e.text("SELECT \"trigger\" FROM sync_runs"), "auto");
}

#[tokio::test]
async fn other_api_errors_stop_with_their_code() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "error": "SOME_BANK_PROBLEM" })))
        .mount(&e.f.server)
        .await;
    match e.run(SyncTrigger::Manual, now()).await.unwrap_err() {
        SyncError::Bank { code } => assert_eq!(code, "SOME_BANK_PROBLEM"),
        other => panic!("{other:?}"),
    }
    assert_eq!(e.text("SELECT error_code FROM sync_runs"), "SOME_BANK_PROBLEM");
}

#[tokio::test]
async fn not_connected_makes_no_http_call_and_no_run() {
    let f = fixture().await;
    let db = Mutex::new(mem_db());
    let store = MemoryStore::new();
    let err = run_sync(&db, &f.client, &store, SyncTrigger::Manual, now()).await.unwrap_err();
    assert!(matches!(err, SyncError::NotConnected));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    let runs: i64 = db.lock().unwrap().query_row("SELECT count(*) FROM sync_runs", [], |r| r.get(0)).unwrap();
    assert_eq!(runs, 0);
}

#[tokio::test]
async fn expired_consent_is_not_connected() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    let after_expiry = Utc.with_ymd_and_hms(2027, 5, 2, 0, 0, 0).unwrap();
    let err = e.run(SyncTrigger::Manual, after_expiry).await.unwrap_err();
    assert!(matches!(err, SyncError::NotConnected));
    assert_eq!(e.requests().await, 0);
    assert_eq!(e.int("SELECT count(*) FROM sync_runs"), 0);
}

#[tokio::test]
async fn blocked_sync_makes_no_http_call() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    insert_run(&e.db.lock().unwrap(), now() - Days::seconds(10), "ok", "manual");
    let err = e.run(SyncTrigger::Manual, now()).await.unwrap_err();
    assert!(matches!(err, SyncError::Blocked(SyncBlocked::ManualTooSoon)));
    assert_eq!(e.requests().await, 0);
}

// ---- policy ---------------------------------------------------------------------

fn insert_run(conn: &Connection, started: DateTime<Utc>, status: &str, trigger: &str) {
    conn.execute(
        "INSERT INTO sync_runs (provider, started_at, status, \"trigger\")
         VALUES ('enable_banking', ?1, ?2, ?3)",
        params![db_timestamp(started), status, trigger],
    )
    .unwrap();
}

fn blocked(conn: &Connection, trigger: SyncTrigger) -> Option<SyncBlocked> {
    match sync_allowed(conn, trigger, now()) {
        Ok(()) => None,
        Err(SyncError::Blocked(b)) => Some(b),
        Err(other) => panic!("{other:?}"),
    }
}

#[test]
fn auto_policy_needs_six_hours_between_runs() {
    let conn = mem_db();
    assert_eq!(blocked(&conn, SyncTrigger::Auto), None);
    insert_run(&conn, now() - Days::hours(6) + Days::seconds(1), "ok", "auto");
    assert_eq!(blocked(&conn, SyncTrigger::Auto), Some(SyncBlocked::AutoTooSoon));

    let conn = mem_db();
    insert_run(&conn, now() - Days::hours(6), "ok", "auto");
    assert_eq!(blocked(&conn, SyncTrigger::Auto), None);

    // Failed runs count as well.
    let conn = mem_db();
    insert_run(&conn, now() - Days::hours(1), "error", "auto");
    assert_eq!(blocked(&conn, SyncTrigger::Auto), Some(SyncBlocked::AutoTooSoon));
}

#[test]
fn auto_policy_allows_four_runs_per_day() {
    let conn = mem_db();
    for hours in [7, 13, 19] {
        insert_run(&conn, now() - Days::hours(hours), "ok", "auto");
    }
    assert_eq!(blocked(&conn, SyncTrigger::Auto), None);
    insert_run(&conn, now() - Days::hours(23), "error", "auto");
    assert_eq!(blocked(&conn, SyncTrigger::Auto), Some(SyncBlocked::AutoDailyLimit));
    // The same history does not stop a manual sync, and old runs fall out of the day.
    assert_eq!(blocked(&conn, SyncTrigger::Manual), None);
    let later = now() + Days::hours(2);
    assert!(sync_allowed(&conn, SyncTrigger::Auto, later).is_ok());
}

#[test]
fn manual_policy_guards_against_double_clicks() {
    let conn = mem_db();
    insert_run(&conn, now() - Days::seconds(59), "ok", "manual");
    assert_eq!(blocked(&conn, SyncTrigger::Manual), Some(SyncBlocked::ManualTooSoon));

    let conn = mem_db();
    insert_run(&conn, now() - Days::seconds(61), "ok", "manual");
    assert_eq!(blocked(&conn, SyncTrigger::Manual), None);

    // An automatic run also counts for the guard.
    let conn = mem_db();
    insert_run(&conn, now() - Days::seconds(5), "ok", "auto");
    assert_eq!(blocked(&conn, SyncTrigger::Manual), Some(SyncBlocked::ManualTooSoon));
}

#[test]
fn running_sync_blocks_until_it_is_stale() {
    let conn = mem_db();
    insert_run(&conn, now() - Days::minutes(5), "running", "manual");
    assert_eq!(blocked(&conn, SyncTrigger::Manual), Some(SyncBlocked::AlreadyRunning));
    assert_eq!(blocked(&conn, SyncTrigger::Auto), Some(SyncBlocked::AlreadyRunning));

    let conn = mem_db();
    insert_run(&conn, now() - Days::minutes(20), "running", "manual");
    assert_eq!(blocked(&conn, SyncTrigger::Manual), None);
}

#[tokio::test]
async fn crashed_runs_are_closed_as_aborted_on_the_next_sync() {
    let e = env(&[(FAKE_IBAN_A, FAKE_UID_A)]).await;
    insert_run(&e.db.lock().unwrap(), now() - Days::minutes(30), "running", "manual");
    mock_account(&e.f.server, FAKE_UID_A, vec![], vec![], None).await;
    e.run(SyncTrigger::Manual, now()).await.unwrap();
    assert_eq!(
        e.int("SELECT count(*) FROM sync_runs WHERE status = 'error' AND error_code = 'ABORTED'"),
        1
    );
}

#[test]
fn summary_and_plan_debug_contain_counts_only() {
    let plan = AccountPlan {
        account_id: 1,
        uid: Sensitive::new(FAKE_UID_A.to_owned()),
        date_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        date_to: NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
    };
    let fetch = AccountFetch {
        balances: vec![],
        transactions: vec![parse_tx(tx(Some("secret-ref"), "BOOK", "DBIT", "12.34", Some("2026-11-10")))],
    };
    let shown = format!("{plan:?} {fetch:?} {:?}", SyncSummary::default());
    for secret in [FAKE_UID_A, "secret-ref", "12.34", "Tienda Ficticia", "Pago ficticio"] {
        assert!(!shown.contains(secret), "leaked {secret}");
    }
}
