use chrono::{Duration, TimeZone};
use rusqlite::params;

use super::*;
use crate::bank::connect::{register_session, SESSION_ENTRY};
use crate::bank::db_timestamp;
use crate::bank::test_support::*;
use crate::keystore::MemoryStore;

const SECRET_SESSION: &str = "fake-session-id-must-not-leak";

fn connected() -> (rusqlite::Connection, MemoryStore) {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    register_session(
        &mut conn,
        &store,
        "Mock ASPSP",
        "ES",
        &authorized(SECRET_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A), (FAKE_IBAN_B, FAKE_UID_B)]),
        now(),
    )
    .unwrap();
    (conn, store)
}

fn insert_run(conn: &rusqlite::Connection, started: DateTime<Utc>, finish_ms: Option<i64>, status: &str, trigger: &str) {
    let finished = finish_ms.map(|ms| db_timestamp(started + Duration::milliseconds(ms)));
    conn.execute(
        "INSERT INTO sync_runs (provider, started_at, finished_at, status, \"trigger\", inserted, updated, error_code)
         VALUES ('enable_banking', ?1, ?2, ?3, ?4, 3, 1, ?5)",
        params![db_timestamp(started), finished, status, trigger, (status == "error").then_some("RATE_LIMITED")],
    )
    .unwrap();
}

#[test]
fn not_connected_without_a_stored_session() {
    let store = MemoryStore::new();
    assert_eq!(connection_view(&store, now()).unwrap(), ConnectionView::NotConnected);
}

#[test]
fn connected_reports_dates_and_days() {
    let (_conn, store) = connected();
    match connection_view(&store, now()).unwrap() {
        ConnectionView::Connected { aspsp_name, valid_until, days_remaining, consent_days_total, granted_on } => {
            assert_eq!(aspsp_name, "Mock ASPSP");
            assert_eq!(granted_on, now());
            assert_eq!(days_remaining, (valid_until - now()).num_days());
            assert_eq!(consent_days_total, days_remaining); // granted just now
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn expired_once_valid_until_has_passed() {
    let (_conn, store) = connected();
    let later = Utc.with_ymd_and_hms(2027, 5, 2, 0, 0, 0).unwrap();
    assert!(matches!(connection_view(&store, later).unwrap(), ConnectionView::Expired { .. }));
}

#[test]
fn views_serialize_without_secrets() {
    let (conn, store) = connected();
    let view = serde_json::to_string(&connection_view(&store, now()).unwrap()).unwrap();
    let accounts = serde_json::to_string(&accounts(&conn).unwrap()).unwrap();
    for text in [view, accounts] {
        assert!(!text.contains(SECRET_SESSION));
        assert!(!text.contains(FAKE_UID_A));
        assert!(!text.contains(FAKE_IBAN_A));
    }
    assert!(store.get(SESSION_ENTRY).unwrap().is_some());
}

#[test]
fn accounts_carry_suffix_balance_and_last_sync() {
    let (conn, _store) = connected();
    let first: i64 = conn.query_row("SELECT min(id) FROM accounts", [], |r| r.get(0)).unwrap();
    conn.execute(
        "INSERT INTO account_balances (account_id, amount_cents, currency, balance_type, fetched_at)
         VALUES (?1, -10573, 'EUR', 'CLBD', '2026-11-15T10:00:00.000Z')",
        [first],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO account_sync_state (account_id, last_synced_to, last_success_at)
         VALUES (?1, '2026-11-15', '2026-11-15T10:00:00.000Z')",
        [first],
    )
    .unwrap();
    let list = accounts(&conn).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].balance_cents, Some(-10573));
    assert_eq!(list[0].last_synced_at.as_deref(), Some("2026-11-15T10:00:00.000Z"));
    assert_eq!(list[0].kind, "checking");
    assert_eq!(list[0].display_suffix.as_deref().map(str::len), Some(4));
    assert_eq!(list[1].balance_cents, None);
    assert_eq!(list[1].last_synced_at, None);
}

#[test]
fn archived_accounts_are_hidden() {
    let (conn, _store) = connected();
    conn.execute("UPDATE accounts SET archived = 1 WHERE id = (SELECT min(id) FROM accounts)", []).unwrap();
    assert_eq!(accounts(&conn).unwrap().len(), 1);
}

#[test]
fn recent_runs_are_newest_first_limited_and_timed() {
    let conn = mem_db();
    for i in 0..7 {
        insert_run(&conn, now() - Duration::hours(i), Some(3800), "ok", "auto");
    }
    insert_run(&conn, now() + Duration::minutes(1), None, "running", "manual");
    let runs = recent_runs(&conn, HISTORY_LIMIT).unwrap();
    assert_eq!(runs.len(), 5);
    assert_eq!(runs[0].status, "running");
    assert_eq!(runs[0].duration_ms, None);
    assert_eq!(runs[1].duration_ms, Some(3800));
    assert_eq!((runs[1].inserted, runs[1].updated), (3, 1));
    assert!(runs[1].started_at > runs[2].started_at);
}

#[test]
fn error_runs_expose_only_the_code() {
    let conn = mem_db();
    insert_run(&conn, now(), Some(100), "error", "manual");
    let runs = recent_runs(&conn, 5).unwrap();
    assert_eq!(runs[0].error_code.as_deref(), Some("RATE_LIMITED"));
}

#[test]
fn next_auto_sync_follows_the_six_hour_spacing() {
    let conn = mem_db();
    assert_eq!(next_auto_sync_at(&conn, now()).unwrap(), now());
    insert_run(&conn, now() - Duration::hours(2), Some(1000), "ok", "auto");
    insert_run(&conn, now() - Duration::minutes(5), Some(1000), "ok", "manual"); // manual runs do not count
    assert_eq!(next_auto_sync_at(&conn, now()).unwrap(), now() + Duration::hours(4));
    insert_run(&conn, now() - Duration::hours(10), Some(1000), "ok", "auto");
    let later = now() + Duration::hours(5);
    assert_eq!(next_auto_sync_at(&conn, later).unwrap(), later);
}
