use std::sync::{Arc, Mutex};

use chrono::TimeZone;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use sha2::{Digest, Sha256};

use super::*;
use crate::bank::enable_banking::Session;
use crate::bank::test_support::*;
use crate::keystore::MemoryStore;

const ASPSP: &str = "Mock ASPSP";

/// Remembers what `POST /auth` was sent, so the fake browser can echo the state.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Option<Value>>>);

impl Respond for Captured {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        *self.0.lock().unwrap() = Some(serde_json::from_slice(&req.body).unwrap());
        ResponseTemplate::new(200).set_body_json(
            json!({ "url": "https://bank.example/authorize", "authorization_id": "auth-1" }),
        )
    }
}

async fn free_port() -> u16 {
    CallbackListener::bind(0).await.unwrap().port()
}

async fn mock_aspsps(server: &MockServer, max_validity_secs: u64) {
    Mock::given(method("GET"))
        .and(path("/aspsps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "aspsps": [
                { "name": "Other Bank", "country": "ES", "maximum_consent_validity": 86400, "psu_types": ["personal"] },
                { "name": ASPSP, "country": "ES", "maximum_consent_validity": max_validity_secs, "psu_types": ["personal"] }
            ]
        })))
        .mount(server)
        .await;
}

async fn mock_auth(server: &MockServer) -> Captured {
    let captured = Captured::default();
    Mock::given(method("POST"))
        .and(path("/auth"))
        .respond_with(captured.clone())
        .mount(server)
        .await;
    captured
}

async fn mock_sessions(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(session_body(FAKE_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A)])),
        )
        .mount(server)
        .await;
}

fn params(port: u16) -> AuthParams {
    AuthParams { port, timeout: Duration::from_secs(20), ..AuthParams::new(ASPSP, "ES") }
}

/// A fake browser: GETs the loopback callback with the query built from the state.
fn browser(
    port: u16,
    captured: Captured,
    query: impl FnOnce(&str) -> String + Send + 'static,
) -> impl FnOnce(&str) -> Result<(), String> {
    move |_url| {
        let state =
            captured.0.lock().unwrap().as_ref().unwrap()["state"].as_str().unwrap().to_owned();
        let q = query(&state);
        tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
            let req = format!("GET /callback?{q} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
            s.write_all(req.as_bytes()).await.unwrap();
            let mut sink = Vec::new();
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut s, &mut sink).await;
        });
        Ok(())
    }
}

async fn requests_to(server: &MockServer, verb: &str, p: &str) -> Vec<Request> {
    server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.method.as_str() == verb && r.url.path() == p)
        .collect()
}

fn offered_validity_secs(captured: &Captured, since: DateTime<Utc>) -> i64 {
    let auth = captured.0.lock().unwrap().clone().unwrap();
    let until: DateTime<Utc> = auth["access"]["valid_until"].as_str().unwrap().parse().unwrap();
    (until - since).num_seconds()
}

#[tokio::test]
async fn full_flow_returns_the_session() {
    let f = fixture().await;
    mock_aspsps(&f.server, 365 * 86_400).await;
    let captured = mock_auth(&f.server).await;
    mock_sessions(&f.server).await;

    let port = free_port().await;
    let open = browser(port, captured.clone(), |s| format!("code={FAKE_CODE}&state={s}"));
    let before = Utc::now();
    let session = authorize(&f.client, params(port), open).await.unwrap();

    assert_eq!(session.session_id.expose(), FAKE_SESSION);
    assert_eq!(session.accounts.len(), 1);

    let auth = captured.0.lock().unwrap().clone().unwrap();
    assert_eq!(auth["redirect_url"], REDIRECT_URL);
    assert_eq!(auth["aspsp"]["name"], ASPSP);
    assert_eq!(auth["psu_type"], "personal");
    // 365 days offered by the bank, capped to 180.
    let secs = offered_validity_secs(&captured, before);
    assert!((180 * 86_400 - 5..=180 * 86_400 + 120).contains(&secs), "{secs}");

    let posted = requests_to(&f.server, "POST", "/sessions").await;
    assert_eq!(posted.len(), 1);
    let body: Value = serde_json::from_slice(&posted[0].body).unwrap();
    assert_eq!(body["code"], FAKE_CODE);
}

