use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use super::error::EbError;

/// Redirect URL registered with Enable Banking and sent in `POST /auth`
/// (see ADR 0004). Must match byte for byte.
pub const REDIRECT_URL: &str = "https://nihilistblves.github.io/pocket-ledger/callback.html";

/// Production API origin.
pub const DEFAULT_BASE_URL: &str = "https://api.enablebanking.com";

/// Environment variable holding the path to the application's private key.
pub const KEY_PATH_ENV: &str = "ENABLE_BANKING_KEY_PATH";

/// Client configuration. The key is referenced by path only; its content is read
/// at signing time and never stored here.
#[derive(Clone)]
pub struct EbConfig {
    pub application_id: String,
    pub base_url: String,
    pub key_path: PathBuf,
}

impl EbConfig {
    /// Reads the key path from `ENABLE_BANKING_KEY_PATH`.
    pub fn from_env(application_id: impl Into<String>) -> Result<Self, EbError> {
        Self::from_lookup(application_id, |name| std::env::var_os(name))
    }

    pub(crate) fn from_lookup(
        application_id: impl Into<String>,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self, EbError> {
        match lookup(KEY_PATH_ENV) {
            Some(p) if !p.is_empty() => Ok(Self {
                application_id: application_id.into(),
                base_url: DEFAULT_BASE_URL.to_owned(),
                key_path: PathBuf::from(p),
            }),
            _ => Err(EbError::KeyNotConfigured),
        }
    }

    /// Builds a config from an explicit key path (e.g. one stored in the app settings).
    /// The file is not opened here; it is read only when a request is signed.
    pub fn new(application_id: impl Into<String>, key_path: impl Into<PathBuf>) -> Self {
        Self {
            application_id: application_id.into(),
            base_url: DEFAULT_BASE_URL.to_owned(),
            key_path: key_path.into(),
        }
    }

    /// Overrides the API origin (tests point this to a localhost mock).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }
}

impl fmt::Debug for EbConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EbConfig")
            .field("application_id", &self.application_id)
            .field("base_url", &self.base_url)
            .field("key_path", &"[redacted]")
            .finish()
    }
}
