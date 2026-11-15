//! Tests against a localhost wiremock server. All data is fictitious; the RSA key is
//! generated at test time and never committed.

use std::ffi::OsString;

use chrono::{NaiveDate, TimeZone, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::{json, Value};
use tempfile::TempDir;
use wiremock::matchers::{header_exists, method, path, query_param};
use wiremock::{Mock, Request, ResponseTemplate};

use super::jwt::{self, Claims};
use super::*;
use crate::bank::test_support::{fixture, keys, APP_ID};

const FAKE_IBAN: &str = "ES0000000000000000000000";
const FAKE_NAME: &str = "Ana Ficticia";
const FAKE_SESSION: &str = "11111111-aaaa-bbbb-cccc-222222222222";
const FAKE_UID: &str = "33333333-dddd-eeee-ffff-444444444444";
const FAKE_CODE: &str = "fake-auth-code-98765";

fn session_json() -> Value {
    json!({
        "session_id": FAKE_SESSION,
        "accounts": [{
            "uid": FAKE_UID,
            "account_id": { "iban": FAKE_IBAN },
            "currency": "EUR",
            "name": FAKE_NAME,
            "details": "Cuenta nomina",
            "product": "Current",
            "cash_account_type": "CACC",
            "identification_hash": "hash-fake"
        }],
        "aspsp": { "name": "Mock ASPSP", "country": "ES" },
        "psu_type": "personal",
        "access": { "valid_until": "2027-05-01T10:00:00Z" }
    })
}

fn tx_json(i: u32) -> Value {
    json!({
        "entry_reference": format!("ref-{i}"),
        "transaction_amount": { "currency": "EUR", "amount": "12.34" },
        "credit_debit_indicator": "DBIT",
        "status": "BOOK",
        "booking_date": "2026-11-01",
        "creditor": { "name": FAKE_NAME },
        "remittance_information": ["Pago ficticio"]
    })
}

fn error_body(code: &str) -> Value {
    json!({ "message": format!("secret detail {FAKE_IBAN}"), "code": 400, "error": code })
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

// ---- config ---------------------------------------------------------------------

#[test]
fn redirect_url_is_byte_exact() {
    assert_eq!(REDIRECT_URL, "https://nihilistblves.github.io/pocket-ledger/callback.html");
}

#[test]
fn config_from_lookup_requires_key_path() {
    let err = EbConfig::from_lookup(APP_ID, |_| None).unwrap_err();
    assert!(matches!(err, EbError::KeyNotConfigured));
    let err = EbConfig::from_lookup(APP_ID, |_| Some(OsString::new())).unwrap_err();
    assert!(matches!(err, EbError::KeyNotConfigured));

    let cfg = EbConfig::from_lookup(APP_ID, |n| {
        assert_eq!(n, KEY_PATH_ENV);
        Some(OsString::from("some/fake/key.pem"))
    })
    .unwrap();
    assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
    assert!(!format!("{cfg:?}").contains("fake"));
}

// ---- jwt ------------------------------------------------------------------------

#[tokio::test]
async fn jwt_has_expected_header_and_claims() {
    let f = fixture().await;
    let iat = Utc::now().timestamp();
    let signed = jwt::sign(&f.config, iat).unwrap();

    let header = jsonwebtoken::decode_header(&signed.token).unwrap();
    assert_eq!(header.alg, Algorithm::RS256);
    assert_eq!(header.typ.as_deref(), Some("JWT"));
    assert_eq!(header.kid.as_deref(), Some(APP_ID));

    let mut v = Validation::new(Algorithm::RS256);
    v.set_audience(&["api.enablebanking.com"]);
    v.set_issuer(&["enablebanking.com"]);
    v.set_required_spec_claims(&["exp", "iss", "aud"]);
    let key = DecodingKey::from_rsa_pem(keys().public_pem.as_bytes()).unwrap();
    let data = jsonwebtoken::decode::<Claims>(&signed.token, &key, &v).unwrap();
    assert_eq!(data.claims.iat, iat);
    assert_eq!(data.claims.exp, iat + 600);
    assert_eq!(signed.exp, iat + 600);
}

#[test]
fn key_errors_are_typed_and_leak_nothing() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("secret-location-marker").join("nokey.pem");
    let cfg = EbConfig {
        application_id: APP_ID.into(),
        base_url: DEFAULT_BASE_URL.into(),
        key_path: missing,
    };
    let e = jwt::sign(&cfg, 0).err().unwrap();
    assert!(matches!(e, EbError::KeyFileNotFound));
    assert!(!format!("{e} {e:?}").contains("secret-location-marker"));

    let bad = dir.path().join("bad.pem");
    std::fs::write(&bad, "CONTENT-MARKER not a pem").unwrap();
    let cfg = EbConfig { key_path: bad, ..cfg };
    let e = jwt::sign(&cfg, 0).err().unwrap();
    assert!(matches!(e, EbError::KeyInvalid));
    let shown = format!("{e} {e:?}");
    assert!(!shown.contains("CONTENT-MARKER") && !shown.contains("bad.pem"));
}

#[tokio::test]
async fn missing_key_file_makes_no_http_request() {
    let f = fixture().await;
    let dir = TempDir::new().unwrap();
    let cfg = EbConfig { key_path: dir.path().join("absent.pem"), ..f.config.clone() };
    let client = EbClient::new(cfg).unwrap();
    let err = client.list_aspsps("ES", PsuType::Personal).await.unwrap_err();
    assert!(matches!(err, EbError::KeyFileNotFound));
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

// ---- requests -------------------------------------------------------------------

#[tokio::test]
async fn bearer_token_is_sent_and_reused() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/aspsps"))
        .and(query_param("country", "ES"))
        .and(query_param("psu_type", "personal"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "aspsps": [{
                "name": "Mock ASPSP", "country": "ES", "maximum_consent_validity": 15552000,
                "psu_types": ["personal", "business"], "logo": "x", "auth_methods": [], "beta": false
            }]
        })))
        .mount(&f.server)
        .await;

    let list = f.client.list_aspsps("ES", PsuType::Personal).await.unwrap();
    f.client.list_aspsps("ES", PsuType::Personal).await.unwrap();
    assert_eq!(list[0].maximum_consent_validity, Some(15_552_000));
    assert_eq!(list[0].psu_types, vec![PsuType::Personal, PsuType::Business]);

    let reqs = f.server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 2);
    let auth: Vec<String> = reqs
        .iter()
        .map(|r| r.headers.get("authorization").unwrap().to_str().unwrap().to_owned())
        .collect();
    assert!(auth[0].starts_with("Bearer "));
    assert_eq!(auth[0], auth[1], "cached token reused");
}

