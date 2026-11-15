//! Shared helpers for the bank tests: a generated test RSA key and a client pointed at
//! a localhost wiremock server. All data is fictitious; the key never leaves memory
//! and the temp directory.

use std::sync::OnceLock;

use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::PrivateKey;
use tempfile::TempDir;
use wiremock::MockServer;

use super::enable_banking::{EbClient, EbConfig, RetryPolicy};

pub const APP_ID: &str = "00000000-test-app";

pub struct TestKeys {
    pub private_pem: String,
    pub public_pem: String,
}

pub fn keys() -> &'static TestKeys {
    static KEYS: OnceLock<TestKeys> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut rng = rsa::rand_core::UnwrapErr(getrandom::SysRng);
        let key = PrivateKey::new(&mut rng, 2048).expect("keygen");
        TestKeys {
            private_pem: key.to_pkcs8_pem(LineEnding::LF).expect("pem").to_string(),
            public_pem: key.to_public_key().to_public_key_pem(LineEnding::LF).expect("pem"),
        }
    })
}

pub struct Fixture {
    pub server: MockServer,
    pub client: EbClient,
    pub config: EbConfig,
    pub _dir: TempDir,
}

pub async fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let key_path = dir.path().join("test-key.pem");
    std::fs::write(&key_path, &keys().private_pem).unwrap();
    let server = MockServer::start().await;
    let config = EbConfig {
        application_id: APP_ID.to_owned(),
        base_url: server.uri(),
        key_path,
    };
    let client = EbClient::new(config.clone()).unwrap().with_retry_policy(RetryPolicy::immediate());
    Fixture { server, client, config, _dir: dir }
}

// ---- fictitious bank data ---------------------------------------------------------

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::Connection;
use serde_json::json;

use super::connect::AuthorizedSession;
use super::enable_banking::Session;
use crate::db::{open_in_memory, DbKey, KEY_LEN};

pub const FAKE_IBAN_A: &str = "ES0000000000000000000001";
pub const FAKE_IBAN_B: &str = "ES0000000000000000000002";
pub const FAKE_NAME: &str = "Ana Ficticia";
pub const FAKE_SESSION: &str = "11111111-aaaa-bbbb-cccc-222222222222";
pub const FAKE_UID_A: &str = "uid-aaaa-0001";
pub const FAKE_UID_B: &str = "uid-bbbb-0002";
pub const FAKE_CODE: &str = "fake-auth-code-98765";

/// The "current" instant of the sync tests.
pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 15, 10, 0, 0).unwrap()
}

pub fn mem_db() -> Connection {
    open_in_memory(&DbKey::from_bytes([0x33; KEY_LEN])).expect("db")
}

/// `POST /sessions` response body for the given `(iban, uid)` accounts.
pub fn session_body(session_id: &str, accounts: &[(&str, &str)]) -> serde_json::Value {
    let accounts: Vec<_> = accounts
        .iter()
        .map(|(iban, uid)| {
            json!({
                "uid": uid,
                "account_id": { "iban": iban },
                "currency": "EUR",
                "name": FAKE_NAME,
                "product": "Cuenta ficticia",
                "cash_account_type": "CACC"
            })
        })
        .collect();
    json!({
        "session_id": session_id,
        "accounts": accounts,
        "access": { "valid_until": "2027-05-01T10:00:00Z" }
    })
}

pub fn authorized(session_id: &str, accounts: &[(&str, &str)]) -> AuthorizedSession {
    let s: Session = serde_json::from_value(session_body(session_id, accounts)).unwrap();
    AuthorizedSession {
        session_id: s.session_id,
        valid_until: s.access.valid_until,
        accounts: s.accounts,
    }
}
