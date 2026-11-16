use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{NaiveDate, Utc};
use reqwest::{Method, Url};
use serde::de::DeserializeOwned;
use serde_json::json;
use zeroize::Zeroizing;

use super::config::EbConfig;
use super::error::{ApiErrorCode, EbError, NetworkKind};
use super::jwt;
use super::retry::{network_retryable, status_retryable, Idempotency, RetryPolicy};
use super::types::{
    Account, AspspsPage, AuthCode, AuthRequest, AuthResponse, Aspsp, Balance, BalancesPage,
    ErrorBody, PsuType, Session, SessionId, SessionInfo, Transaction, TransactionStatus, TransactionsPage,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Reuse a cached token until this many seconds before it expires.
const TOKEN_REFRESH_MARGIN_SECS: i64 = 60;
const MAX_PAGES: usize = 500;

struct CachedToken {
    token: Zeroizing<String>,
    exp: i64,
}

/// Enable Banking API client.
pub struct EbClient {
    config: EbConfig,
    http: reqwest::Client,
    base: Url,
    retry: RetryPolicy,
    token: Mutex<Option<CachedToken>>,
}

impl std::fmt::Debug for EbClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EbClient").field("config", &self.config).finish_non_exhaustive()
    }
}

/// One endpoint: method, log/error template and idempotency.
#[derive(Clone, Copy)]
struct Endpoint {
    method: &'static Method,
    template: &'static str,
    idem: Idempotency,
}

const GET: &Method = &Method::GET;
const POST: &Method = &Method::POST;
const DELETE: &Method = &Method::DELETE;

const EP_ASPSPS: Endpoint =
    Endpoint { method: GET, template: "GET /aspsps", idem: Idempotency::Idempotent };
const EP_AUTH: Endpoint =
    Endpoint { method: POST, template: "POST /auth", idem: Idempotency::NonIdempotent };
const EP_SESSIONS: Endpoint =
    Endpoint { method: POST, template: "POST /sessions", idem: Idempotency::NonIdempotent };
const EP_GET_SESSION: Endpoint = Endpoint {
    method: GET,
    template: "GET /sessions/{session_id}",
    idem: Idempotency::Idempotent,
};
const EP_DEL_SESSION: Endpoint = Endpoint {
    method: DELETE,
    template: "DELETE /sessions/{session_id}",
    idem: Idempotency::Idempotent,
};
const EP_DETAILS: Endpoint = Endpoint {
    method: GET,
    template: "GET /accounts/{uid}/details",
    idem: Idempotency::Idempotent,
};
const EP_BALANCES: Endpoint = Endpoint {
    method: GET,
    template: "GET /accounts/{uid}/balances",
    idem: Idempotency::Idempotent,
};
const EP_TRANSACTIONS: Endpoint = Endpoint {
    method: GET,
    template: "GET /accounts/{uid}/transactions",
    idem: Idempotency::Idempotent,
};

struct Raw {
    status: u16,
    retry_after: Option<Duration>,
    body: Vec<u8>,
}

impl EbClient {
    pub fn new(config: EbConfig) -> Result<Self, EbError> {
        let base = Url::parse(&config.base_url).map_err(|_| EbError::InvalidConfig)?;
        if base.cannot_be_a_base() {
            return Err(EbError::InvalidConfig);
        }
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            // Never follow redirects: the bearer token must only reach the API origin.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| EbError::InvalidConfig)?;
        Ok(Self {
            config,
            http,
            base,
            retry: RetryPolicy::default(),
            token: Mutex::new(None),
        })
    }

    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    // ---- public API -------------------------------------------------------------

    pub async fn list_aspsps(
        &self,
        country: &str,
        psu_type: PsuType,
    ) -> Result<Vec<Aspsp>, EbError> {
        let query = [("country", country.to_owned()), ("psu_type", psu_type.as_str().to_owned())];
        let page: AspspsPage = self.call(EP_ASPSPS, &["aspsps"], &query, None).await?;
        Ok(page.aspsps)
    }

    pub async fn start_auth(&self, req: &AuthRequest) -> Result<AuthResponse, EbError> {
        let body = json!({
            "access": { "valid_until": req.valid_until },
            "aspsp": { "name": req.aspsp_name, "country": req.aspsp_country },
            "state": req.state.expose(),
            "redirect_url": req.redirect_url,
            "psu_type": req.psu_type.as_str(),
        });
        self.call(EP_AUTH, &["auth"], &[], Some(body)).await
    }

    pub async fn create_session(&self, code: &AuthCode) -> Result<Session, EbError> {
        let body = json!({ "code": code.expose() });
        self.call(EP_SESSIONS, &["sessions"], &[], Some(body)).await
    }

    pub async fn get_session(&self, session_id: &SessionId) -> Result<SessionInfo, EbError> {
        self.call(EP_GET_SESSION, &["sessions", session_id.expose()], &[], None).await
    }

    pub async fn delete_session(&self, session_id: &SessionId) -> Result<(), EbError> {
        self.send(EP_DEL_SESSION, &["sessions", session_id.expose()], &[], None).await?;
        Ok(())
    }

    pub async fn account_details(&self, uid: &str) -> Result<Account, EbError> {
        self.call(EP_DETAILS, &["accounts", uid, "details"], &[], None).await
    }