#[tokio::test]
async fn shorter_bank_limit_wins_over_the_cap() {
    let f = fixture().await;
    mock_aspsps(&f.server, 90 * 86_400).await;
    let captured = mock_auth(&f.server).await;
    mock_sessions(&f.server).await;
    let port = free_port().await;
    let open = browser(port, captured.clone(), |s| format!("code={FAKE_CODE}&state={s}"));
    let before = Utc::now();
    authorize(&f.client, params(port), open).await.unwrap();
    let secs = offered_validity_secs(&captured, before);
    // The bank's limit minus the clock-skew margin, so valid_until never exceeds it.
    let expected = 90 * 86_400 - i64::try_from(CONSENT_MARGIN_SECS).unwrap();
    assert!((expected - 5..=expected + 120).contains(&secs), "{secs}");
}

#[tokio::test]
async fn wrong_state_aborts_before_the_code_exchange() {
    let f = fixture().await;
    mock_aspsps(&f.server, 86_400 * 365).await;
    let captured = mock_auth(&f.server).await;
    mock_sessions(&f.server).await;
    let port = free_port().await;
    let open = browser(port, captured, |_| format!("code={FAKE_CODE}&state=forged"));
    let err = authorize(&f.client, params(port), open).await.unwrap_err();
    assert!(matches!(err, AuthError::StateMismatch));
    assert!(requests_to(&f.server, "POST", "/sessions").await.is_empty());
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains(FAKE_CODE) && !shown.contains("forged"));
}

#[tokio::test]
async fn denied_authorization_is_reported() {
    let f = fixture().await;
    mock_aspsps(&f.server, 86_400 * 365).await;
    let captured = mock_auth(&f.server).await;
    let port = free_port().await;
    let open = browser(port, captured, |s| format!("error=access_denied&state={s}"));
    let err = authorize(&f.client, params(port), open).await.unwrap_err();
    assert!(matches!(err, AuthError::Denied { ref code } if code == "access_denied"));
}

#[tokio::test]
async fn unknown_bank_is_reported_before_auth() {
    let f = fixture().await;
    mock_aspsps(&f.server, 86_400).await;
    let mut p = params(free_port().await);
    p.aspsp_name = "Nope".into();
    let err = authorize(&f.client, p, |_| Ok(())).await.unwrap_err();
    assert!(matches!(err, AuthError::AspspNotFound));
    assert!(requests_to(&f.server, "POST", "/auth").await.is_empty());
}