#[tokio::test]
async fn start_auth_sends_expected_body() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/auth"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "url": format!("https://auth.example.test/ais/start?sessionid={FAKE_SESSION}"),
            "authorization_id": "55555555-0000-0000-0000-666666666666"
        })))
        .mount(&f.server)
        .await;
    let req = AuthRequest {
        aspsp_name: "Mock ASPSP".into(),
        aspsp_country: "ES".into(),
        valid_until: Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap(),
        state: State::new("state-value-xyz"),
        redirect_url: REDIRECT_URL.into(),
        psu_type: PsuType::Personal,
    };
    let resp = f.client.start_auth(&req).await.unwrap();
    assert!(resp.url.expose().contains("auth.example.test"));
    assert!(!format!("{resp:?}{req:?}").contains(FAKE_SESSION));
    assert!(!format!("{req:?}").contains("state-value-xyz"));

    let r = &f.server.received_requests().await.unwrap()[0];
    let body: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(body["aspsp"], json!({"name": "Mock ASPSP", "country": "ES"}));
    assert_eq!(body["state"], "state-value-xyz");
    assert_eq!(body["redirect_url"], REDIRECT_URL);
    assert_eq!(body["psu_type"], "personal");
    assert!(body["access"]["valid_until"].as_str().unwrap().starts_with("2027-01-01T00:00:00"));
}

#[tokio::test]
async fn session_lifecycle_and_account_endpoints() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(session_json()))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/sessions/{FAKE_SESSION}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            // GET /sessions/{id} has its own shape: account uids only, plus status.
            "status": "AUTHORIZED",
            "accounts": [FAKE_UID],
            "access": { "valid_until": "2027-05-01T10:00:00+00:00" },
            "psu_type": "personal"
        })))
        .mount(&f.server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("/sessions/{FAKE_SESSION}")))
        .respond_with(ResponseTemplate::new(204))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{FAKE_UID}/details")))
        .respond_with(ResponseTemplate::new(200).set_body_json(session_json()["accounts"][0].clone()))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{FAKE_UID}/balances")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "balances": [{
                "name": "Booked", "balance_type": "CLBD",
                "balance_amount": { "currency": "EUR", "amount": "1000.00" }
            }]
        })))
        .mount(&f.server)
        .await;

    let session = f.client.create_session(&AuthCode::new(FAKE_CODE)).await.unwrap();
    assert_eq!(session.session_id.expose(), FAKE_SESSION);
    assert_eq!(session.accounts.len(), 1);
    assert_eq!(session.accounts[0].uid.expose(), FAKE_UID);
    assert_eq!(session.access.valid_until, Utc.with_ymd_and_hms(2027, 5, 1, 10, 0, 0).unwrap());

    let shown = format!("{session:?}");
    for secret in [FAKE_IBAN, FAKE_NAME, FAKE_SESSION, FAKE_UID, "hash-fake"] {
        assert!(!shown.contains(secret), "Debug leaked {secret}");
    }

    let info = f.client.get_session(&session.session_id).await.unwrap();
    assert_eq!(info.status, "AUTHORIZED");
    assert_eq!(info.accounts.len(), 1);
    assert_eq!(info.accounts[0].expose(), FAKE_UID);
    assert!(!format!("{info:?}").contains(FAKE_UID));
    let acc = f.client.account_details(FAKE_UID).await.unwrap();
    assert_eq!(acc.account_id.unwrap().iban.unwrap().expose(), FAKE_IBAN);
    let bal = f.client.account_balances(FAKE_UID).await.unwrap();
    assert_eq!(bal[0].balance_amount.expose().amount, "1000.00");
    assert!(!format!("{bal:?}").contains("1000.00"));
    f.client.delete_session(&session.session_id).await.unwrap();

    let reqs = f.server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(body, json!({ "code": FAKE_CODE }));
}

