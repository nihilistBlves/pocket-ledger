//! Enable Banking authorization flow and the stored connection.
//!
//! - [`authorize`] is the async network part: it opens the loopback listener, starts
//!   the authorization, waits for the redirect and exchanges the code for a session.
//!   It returns plain data and touches no database.
//! - [`register_session`] is the sync part: it stores the session in the OS secret
//!   store (the session id lives nowhere else) and links the bank accounts to local
//!   accounts in the database.
//! - [`connection_status`], [`revoke`] and [`unlink_accounts`] cover the rest of the
//!   connection lifecycle.
//!
//! Security notes: the session id, authorization code and state never reach logs,
//! errors or `Debug` output; account holder names are never stored.

use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

use super::callback::{CallbackListener, DEFAULT_PORT, DEFAULT_TIMEOUT};
use super::enable_banking::{
    Account, ApiErrorCode, AuthRequest, EbClient, EbError, PsuType, SessionId, State,
    REDIRECT_URL,
};
use super::db_timestamp;
use crate::keystore::{KeystoreError, SecretStore};

/// Keyring entry holding the [`StoredSession`] JSON.
pub const SESSION_ENTRY: &str = "enable-banking-session";
/// `account_links.provider` value.
pub const PROVIDER: &str = "enable_banking";
/// Longest consent we ask for, whatever the bank allows.
pub const MAX_VALIDITY_DAYS_CAP: u32 = 180;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("the local authorization port is already in use")]
    PortInUse,
    #[error("could not open the local authorization listener")]
    Bind,
    #[error("the bank was not found in the Enable Banking list")]
    AspspNotFound,
    #[error("could not generate a random state")]
    Random,
    #[error("could not open the system browser")]
    BrowserOpen,
    #[error("the authorization was denied ({code})")]
    Denied { code: String },
    #[error("the authorization response did not match this request")]
    StateMismatch,
    #[error("the authorization response had no code")]
    MissingCode,
    #[error("timed out waiting for the authorization")]
    Timeout,
    #[error(transparent)]
    Api(#[from] EbError),
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Store(#[from] KeystoreError),
    #[error("could not encode the session for the secret store")]
    Encode,
}

#[derive(Debug, Clone)]
pub struct AuthParams {
    pub aspsp_name: String,
    pub aspsp_country: String,
    pub max_validity_days_cap: u32,
    pub port: u16,
    pub timeout: Duration,
}

impl AuthParams {
    /// Production defaults: port 8080, 5 minute timeout, 180 day consent cap.
    pub fn new(aspsp_name: impl Into<String>, aspsp_country: impl Into<String>) -> Self {
        Self {
            aspsp_name: aspsp_name.into(),
            aspsp_country: aspsp_country.into(),
            max_validity_days_cap: MAX_VALIDITY_DAYS_CAP,
            port: DEFAULT_PORT,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// Result of a completed authorization. `Debug` shows no secrets (the types inside
/// redact themselves).
#[derive(Debug, Clone)]
pub struct AuthorizedSession {
    pub session_id: SessionId,
    pub valid_until: DateTime<Utc>,
    pub accounts: Vec<Account>,
}

/// Runs the whole authorization. The listener is bound first so a busy port fails
/// before anything is sent to the bank; `open_url` receives the bank's authorization
/// URL (the caller opens the system browser).
pub async fn authorize(
    client: &EbClient,
    params: AuthParams,
    open_url: impl FnOnce(&str) -> Result<(), String>,
) -> Result<AuthorizedSession, AuthError> {
    let listener = CallbackListener::bind(params.port).await?;

    let aspsps = client.list_aspsps(&params.aspsp_country, PsuType::Personal).await?;
    let aspsp = aspsps
        .into_iter()
        .find(|a| a.name == params.aspsp_name)
        .ok_or(AuthError::AspspNotFound)?;
    let cap_secs = u64::from(params.max_validity_days_cap) * 86_400;
    let secs = aspsp.maximum_consent_validity.map_or(cap_secs, |m| m.min(cap_secs));
    let valid_until = Utc::now() + chrono::Duration::seconds(i64::try_from(secs).unwrap_or(i64::MAX / 2));

    let state = State::generate().map_err(|_| AuthError::Random)?;
    let auth = client
        .start_auth(&AuthRequest {
            aspsp_name: params.aspsp_name.clone(),
            aspsp_country: params.aspsp_country.clone(),
            valid_until,
            state: state.clone(),
            redirect_url: REDIRECT_URL.to_owned(),
            psu_type: PsuType::Personal,
        })
        .await?;

    // The closure's error text is dropped: it could echo the URL.
    open_url(auth.url.expose()).map_err(|_| AuthError::BrowserOpen)?;

    let code = listener.wait_for_code(&state, params.timeout).await?;
    let session = client.create_session(&code).await?;
    Ok(AuthorizedSession {
        session_id: session.session_id.clone(),
        valid_until: session.access.valid_until,
        accounts: session.accounts.clone(),
    })
}

/// What is kept in the OS secret store about the connection. Serialized as JSON under
/// [`SESSION_ENTRY`].
#[derive(Serialize, Deserialize)]
pub struct StoredSession {
    session_id: String,
    pub valid_until: DateTime<Utc>,
    pub aspsp_name: String,
    pub aspsp_country: String,
    pub connected_at: DateTime<Utc>,
}

impl StoredSession {
    pub fn session_id(&self) -> SessionId {
        SessionId::new(self.session_id.clone())
    }
}

impl Drop for StoredSession {
    fn drop(&mut self) {
        self.session_id.zeroize();
    }
}

impl std::fmt::Debug for StoredSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSession")
            .field("session_id", &"[redacted]")
            .field("valid_until", &self.valid_until)
            .field("aspsp_name", &self.aspsp_name)
            .field("aspsp_country", &self.aspsp_country)
            .field("connected_at", &self.connected_at)
            .finish()
    }
}

/// Reads the stored session. A missing or unreadable entry is "not connected"; an
/// unreadable one is overwritten by the next authorization.
pub fn load_session(store: &dyn SecretStore) -> Result<Option<StoredSession>, KeystoreError> {
    let Some(bytes) = store.get(SESSION_ENTRY)? else {
        return Ok(None);
    };
    match serde_json::from_slice::<StoredSession>(&bytes) {
        Ok(s) => Ok(Some(s)),
        Err(_) => {
            log::warn!("stored bank session is unreadable; treating it as not connected");
            Ok(None)
        }
    }
}

#[derive(Debug)]
pub struct ConnectOutcome {
    pub accounts_linked: usize,
    /// Session accounts that could not be linked (no IBAN and no identification hash).
    pub accounts_skipped: usize,
    pub valid_until: DateTime<Utc>,
    /// Session id replaced by this one (renewal): the caller should best-effort
    /// [`revoke_previous`] it at the API.
    pub previous_session_id: Option<SessionId>,
}

/// Stores the session in the secret store and links its accounts. Reconnecting
/// replaces every previous `enable_banking` link (uids change per session) and keeps
/// the accounts, so history and imported data stay attached.
pub fn register_session(
    conn: &mut Connection,
    store: &dyn SecretStore,
    aspsp_name: &str,
    aspsp_country: &str,
    session: &AuthorizedSession,
    now: DateTime<Utc>,
) -> Result<ConnectOutcome, ConnectError> {
    let previous = load_session(store)?
        .map(|s| s.session_id())
        .filter(|p| p.expose() != session.session_id.expose());

    let valid_until_text = db_timestamp(session.valid_until);
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM account_links WHERE provider = ?1", [PROVIDER])?;
    let (mut linked, mut skipped) = (0, 0);
    for account in &session.accounts {
        let Some(identity) = AccountIdentity::from_account(account) else {
            skipped += 1;
            continue;
        };
        let currency = account
            .currency
            .as_deref()
            .filter(|c| c.len() == 3 && c.bytes().all(|b| b.is_ascii_alphabetic()))
            .map_or_else(|| "EUR".to_owned(), str::to_ascii_uppercase);
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM accounts WHERE identity_hash = ?1",
                [&identity.hash],
                |r| r.get(0),
            )
            .optional()?;
        let account_id = match existing {
            Some(id) => {
                tx.execute(
                    "UPDATE accounts SET institution = COALESCE(institution, ?1),
                                         display_suffix = COALESCE(display_suffix, ?2)
                     WHERE id = ?3",
                    params![aspsp_name, identity.suffix, id],
                )?;
                id
            }
            None => {
                let name = default_name(account.product.as_deref(), identity.suffix.as_deref());
                tx.execute(
                    "INSERT INTO accounts (name, institution, kind, currency, identity_hash, display_suffix)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        name,
                        aspsp_name,
                        kind_of(account.cash_account_type.as_deref()),
                        currency,
                        identity.hash,
                        identity.suffix
                    ],
                )?;
                tx.last_insert_rowid()
            }
        };
        // `session_ref` stays NULL: the session id lives only in the secret store.
        tx.execute(
            "INSERT OR REPLACE INTO account_links (account_id, provider, external_uid, valid_until)
             VALUES (?1, ?2, ?3, ?4)",
            params![account_id, PROVIDER, account.uid.expose(), valid_until_text],
        )?;
        linked += 1;
    }
    tx.commit()?;

    let stored = StoredSession {
        session_id: session.session_id.expose().to_owned(),
        valid_until: session.valid_until,
        aspsp_name: aspsp_name.to_owned(),
        aspsp_country: aspsp_country.to_owned(),
        connected_at: now,
    };
    let json = Zeroizing::new(serde_json::to_vec(&stored).map_err(|_| ConnectError::Encode)?);
    store.set(SESSION_ENTRY, &json)?;

    Ok(ConnectOutcome {
        accounts_linked: linked,
        accounts_skipped: skipped,
        valid_until: session.valid_until,
        previous_session_id: previous,
    })
}

