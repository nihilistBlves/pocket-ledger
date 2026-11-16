//! `enable-banking.json` in the app data directory: which application and bank to
//! use. Not secret (ADR 0003). The private key is referenced by path only: the
//! `ENABLE_BANKING_KEY_PATH` environment variable (priority, used by the dev script) or
//! the optional `key_path` field set from Ajustes. The key file is never read here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ledger_core::bank::enable_banking::{EbClient, EbConfig, EbError, KEY_PATH_ENV};
use serde::Deserialize;
use serde_json::Value;

use crate::errors::{CommandError, ErrorCode};

pub const CONFIG_FILE: &str = "enable-banking.json";

#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct BankConfig {
    pub application_id: String,
    pub aspsp_name: String,
    #[serde(default = "default_country")]
    pub aspsp_country: String,
    /// Absolute path of the private key `.pem`, set from Ajustes. Never logged.
    #[serde(default)]
    pub key_path: Option<String>,
}

impl std::fmt::Debug for BankConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BankConfig")
            .field("application_id", &self.application_id)
            .field("aspsp_name", &self.aspsp_name)
            .field("aspsp_country", &self.aspsp_country)
            .field("key_path", &self.key_path.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

fn default_country() -> String {
    "ES".to_owned()
}

/// Where the private key path comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Env,
    Settings,
    None,
}

impl KeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            KeySource::Env => "env",
            KeySource::Settings => "settings",
            KeySource::None => "none",
        }
    }
}

/// Picks the key path: a non-empty environment value wins over the stored setting.
pub fn resolve_key_path(
    env: Option<OsString>,
    setting: Option<&str>,
) -> (KeySource, Option<PathBuf>) {
    match env {
        Some(p) if !p.is_empty() => (KeySource::Env, Some(PathBuf::from(p))),
        _ => match setting {
            Some(p) if !p.is_empty() => (KeySource::Settings, Some(PathBuf::from(p))),
            _ => (KeySource::None, None),
        },
    }
}

/// Parses and validates the file content. The error carries no content.
pub fn parse(text: &str) -> Result<BankConfig, ErrorCode> {
    let mut cfg: BankConfig = serde_json::from_str(text).map_err(|_| ErrorCode::NotConfigured)?;
    cfg.application_id = cfg.application_id.trim().to_owned();
    cfg.aspsp_name = cfg.aspsp_name.trim().to_owned();
    cfg.aspsp_country = cfg.aspsp_country.trim().to_ascii_uppercase();
    cfg.key_path = cfg.key_path.take().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty());
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
    /// Builds the API client. The key path comes from `ENABLE_BANKING_KEY_PATH` or, when
    /// that is unset or empty, from `key_path`; the key itself is only read when a
    /// request is signed.
    pub fn client(&self) -> Result<EbClient, CommandError> {
        self.client_with(std::env::var_os(KEY_PATH_ENV))
    }

    fn client_with(&self, env: Option<OsString>) -> Result<EbClient, CommandError> {
        let (_, path) = resolve_key_path(env, self.key_path.as_deref());
        let path = path.ok_or(EbError::KeyNotConfigured)?;
        let config = EbConfig::new(self.application_id.clone(), path);
        Ok(EbClient::new(config)?)
    }
}

// ------------------------------------------------------------ key path

/// What the UI may know about the key: never the full path or the content.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct KeyStatus {
    pub configured: bool,
    pub source: &'static str,
    pub file_name: Option<String>,
    pub exists: bool,
}

pub fn key_status_with(env: Option<OsString>, setting: Option<&str>) -> KeyStatus {
    let (source, path) = resolve_key_path(env, setting);
    match path {
        Some(p) => KeyStatus {
            configured: true,
            source: source.as_str(),
            file_name: p.file_name().map(|n| n.to_string_lossy().into_owned()),
            exists: p.is_file(),
        },
        None => KeyStatus { configured: false, source: "none", file_name: None, exists: false },
    }
}

/// Status from the real environment and the config file in `dir`.
pub fn key_status(dir: &Path) -> KeyStatus {
    let setting = load(dir).ok().and_then(|c| c.key_path);
    key_status_with(std::env::var_os(KEY_PATH_ENV), setting.as_deref())
}

/// The path must be absolute, an existing file and end in `.pem`. The file is not opened.
pub fn validate_key_path(path: &Path) -> Result<(), ErrorCode> {
    let pem =
        path.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("pem"));
    if !path.is_absolute() || !pem || !path.is_file() {
        return Err(ErrorCode::InvalidInput);
    }
    Ok(())
}