#[tokio::test]
async fn busy_port_fails_before_any_request() {
    let f = fixture().await;
    let taken = CallbackListener::bind(0).await.unwrap();
    let err = authorize(&f.client, params(taken.port()), |_| Ok(())).await.unwrap_err();
    assert!(matches!(err, AuthError::PortInUse));
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn browser_failure_is_reported_without_its_message() {
    let f = fixture().await;
    mock_aspsps(&f.server, 86_400 * 365).await;
    mock_auth(&f.server).await;
    let open = |_: &str| Err("https://bank.example/secret-url".to_owned());
    let err = authorize(&f.client, params(free_port().await), open).await.unwrap_err();
    assert!(matches!(err, AuthError::BrowserOpen));
    assert!(!format!("{err} {err:?}").contains("secret-url"));
}

#[tokio::test]
async fn timeout_without_a_callback() {
    let f = fixture().await;
    mock_aspsps(&f.server, 86_400 * 365).await;
    mock_auth(&f.server).await;
    let mut p = params(free_port().await);
    p.timeout = Duration::from_millis(150);
    let err = authorize(&f.client, p, |_| Ok(())).await.unwrap_err();
    assert!(matches!(err, AuthError::Timeout));
}

// ---- register_session -----------------------------------------------------------

fn text(conn: &Connection, sql: &str) -> String {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn register(conn: &mut Connection, store: &MemoryStore, s: &AuthorizedSession) -> ConnectOutcome {
    register_session(conn, store, ASPSP, "ES", s, now()).unwrap()
}

fn from_body(body: Value) -> AuthorizedSession {
    let s: Session = serde_json::from_value(body).unwrap();
    AuthorizedSession { session_id: s.session_id, valid_until: s.access.valid_until, accounts: s.accounts }
}

#[test]
fn register_stores_the_session_and_links_accounts() {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    let s = authorized(FAKE_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A), (FAKE_IBAN_B, FAKE_UID_B)]);
    let out = register(&mut conn, &store, &s);
    assert_eq!(out.accounts_linked, 2);
    assert!(out.previous_session_id.is_none());

    let raw = store.get(SESSION_ENTRY).unwrap().unwrap();
    let json: Value = serde_json::from_slice(&raw).unwrap();
    assert_eq!(json["session_id"], FAKE_SESSION);
    assert_eq!(json["aspsp_name"], ASPSP);
    assert_eq!(json["aspsp_country"], "ES");
    assert!(json["valid_until"].is_string() && json["connected_at"].is_string());

    let by_suffix = |col: &str| {
        text(&conn, &format!("SELECT {col} FROM accounts WHERE display_suffix = '0001'"))
    };
    assert_eq!(by_suffix("identity_hash"), hex::encode(Sha256::digest(FAKE_IBAN_A.as_bytes())));
    assert_eq!(by_suffix("institution"), ASPSP);
    assert_eq!(by_suffix("kind"), "checking");
    assert_eq!(by_suffix("name"), "Cuenta ficticia");
    assert_eq!(count(&conn, "SELECT count(*) FROM account_links WHERE session_ref IS NULL"), 2);
    assert_eq!(
        text(&conn, &format!("SELECT valid_until FROM account_links WHERE external_uid = '{FAKE_UID_A}'")),
        "2027-05-01T10:00:00.000Z"
    );
    // Neither the holder name nor the IBAN is stored in the accounts.
    let all = text(
        &conn,
        "SELECT group_concat(name || institution || identity_hash || display_suffix) FROM accounts",
    );
    assert!(!all.contains(FAKE_NAME) && !all.contains(FAKE_IBAN_A));
}

#[test]
fn reconnect_replaces_uids_keeps_accounts_and_returns_the_previous_session() {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    register(&mut conn, &store, &authorized(FAKE_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A)]));
    let account_id = count(&conn, "SELECT id FROM accounts");

    // Same IBAN (typed differently), new session and new uid.
    let again = authorized("99999999-session", &[("es00 0000 0000 0000 0000 0001", "uid-new")]);
    let out = register(&mut conn, &store, &again);

    assert_eq!(out.previous_session_id.unwrap().expose(), FAKE_SESSION);
    assert_eq!(count(&conn, "SELECT count(*) FROM accounts"), 1);
    assert_eq!(count(&conn, "SELECT id FROM accounts"), account_id);
    assert_eq!(count(&conn, "SELECT count(*) FROM account_links"), 1);
    assert_eq!(text(&conn, "SELECT external_uid FROM account_links"), "uid-new");
    let stored = load_session(&store).unwrap().unwrap();
    assert_eq!(stored.session_id().expose(), "99999999-session");

    // Registering the very same session again reports no previous session.
    let out = register(&mut conn, &store, &again);
    assert!(out.previous_session_id.is_none());
}

#[test]
fn account_without_product_gets_a_generic_name_and_unknown_type_is_other() {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    let mut body = session_body(FAKE_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A)]);
    body["accounts"][0].as_object_mut().unwrap().remove("product");
    body["accounts"][0]["cash_account_type"] = json!("LLSV");
    register(&mut conn, &store, &from_body(body));
    assert_eq!(text(&conn, "SELECT name FROM accounts"), "Cuenta \u{b7}\u{b7}\u{b7}\u{b7}0001");
    assert_eq!(text(&conn, "SELECT kind FROM accounts"), "other");
}

#[test]
fn account_without_iban_falls_back_to_the_identification_hash() {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    let body = json!({
        "session_id": FAKE_SESSION,
        "accounts": [
            { "uid": "u1", "identification_hash": "fake-hash", "currency": "EUR" },
            { "uid": "u2" }
        ],
        "access": { "valid_until": "2027-05-01T10:00:00Z" }
    });
    let out = register(&mut conn, &store, &from_body(body));
    assert_eq!((out.accounts_linked, out.accounts_skipped), (1, 1));
    assert_eq!(text(&conn, "SELECT identity_hash FROM accounts").len(), 64);
}