/// Stable identity of a bank account: the SHA-256 (hex) of the normalized IBAN, so a
/// bank-synced account and an imported one with the same IBAN are the same row. The
/// IBAN itself is never stored; only its last 4 characters are kept for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub hash: String,
    pub suffix: Option<String>,
}

impl AccountIdentity {
    /// `iban` may contain spaces and lower case.
    pub fn from_iban(iban: &str) -> Option<Self> {
        let normalized: String = iban
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        if normalized.is_empty() {
            return None;
        }
        let suffix: String =
            normalized.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        Some(Self { hash: hex::encode(Sha256::digest(normalized.as_bytes())), suffix: Some(suffix) })
    }

    fn from_identification_hash(value: &str) -> Option<Self> {
        let value = value.trim();
        (!value.is_empty()).then(|| Self {
            hash: hex::encode(Sha256::digest(format!("eb-identification:{value}").as_bytes())),
            suffix: None,
        })
    }

    fn from_account(account: &Account) -> Option<Self> {
        account
            .account_id
            .as_ref()
            .and_then(|a| a.iban.as_ref())
            .and_then(|iban| Self::from_iban(iban.expose()))
            .or_else(|| {
                account
                    .identification_hash
                    .as_ref()
                    .and_then(|h| Self::from_identification_hash(h.expose()))
            })
    }
}

