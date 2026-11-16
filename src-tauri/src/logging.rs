//! Application logger: a size-rotated file (plus stdout in debug builds), with every
//! line passed through `ledger_core::redact` (ADR 0003).
//!
//! - Where: `%LOCALAPPDATA%\dev.pocketledger.app\logs\pocket-ledger.log` (the app log
//!   dir), or `<data dir>\logs` when the debug-only `POCKET_LEDGER_DATA_DIR` override
//!   is set. 1 MB per file, the 5 newest rotated files are kept.
//! - Level: `info`. Debug builds read `POCKET_LEDGER_LOG` (`error`, `warn`, `info`,
//!   `debug`, `trace`). Third-party crates (HTTP stack, webview) never go below
//!   `info`, so their request/response tracing cannot reach the file.

use std::path::PathBuf;

use log::{Level, LevelFilter, Metadata};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

const FILE_NAME: &str = "pocket-ledger";
const MAX_FILE_BYTES: u128 = 1_000_000;
const KEEP_FILES: usize = 5;
/// Crates allowed to log below `info` when a more verbose level is configured.
const OWN_TARGETS: [&str; 3] = ["pocket_ledger", "pocket_ledger_lib", "ledger_core"];

/// The configured level: `info`, or `POCKET_LEDGER_LOG` in debug builds.
fn level() -> LevelFilter {
    if cfg!(debug_assertions) {
        std::env::var("POCKET_LEDGER_LOG")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(LevelFilter::Info)
    } else {
        LevelFilter::Info
    }
}

/// Only our own crates may log below `info`.
fn allowed(metadata: &Metadata) -> bool {
    metadata.level() <= Level::Info
        || OWN_TARGETS
            .iter()
            .any(|t| metadata.target() == *t || metadata.target().starts_with(&format!("{t}::")))
}

/// One output line. The message is redacted after formatting, so values that reach a
/// log call by mistake (an IBAN, a token, a session id) are masked in the file.
fn format_line(timestamp: &str, level: Level, target: &str, message: &str) -> String {
    let line = format!("{timestamp} {level:<5} {target}: {message}");
    ledger_core::redact::redact(&line).into_owned()
}

/// Builds the logger plugin writing to `log_dir`.
pub fn plugin<R: tauri::Runtime>(log_dir: PathBuf) -> tauri::plugin::TauriPlugin<R> {
    let mut targets = vec![Target::new(TargetKind::Folder {
        path: log_dir,
        file_name: Some(FILE_NAME.to_owned()),
    })];
    if cfg!(debug_assertions) {
        targets.push(Target::new(TargetKind::Stdout));
    }
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets(targets)
        .level(level())
        .filter(allowed)
        .max_file_size(MAX_FILE_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(KEEP_FILES))
        .timezone_strategy(TimezoneStrategy::UseUtc)
        .format(|out, message, record| {
            let timestamp = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            out.finish(format_args!(
                "{}",
                format_line(&timestamp, record.level(), record.target(), &message.to_string())
            ));
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(level: Level, target: &str) -> Metadata<'_> {
        Metadata::builder().level(level).target(target).build()
    }

    #[test]
    fn lines_are_redacted() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJ4In0.c2ln";
        let line = format_line(
            "2026-11-16T10:00:00Z",
            Level::Warn,
            "ledger_core::bank",
            &format!(
                "leak ES00 0000 0000 0000 0000 0001 {jwt} /sessions/0f1e2d3c-4b5a-6978-8a9b-0c1d2e3f4a5b"
            ),
        );
        assert_eq!(
            line,
            "2026-11-16T10:00:00Z WARN  ledger_core::bank: leak [iban] [jwt] /sessions/[uuid]"
        );
    }

    #[test]
    fn allowed_lines_are_kept_verbatim() {
        let line = format_line(
            "2026-11-16T10:00:00Z",
            Level::Info,
            "ledger_core::bank::enable_banking::client",
            "GET /accounts/{uid}/balances -> 200 (attempt 1, 87 ms)",
        );
        assert!(line.ends_with("GET /accounts/{uid}/balances -> 200 (attempt 1, 87 ms)"));
    }

    #[test]
    fn third_party_crates_stay_at_info() {
        assert!(allowed(&meta(Level::Debug, "ledger_core::bank")));
        assert!(allowed(&meta(Level::Trace, "pocket_ledger_lib::scheduler")));
        assert!(allowed(&meta(Level::Trace, "pocket_ledger")));
        assert!(!allowed(&meta(Level::Debug, "reqwest::connect")));
        assert!(!allowed(&meta(Level::Trace, "hyper_util")));
        assert!(!allowed(&meta(Level::Debug, "ledger_core_extra")));
        assert!(allowed(&meta(Level::Warn, "reqwest")));
    }
}