// ---- pagination -----------------------------------------------------------------

#[tokio::test]
async fn transactions_follow_continuation_key_over_three_pages() {
    let f = fixture().await;
    let p = format!("/accounts/{FAKE_UID}/transactions");
    // Most specific mocks first (wiremock picks the first match in registration order).
    Mock::given(method("GET"))
        .and(path(p.clone()))
        .and(query_param("continuation_key", "k2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [tx_json(3)]
        })))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(p.clone()))
        .and(query_param("continuation_key", "k1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [tx_json(2)], "continuation_key": "k2"
        })))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(p))
        .and(query_param("date_from", "2026-10-01"))
        .and(query_param("date_to", "2026-11-01"))
        .and(query_param("transaction_status", "BOOK"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [tx_json(1)], "continuation_key": "k1"
        })))
        .mount(&f.server)
        .await;

    let txs = f
        .client
        .account_transactions(FAKE_UID, d(2026, 10, 1), d(2026, 11, 1), Some(TransactionStatus::Book))
        .await
        .unwrap();
    assert_eq!(txs.len(), 3);
    assert_eq!(txs[0].credit_debit_indicator, CreditDebitIndicator::Dbit);
    assert_eq!(txs[0].booking_date, Some(d(2026, 11, 1)));
    let shown = format!("{txs:?}");
    for secret in [FAKE_NAME, "12.34", "Pago ficticio", "ref-1"] {
        assert!(!shown.contains(secret), "Debug leaked {secret}");
    }
}

#[tokio::test]
async fn repeated_continuation_key_is_a_loop() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{FAKE_UID}/transactions")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "transactions": [tx_json(1)], "continuation_key": "same"
        })))
        .mount(&f.server)
        .await;
    let err = f
        .client
        .account_transactions(FAKE_UID, d(2026, 10, 1), d(2026, 11, 1), None)
        .await
        .unwrap_err();
    assert!(matches!(err, EbError::PaginationLoop));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn too_many_pages_is_an_error() {
    let f = fixture().await;
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = counter.clone();
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{FAKE_UID}/transactions")))
        .respond_with(move |_: &Request| {
            let n = c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(json!({
                "transactions": [], "continuation_key": format!("k{n}")
            }))
        })
        .mount(&f.server)
        .await;
    let err = f
        .client
        .account_transactions(FAKE_UID, d(2026, 10, 1), d(2026, 11, 1), None)
        .await
        .unwrap_err();
    assert!(matches!(err, EbError::PaginationLimit));
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 500);
}

// ---- retries --------------------------------------------------------------------

#[tokio::test]
async fn rate_limit_then_success() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/aspsps"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "0")
                .set_body_json(error_body("ASPSP_RATE_LIMIT_EXCEEDED")),
        )
        .up_to_n_times(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/aspsps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "aspsps": [] })))
        .mount(&f.server)
        .await;
    let list = f.client.list_aspsps("ES", PsuType::Personal).await.unwrap();
    assert!(list.is_empty());
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn persistent_503_exhausts_retries() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path("/aspsps"))
        .respond_with(ResponseTemplate::new(503).set_body_json(error_body("ASPSP_ERROR")))
        .mount(&f.server)
        .await;
    let err = f.client.list_aspsps("ES", PsuType::Personal).await.unwrap_err();
    assert!(matches!(err, EbError::Api { status: 503, code: ApiErrorCode::AspspError }));
    // 1 attempt + 4 retries.
    assert_eq!(f.server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test]