fn kind_of(cash_account_type: Option<&str>) -> &'static str {
    match cash_account_type {
        Some("CACC") => "checking",
        Some("SVGS") => "savings",
        Some("CARD") => "card",
        _ => "other",
    }
}

/// The product name if the bank gives one, else a generic name with the IBAN tail.
/// The holder name is never used.
fn default_name(product: Option<&str>, suffix: Option<&str>) -> String {
    match product.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.chars().take(80).collect(),
        None => match suffix {
            Some(s) => format!("Cuenta \u{b7}\u{b7}\u{b7}\u{b7}{s}"),
            None => "Cuenta".to_owned(),
        },
    }
}

/// Connection state shown to the user. Carries no secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    NotConnected,
    Connected { aspsp_name: String, valid_until: DateTime<Utc>, days_remaining: i64 },
    Expired { aspsp_name: String, expired_on: NaiveDate },
}

pub fn connection_status(
    store: &dyn SecretStore,
    now: DateTime<Utc>,
) -> Result<ConnectionStatus, KeystoreError> {
    Ok(match load_session(store)? {
        None => ConnectionStatus::NotConnected,
        Some(s) if s.valid_until <= now => ConnectionStatus::Expired {
            aspsp_name: s.aspsp_name.clone(),
            expired_on: s.valid_until.date_naive(),
        },
        Some(s) => ConnectionStatus::Connected {
            aspsp_name: s.aspsp_name.clone(),
            valid_until: s.valid_until,
            days_remaining: (s.valid_until - now).num_days(),
        },
    })
}

/// How the best-effort `DELETE /sessions/{id}` went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevokeResult {
    /// There was no stored session.
    NotConnected,
    Revoked,
    /// The bank side was already gone (expired, revoked, closed or unknown).
    AlreadyInactive,
    /// Could not reach or use the API; the local connection is removed regardless.
    /// `reason` is a short code, never content.
    Failed { reason: String },
}

/// Best-effort revocation of a session at the API.
pub async fn revoke_previous(client: &EbClient, session_id: &SessionId) -> RevokeResult {
    match client.delete_session(session_id).await {
        Ok(()) => RevokeResult::Revoked,
        Err(EbError::Api {
            code:
                ApiErrorCode::NotFound
                | ApiErrorCode::ExpiredSession
                | ApiErrorCode::RevokedSession
                | ApiErrorCode::ClosedSession,
            ..
        }) => RevokeResult::AlreadyInactive,
        Err(e) => RevokeResult::Failed { reason: e.to_string() },
    }
}

/// Disconnects: revokes the session at the API (best effort) and always removes the
/// secret-store entry. Pair with [`unlink_accounts`] for the database side.
pub async fn revoke(
    client: &EbClient,
    store: &dyn SecretStore,
) -> Result<RevokeResult, ConnectError> {
    let result = match load_session(store)? {
        None => RevokeResult::NotConnected,
        Some(stored) => revoke_previous(client, &stored.session_id()).await,
    };
    store.delete(SESSION_ENTRY)?;
    Ok(result)
}

/// Removes the provider links. Accounts and transactions are kept.
pub fn unlink_accounts(conn: &Connection) -> Result<usize, rusqlite::Error> {
    conn.execute("DELETE FROM account_links WHERE provider = ?1", [PROVIDER])
}

#[cfg(test)]
mod tests;
