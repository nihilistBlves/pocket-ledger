//! File-system helpers shared by backup and restore.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;

use super::error::BackupError;

/// Sidecar suffixes SQLite may create next to a database file.
pub(crate) const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// UTC timestamp for file and entry names: `yyyymmddThhmmssZ`.
pub(crate) fn utc_stamp() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// RFC 3339 UTC timestamp with second precision.
pub(crate) fn utc_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// `path` with `suffix` appended to its file name (not replacing the extension).
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// First free path of the form `<path><label>-<stamp>[-n]`.
pub(crate) fn aside_path(path: &Path, label: &str, stamp: &str) -> PathBuf {
    let base = format!("{label}-{stamp}");
    let first = with_suffix(path, &base);
    if !first.exists() {
        return first;
    }
    (1u32..)
        .map(|n| with_suffix(path, &format!("{base}-{n}")))
        .find(|p| !p.exists())
        .expect("unbounded range")
}

/// Random lowercase hex id for unique temp file names.
pub(crate) fn random_id() -> Result<String, BackupError> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| BackupError::Random)?;
    Ok(hex::encode(bytes))
}

/// A temp file in the data directory, removed (with SQLite sidecars) on drop.
pub(crate) struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub(crate) fn new(dir: &Path, prefix: &str) -> Result<Self, BackupError> {
        let path = dir.join(format!("{prefix}{}.tmp", random_id()?));
        Ok(TempFile { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        remove_with_sidecars(&self.path);
    }
}

/// Best-effort removal of a database file and its SQLite sidecars.
pub(crate) fn remove_with_sidecars(path: &Path) {
    let _ = fs::remove_file(path);
    for s in SIDECARS {
        let _ = fs::remove_file(with_suffix(path, s));
    }
}

/// Writes `bytes` to `<target>.partial`, fsyncs, then renames over `target`.
pub(crate) fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), BackupError> {
    let partial = with_suffix(target, ".partial");
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&partial)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&partial);
        return Err(e.into());
    }
    Ok(())
}