#[test]
fn debug_output_hides_secrets() {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    let s = authorized(FAKE_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A)]);
    let out = register(&mut conn, &store, &s);
    let stored = load_session(&store).unwrap().unwrap();
    let shown = format!("{s:?} {out:?} {stored:?}");
    for secret in [FAKE_SESSION, FAKE_IBAN_A, FAKE_UID_A, FAKE_NAME] {
        assert!(!shown.contains(secret), "leaked {secret}");
    }
}

// ---- status ---------------------------------------------------------------------

#[test]
fn status_not_connected_connected_expired() {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    assert_eq!(connection_status(&store, now()).unwrap(), ConnectionStatus::NotConnected);

    register(&mut conn, &store, &authorized(FAKE_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A)]));
    match connection_status(&store, now()).unwrap() {
        ConnectionStatus::Connected { aspsp_name, days_remaining, valid_until } => {
            assert_eq!(aspsp_name, ASPSP);
            assert_eq!(valid_until, Utc.with_ymd_and_hms(2027, 5, 1, 10, 0, 0).unwrap());
            assert_eq!(days_remaining, 167);
        }
        other => panic!("{other:?}"),
    }
    let later = Utc.with_ymd_and_hms(2027, 5, 2, 0, 0, 0).unwrap();
    assert_eq!(
        connection_status(&store, later).unwrap(),
        ConnectionStatus::Expired {
            aspsp_name: ASPSP.into(),
            expired_on: NaiveDate::from_ymd_opt(2027, 5, 1).unwrap()
        }
    );
}

#[test]
fn unreadable_entry_counts_as_not_connected() {
    let store = MemoryStore::new();
    store.set(SESSION_ENTRY, b"not json").unwrap();
    assert_eq!(connection_status(&store, now()).unwrap(), ConnectionStatus::NotConnected);
}

// ---- disconnect -----------------------------------------------------------------

fn connected() -> (Connection, MemoryStore) {
    let mut conn = mem_db();
    let store = MemoryStore::new();
    register(&mut conn, &store, &authorized(FAKE_SESSION, &[(FAKE_IBAN_A, FAKE_UID_A)]));
    (conn, store)
}

#[tokio::test]
async fn revoke_deletes_the_session_and_the_keyring_entry() {
    let f = fixture().await;
    let (conn, store) = connected();
    Mock::given(method("DELETE"))
        .and(path(format!("/sessions/{FAKE_SESSION}")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&f.server)
        .await;
    assert_eq!(revoke(&f.client, &store).await.unwrap(), RevokeResult::Revoked);
    assert!(store.get(SESSION_ENTRY).unwrap().is_none());
    assert_eq!(unlink_accounts(&conn).unwrap(), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM account_links"), 0);
    assert_eq!(count(&conn, "SELECT count(*) FROM accounts"), 1);
}

#[tokio::test]
async fn revoke_removes_the_keyring_entry_even_when_the_api_fails() {
    let f = fixture().await;
    let (_conn, store) = connected();
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "error": "ASPSP_ERROR" })))
        .mount(&f.server)
        .await;
    match revoke(&f.client, &store).await.unwrap() {
        RevokeResult::Failed { reason } => assert!(!reason.contains(FAKE_SESSION)),
        other => panic!("{other:?}"),
    }
    assert!(store.get(SESSION_ENTRY).unwrap().is_none());
}

#[tokio::test]
async fn revoke_treats_an_inactive_session_as_done() {
    let f = fixture().await;
    let (_conn, store) = connected();
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({ "error": "EXPIRED_SESSION" })))
        .mount(&f.server)
        .await;
    assert_eq!(revoke(&f.client, &store).await.unwrap(), RevokeResult::AlreadyInactive);
    assert!(store.get(SESSION_ENTRY).unwrap().is_none());
}

#[tokio::test]
async fn revoke_without_a_session_makes_no_request() {
    let f = fixture().await;
    let store = MemoryStore::new();
    assert_eq!(revoke(&f.client, &store).await.unwrap(), RevokeResult::NotConnected);
    assert!(f.server.received_requests().await.unwrap().is_empty());
}