async fn post_sessions_is_not_retried_on_500() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(error_body("ASPSP_ERROR")))
        .mount(&f.server)
        .await;
    let err = f.client.create_session(&AuthCode::new(FAKE_CODE)).await.unwrap_err();
    assert!(matches!(err, EbError::Api { status: 500, .. }));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn post_sessions_is_retried_on_429() {
    let f = fixture().await;
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&f.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(session_json()))
        .mount(&f.server)
        .await;
    f.client.create_session(&AuthCode::new(FAKE_CODE)).await.unwrap();
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn connect_error_is_typed_without_url() {
    let f = fixture().await;
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let uri = format!("http://127.0.0.1:{port}"); // listener dropped: port closed
    let cfg = EbConfig { base_url: uri.clone(), ..f.config.clone() };
    let client = EbClient::new(cfg).unwrap().with_retry_policy(RetryPolicy {
        max_retries: 1,
        ..RetryPolicy::immediate()
    });
    let err = client.account_details(FAKE_UID).await.unwrap_err();
    assert!(matches!(err, EbError::Network { kind: NetworkKind::Connect }), "{err:?}");
    let shown = format!("{err} {err:?}");
    assert!(!shown.contains(FAKE_UID) && !shown.contains(&uri));
}

// ---- error mapping --------------------------------------------------------------

async fn api_code(status: u16, body: Value) -> EbError {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path(format!("/sessions/{FAKE_SESSION}")))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(&f.server)
        .await;
    f.client.get_session(&SessionId::new(FAKE_SESSION)).await.unwrap_err()
}

#[tokio::test]
async fn error_codes_are_mapped() {
    let e = api_code(401, error_body("EXPIRED_SESSION")).await;
    assert!(matches!(e, EbError::Api { status: 401, code: ApiErrorCode::ExpiredSession }));
    assert!(e.requires_reauthorization() && !e.is_rate_limited());

    let e = api_code(401, error_body("REVOKED_SESSION")).await;
    assert!(matches!(e, EbError::Api { code: ApiErrorCode::RevokedSession, .. }));
    assert!(e.requires_reauthorization());

    let e = api_code(401, error_body("CLOSED_SESSION")).await;
    assert!(e.requires_reauthorization());

    let e = api_code(400, error_body("REDIRECT_URI_NOT_ALLOWED")).await;
    assert!(matches!(e, EbError::Api { code: ApiErrorCode::RedirectUriNotAllowed, .. }));
    assert!(!e.requires_reauthorization());

    let e = api_code(404, json!({ "message": "gone" })).await;
    assert!(matches!(e, EbError::Api { status: 404, code: ApiErrorCode::NotFound }));

    let e = api_code(403, json!({ "message": "no" })).await;
    assert!(matches!(e, EbError::Api { code: ApiErrorCode::Unauthorized, .. }));
}

#[tokio::test]
async fn rate_limit_error_after_retries() {
    let e = api_code(429, error_body("ASPSP_RATE_LIMIT_EXCEEDED")).await;
    assert!(matches!(e, EbError::Api { status: 429, code: ApiErrorCode::AspspRateLimitExceeded }));
    assert!(e.is_rate_limited());
}

#[tokio::test]
async fn unknown_codes_are_sanitized_and_messages_dropped() {
    let e = api_code(422, error_body("WRONG_DATE_INTERVAL")).await;
    assert!(matches!(&e, EbError::Api { code: ApiErrorCode::Other(c), .. } if c == "WRONG_DATE_INTERVAL"));

    let evil = format!("bad code {FAKE_IBAN} \n");
    let e = api_code(422, json!({ "message": FAKE_IBAN, "error": evil })).await;
    assert!(matches!(&e, EbError::Api { code: ApiErrorCode::Other(c), .. } if c == "UNKNOWN"));
    let long = "A".repeat(65);
    assert!(matches!(
        ApiErrorCode::from_response(422, Some(&long)),
        ApiErrorCode::Other(c) if c == "UNKNOWN"
    ));
    for e in [e, api_code(500, error_body("ASPSP_ERROR")).await] {
        let shown = format!("{e} {e:?}");
        assert!(!shown.contains(FAKE_IBAN) && !shown.contains(FAKE_SESSION));
    }
}

#[tokio::test]
async fn decode_error_names_endpoint_not_content() {
    let f = fixture().await;
    Mock::given(method("GET"))
        .and(path(format!("/accounts/{FAKE_UID}/balances")))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("not json {FAKE_IBAN}")))
        .mount(&f.server)
        .await;
    let e = f.client.account_balances(FAKE_UID).await.unwrap_err();
    assert!(matches!(e, EbError::Decode { endpoint: "GET /accounts/{uid}/balances" }));
    let shown = format!("{e} {e:?}");
    assert!(!shown.contains(FAKE_IBAN) && !shown.contains(FAKE_UID));
}

#[test]
fn secret_newtypes_are_redacted() {
    let s = SessionId::new(FAKE_SESSION);
    let c = AuthCode::new(FAKE_CODE);
    let st = State::new("state-xyz");
    let shown = format!("{s:?} {c:?} {st:?}");
    for secret in [FAKE_SESSION, FAKE_CODE, "state-xyz"] {
        assert!(!shown.contains(secret));
    }
}
