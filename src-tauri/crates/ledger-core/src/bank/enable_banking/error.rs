use std::fmt;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkKind {
    Timeout,
    Connect,
    Other,
}

impl fmt::Display for NetworkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NetworkKind::Timeout => "timeout",
            NetworkKind::Connect => "connect",
            NetworkKind::Other => "other",
        })
    }
}

/// Machine-readable error code from the `error` field of an Enable Banking error body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiErrorCode {
    ExpiredSession,
    RevokedSession,
    ClosedSession,
    AspspRateLimitExceeded,
    AspspTimeout,
    AspspError,
    RedirectUriNotAllowed,
    Unauthorized,
    NotFound,
    /// Sanitized unknown code (`[A-Z0-9_]`, at most 64 chars) or `UNKNOWN`.
    Other(String),
}

const MAX_CODE_LEN: usize = 64;

impl ApiErrorCode {
    /// Maps the body's error code, falling back to the HTTP status when the body has
    /// no code. Unknown codes are kept only if they look like an enum identifier.
    pub(crate) fn from_response(status: u16, raw: Option<&str>) -> Self {
        match raw {
            Some("EXPIRED_SESSION") => Self::ExpiredSession,
            Some("REVOKED_SESSION") => Self::RevokedSession,
            Some("CLOSED_SESSION") => Self::ClosedSession,
            Some("ASPSP_RATE_LIMIT_EXCEEDED") => Self::AspspRateLimitExceeded,
            Some("ASPSP_TIMEOUT") => Self::AspspTimeout,
            Some("ASPSP_ERROR") => Self::AspspError,
            Some("REDIRECT_URI_NOT_ALLOWED") => Self::RedirectUriNotAllowed,
            Some("UNAUTHORIZED_ACCESS" | "AUTHORIZATION_NOT_PROVIDED" | "UNAUTHORIZED_IP") => {
                Self::Unauthorized
            }
            Some("SESSION_DOES_NOT_EXIST" | "ACCOUNT_DOES_NOT_EXIST") => Self::NotFound,
            Some(code) => Self::Other(sanitize(code)),
            None => match status {
                401 | 403 => Self::Unauthorized,
                404 => Self::NotFound,
                _ => Self::Other("UNKNOWN".to_owned()),
            },
        }
    }
}

fn sanitize(code: &str) -> String {
    let ok = !code.is_empty()
        && code.len() <= MAX_CODE_LEN
        && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        code.to_owned()
    } else {
        "UNKNOWN".to_owned()
    }
}

impl fmt::Display for ApiErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::ExpiredSession => "EXPIRED_SESSION",
            Self::RevokedSession => "REVOKED_SESSION",
            Self::ClosedSession => "CLOSED_SESSION",
            Self::AspspRateLimitExceeded => "ASPSP_RATE_LIMIT_EXCEEDED",
            Self::AspspTimeout => "ASPSP_TIMEOUT",
            Self::AspspError => "ASPSP_ERROR",
            Self::RedirectUriNotAllowed => "REDIRECT_URI_NOT_ALLOWED",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::NotFound => "NOT_FOUND",
            Self::Other(c) => c,
        };
        f.write_str(s)
    }
}

/// Errors of the Enable Banking client. No variant carries URLs, key paths, response
/// bodies or server messages.
#[derive(Debug, Error)]
pub enum EbError {
    #[error("Enable Banking private key path is not configured")]
    KeyNotConfigured,
    #[error("Enable Banking private key file not found")]
    KeyFileNotFound,
    #[error("Enable Banking private key is unreadable or invalid")]
    KeyInvalid,
    #[error("invalid client configuration")]
    InvalidConfig,
    #[error("network error ({kind})")]
    Network { kind: NetworkKind },
    #[error("Enable Banking API error: HTTP {status} ({code})")]
    Api { status: u16, code: ApiErrorCode },
    #[error("could not decode the response of {endpoint}")]
    Decode { endpoint: &'static str },
    #[error("pagination loop detected: continuation key repeated")]
    PaginationLoop,
    #[error("pagination limit exceeded")]
    PaginationLimit,
}

impl EbError {
    /// True when the user must go through the authorization flow again.
    pub fn requires_reauthorization(&self) -> bool {
        matches!(
            self,
            EbError::Api {
                code: ApiErrorCode::ExpiredSession
                    | ApiErrorCode::RevokedSession
                    | ApiErrorCode::ClosedSession,
                ..
            }
        )
    }

    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            EbError::Api { status: 429, .. }
                | EbError::Api { code: ApiErrorCode::AspspRateLimitExceeded, .. }
        )
    }
}

impl From<reqwest::Error> for EbError {
    fn from(e: reqwest::Error) -> Self {
        // `without_url` strips the URL (it contains session ids / account uids); we then
        // keep only the kind and drop the error text entirely.
        let e = e.without_url();
        let kind = if e.is_timeout() {
            NetworkKind::Timeout
        } else if e.is_connect() {
            NetworkKind::Connect
        } else {
            NetworkKind::Other
        };
        EbError::Network { kind }
    }
}
