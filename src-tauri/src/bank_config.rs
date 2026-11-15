//! `enable-banking.json` in the app data directory: which application and bank to
//! use. Not secret (ADR 0003); the private key path comes from the environment.

use std::path::Path;

use ledger_core::bank::enable_banking::{EbClient, EbConfig};
use serde::Deserialize;

use crate::errors::{CommandError, ErrorCode};

pub const CONFIG_FILE: &str = "enable-banking.json";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BankConfig {
    pub application_id: String,
    pub aspsp_name: String,
    #[serde(default = "default_country")]
    pub aspsp_country: String,
}

fn default_country() -> String {
    "ES".to_owned()
}

/// Parses and validates the file content. The error carries no content.
pub fn parse(text: &str) -> Result<BankConfig, ErrorCode> {
    let mut cfg: BankConfig = serde_json::from_str(text).map_err(|_| ErrorCode::NotConfigured)?;
    cfg.application_id = cfg.application_id.trim().to_owned();
    cfg.aspsp_name = cfg.aspsp_name.trim().to_owned();
    cfg.aspsp_country = cfg.aspsp_country.trim().to_ascii_uppercase();
    let country_ok =
        cfg.aspsp_country.len() == 2 && cfg.aspsp_country.bytes().all(|b| b.is_ascii_alphabetic());
    if cfg.application_id.is_empty() || cfg.aspsp_name.is_empty() || !country_ok {
        return Err(ErrorCode::NotConfigured);
    }
    Ok(cfg)
}

/// Reads the config from `dir`. Missing, unreadable or invalid is `NotConfigured`.
pub fn load(dir: &Path) -> Result<BankConfig, ErrorCode> {
    let text =
        std::fs::read_to_string(dir.join(CONFIG_FILE)).map_err(|_| ErrorCode::NotConfigured)?;
    parse(&text)
}

impl BankConfig {
    /// Builds the API client (reads `ENABLE_BANKING_KEY_PATH`; the key itself is only
    /// read when a request is signed).
    pub fn client(&self) -> Result<EbClient, CommandError> {
        let config = EbConfig::from_env(self.application_id.clone())?;
        Ok(EbClient::new(config)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_config() {
        let cfg =
            parse(r#"{"application_id":"app-1","aspsp_name":"Mock Bank","aspsp_country":"es"}"#)
                .unwrap();
        assert_eq!(cfg.aspsp_country, "ES");
        assert_eq!(cfg.aspsp_name, "Mock Bank");
    }

    #[test]
    fn country_defaults_to_spain_and_unknown_fields_are_ignored() {
        let cfg =
            parse(r#"{"application_id":" app-1 ","aspsp_name":"Mock Bank","extra":1}"#).unwrap();
        assert_eq!(cfg.aspsp_country, "ES");
        assert_eq!(cfg.application_id, "app-1");
    }

    #[test]
    fn rejects_invalid_content() {
        for bad in [
            "",
            "not json",
            "{}",
            r#"{"application_id":"","aspsp_name":"Mock Bank"}"#,
            r#"{"application_id":"a","aspsp_name":"  "}"#,
            r#"{"application_id":"a","aspsp_name":"B","aspsp_country":"ESP"}"#,
            r#"{"application_id":"a","aspsp_name":"B","aspsp_country":"1!"}"#,
        ] {
            assert_eq!(parse(bad), Err(ErrorCode::NotConfigured), "{bad}");
        }
    }

    #[test]
    fn load_reads_the_file_or_reports_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Err(ErrorCode::NotConfigured));
        std::fs::write(dir.path().join(CONFIG_FILE), r#"{"application_id":"a","aspsp_name":"B"}"#)
            .unwrap();
        assert_eq!(load(dir.path()).unwrap().aspsp_name, "B");
    }
}