/// Sets (`Some`) or removes (`None`) `key_path`, keeping every other field. The file
/// must already hold a valid config (otherwise `NotConfigured`, nothing is created).
/// Written atomically: temp file in the same directory, then rename.
pub fn update_key_path(dir: &Path, key_path: Option<&Path>) -> Result<(), ErrorCode> {
    use std::io::Write;
    let target = dir.join(CONFIG_FILE);
    let text = std::fs::read_to_string(&target).map_err(|_| ErrorCode::NotConfigured)?;
    parse(&text)?;
    let mut value: Value = serde_json::from_str(&text).map_err(|_| ErrorCode::NotConfigured)?;
    let obj = value.as_object_mut().ok_or(ErrorCode::NotConfigured)?;
    match key_path {
        Some(p) => {
            let p = p.to_str().ok_or(ErrorCode::InvalidInput)?;
            obj.insert("key_path".to_owned(), Value::String(p.to_owned()));
        }
        None => {
            obj.remove("key_path");
        }
    }
    let out = serde_json::to_string_pretty(&value).map_err(|_| ErrorCode::Internal)?;
    let tmp = dir.join(format!("{CONFIG_FILE}.partial"));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &target)
    };
    write().map_err(|_| {
        let _ = std::fs::remove_file(&tmp);
        ErrorCode::Internal
    })
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
        assert_eq!(cfg.key_path, None);
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

    #[test]
    fn key_path_is_trimmed_and_empty_means_none() {
        let base = r#""application_id":"a","aspsp_name":"B""#;
        let with = parse(&format!(r#"{{{base},"key_path":"  /k/a.pem "}}"#)).unwrap();
        assert_eq!(with.key_path.as_deref(), Some("/k/a.pem"));
        for empty in [r#","key_path":"   ""#, r#","key_path":null"#, ""] {
            let cfg = parse(&format!("{{{base}{empty}}}")).unwrap();
            assert_eq!(cfg.key_path, None, "{empty}");
        }
    }

    #[test]
    fn resolution_priority_is_env_then_settings_then_none() {
        let os = |s: &str| Some(OsString::from(s));
        assert_eq!(
            resolve_key_path(os("/env.pem"), Some("/set.pem")),
            (KeySource::Env, Some(PathBuf::from("/env.pem")))
        );
        for empty_env in [None, os("")] {
            assert_eq!(
                resolve_key_path(empty_env, Some("/set.pem")),
                (KeySource::Settings, Some(PathBuf::from("/set.pem")))
            );
        }
        assert_eq!(resolve_key_path(None, None), (KeySource::None, None));
        assert_eq!(resolve_key_path(os(""), Some("")), (KeySource::None, None));
    }

    #[test]
    fn client_needs_a_key_path_from_somewhere() {
        let cfg = parse(r#"{"application_id":"a","aspsp_name":"B"}"#).unwrap();
        let err = cfg.client_with(None).err().unwrap();
        assert_eq!(err.code, ErrorCode::KeyMissing.as_str());
        // A path is enough to build the client: the file is only read when signing.
        let cfg =
            parse(r#"{"application_id":"a","aspsp_name":"B","key_path":"/k/a.pem"}"#).unwrap();
        assert!(cfg.client_with(None).is_ok());
    }

    #[test]
    fn debug_redacts_the_key_path() {
        let cfg =
            parse(r#"{"application_id":"a","aspsp_name":"B","key_path":"/secret/dir/a.pem"}"#)
                .unwrap();
        assert!(!format!("{cfg:?}").contains("secret"));
    }

    #[test]
    fn status_reports_only_the_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let pem = dir.path().join("my-key.pem");
        std::fs::write(&pem, b"fictitious").unwrap();
        let s = key_status_with(None, pem.to_str());
        assert_eq!(
            s,
            KeyStatus {
                configured: true,
                source: "settings",
                file_name: Some("my-key.pem".into()),
                exists: true
            }
        );
        let missing = dir.path().join("gone.pem");
        let s = key_status_with(Some(missing.into_os_string()), pem.to_str());
        assert_eq!((s.source, s.exists, s.file_name.as_deref()), ("env", false, Some("gone.pem")));
        let s = key_status_with(None, None);
        assert_eq!(
            s,
            KeyStatus { configured: false, source: "none", file_name: None, exists: false }
        );
    }

    #[test]
    fn validation_requires_an_absolute_existing_pem_file() {
        let dir = tempfile::tempdir().unwrap();
        let pem = dir.path().join("k.PEM");
        let txt = dir.path().join("k.txt");
        std::fs::write(&pem, b"x").unwrap();
        std::fs::write(&txt, b"x").unwrap();
        assert_eq!(validate_key_path(&pem), Ok(()));
        assert_eq!(validate_key_path(&txt), Err(ErrorCode::InvalidInput));
        assert_eq!(validate_key_path(&dir.path().join("none.pem")), Err(ErrorCode::InvalidInput));
        assert_eq!(validate_key_path(dir.path()), Err(ErrorCode::InvalidInput));
        assert_eq!(validate_key_path(Path::new("k.pem")), Err(ErrorCode::InvalidInput));
    }

    #[test]
    fn update_preserves_unknown_fields_and_clear_removes_the_field() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(CONFIG_FILE),
            r#"{"application_id":"a","aspsp_name":"B","aspsp_country":"ES","future":{"x":[1,2]}}"#,
        )
        .unwrap();
        let pem = dir.path().join("k.pem");
        update_key_path(dir.path(), Some(&pem)).unwrap();
        let read = || -> Value {
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap())
                .unwrap()
        };
        let v = read();
        assert_eq!(v["future"]["x"][1], 2);
        assert_eq!(v["application_id"], "a");
        assert_eq!(v["key_path"], pem.to_str().unwrap());
        assert_eq!(load(dir.path()).unwrap().key_path.as_deref(), pem.to_str());
        assert!(!dir.path().join(format!("{CONFIG_FILE}.partial")).exists());

        update_key_path(dir.path(), None).unwrap();
        let v = read();
        assert!(v.get("key_path").is_none());
        assert_eq!(v["future"]["x"][0], 1);
    }

    #[test]
    fn update_never_creates_or_fixes_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let pem = dir.path().join("k.pem");
        assert_eq!(update_key_path(dir.path(), Some(&pem)), Err(ErrorCode::NotConfigured));
        assert!(!dir.path().join(CONFIG_FILE).exists());
        std::fs::write(dir.path().join(CONFIG_FILE), "not json").unwrap();
        assert_eq!(update_key_path(dir.path(), None), Err(ErrorCode::NotConfigured));
        assert_eq!(std::fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap(), "not json");
    }
}