    pub async fn account_balances(&self, uid: &str) -> Result<Vec<Balance>, EbError> {
        let page: BalancesPage =
            self.call(EP_BALANCES, &["accounts", uid, "balances"], &[], None).await?;
        Ok(page.balances)
    }

    /// Fetches every page of transactions, following `continuation_key`.
    pub async fn account_transactions(
        &self,
        uid: &str,
        date_from: NaiveDate,
        date_to: NaiveDate,
        transaction_status: Option<TransactionStatus>,
    ) -> Result<Vec<Transaction>, EbError> {
        let mut all = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut key: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut query = vec![
                ("date_from", date_from.to_string()),
                ("date_to", date_to.to_string()),
            ];
            if let Some(s) = transaction_status {
                query.push(("transaction_status", s.as_str().to_owned()));
            }
            if let Some(k) = &key {
                query.push(("continuation_key", k.clone()));
            }
            let page: TransactionsPage = self
                .call(EP_TRANSACTIONS, &["accounts", uid, "transactions"], &query, None)
                .await?;
            all.extend(page.transactions);
            match page.continuation_key.filter(|k| !k.is_empty()) {
                None => return Ok(all),
                Some(next) => {
                    if !seen.insert(next.clone()) {
                        return Err(EbError::PaginationLoop);
                    }
                    key = Some(next);
                }
            }
        }
        Err(EbError::PaginationLimit)
    }

    // ---- plumbing ---------------------------------------------------------------

    async fn call<T: DeserializeOwned>(
        &self,
        ep: Endpoint,
        segments: &[&str],
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
    ) -> Result<T, EbError> {
        let raw = self.send(ep, segments, query, body).await?;
        serde_json::from_slice(&raw.body).map_err(|_| EbError::Decode { endpoint: ep.template })
    }

    fn url(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Url, EbError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| EbError::InvalidConfig)?
            .pop_if_empty()
            .extend(segments);
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        Ok(url)
    }

    /// Returns a valid bearer token, signing a new one when the cached one is close
    /// to expiry. The key file is only touched when a new token is needed.
    fn bearer(&self) -> Result<Zeroizing<String>, EbError> {
        let now = Utc::now().timestamp();
        let mut guard = self.token.lock().map_err(|_| EbError::InvalidConfig)?;
        if let Some(c) = guard.as_ref() {
            if c.exp - TOKEN_REFRESH_MARGIN_SECS > now {
                return Ok(c.token.clone());
            }
        }
        let signed = jwt::sign(&self.config, now)?;
        let token = signed.token.clone();
        *guard = Some(CachedToken { token: signed.token, exp: signed.exp });
        Ok(token)
    }

    async fn send(
        &self,
        ep: Endpoint,
        segments: &[&str],
        query: &[(&str, String)],
        body: Option<serde_json::Value>,
    ) -> Result<Raw, EbError> {
        let url = self.url(segments, query)?;
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let token = self.bearer()?;
            let started = Instant::now();
            let outcome = self.once(ep, &url, &token, body.as_ref()).await;
            let elapsed_ms = started.elapsed().as_millis();

            match outcome {
                Ok(raw) if (200..300).contains(&raw.status) => {
                    log::debug!(
                        "{} -> {} (attempt {}, {} ms)",
                        ep.template, raw.status, attempt, elapsed_ms
                    );
                    return Ok(raw);
                }
                Ok(raw) => {
                    // Only the sanitized error code is logged, never `message` or `detail`.
                    let code = match api_error(raw.status, &raw.body) {
                        EbError::Api { code, .. } => code.to_string(),
                        _ => String::new(),
                    };
                    log::warn!(
                        "{} -> {} {} (attempt {}, {} ms)",
                        ep.template, raw.status, code, attempt, elapsed_ms
                    );
                    if attempt <= self.retry.max_retries && status_retryable(raw.status, ep.idem) {
                        tokio::time::sleep(self.retry.delay(attempt, raw.retry_after)).await;
                        continue;
                    }
                    return Err(api_error(raw.status, &raw.body));
                }
                Err(err) => {
                    let kind = match &err {
                        EbError::Network { kind } => *kind,
                        _ => NetworkKind::Other,
                    };
                    log::warn!(
                        "{} -> network error {} (attempt {}, {} ms)",
                        ep.template, kind, attempt, elapsed_ms
                    );
                    if attempt <= self.retry.max_retries && network_retryable(kind, ep.idem) {
                        tokio::time::sleep(self.retry.delay(attempt, None)).await;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
    }

    async fn once(
        &self,
        ep: Endpoint,
        url: &Url,
        token: &Zeroizing<String>,
        body: Option<&serde_json::Value>,
    ) -> Result<Raw, EbError> {
        let mut req = self
            .http
            .request(ep.method.clone(), url.clone())
            .bearer_auth(token.as_str())
            .header(reqwest::header::ACCEPT, "application/json");
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = resp.bytes().await?.to_vec();
        Ok(Raw { status, retry_after, body })
    }
}

/// Builds an API error keeping only the sanitized error code of the body; the
/// `message` and `detail` fields are discarded.
fn api_error(status: u16, body: &[u8]) -> EbError {
    let raw = serde_json::from_slice::<ErrorBody>(body).ok().and_then(|b| b.error);
    EbError::Api { status, code: ApiErrorCode::from_response(status, raw.as_deref()) }
}
