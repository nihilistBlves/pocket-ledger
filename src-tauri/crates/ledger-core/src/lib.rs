//! Domain core of Pocket Ledger.
//!
//! Pure Rust, no Tauri, no network. Modules:
//! - [`bank`]: open-banking client (Enable Banking).
//! - [`money`]: exact parsing/formatting of amounts as signed integer cents.
//! - [`db`]: encrypted SQLite (SQLCipher) access, schema and migrations.
//! - [`identity`]: stable account identity (hashed IBAN) shared by sync and import.
//! - [`backup`]: encrypted backup/restore and startup recovery (ADR 0005).
//! - [`keystore`]: where secrets (the database key) are kept.
//! - [`model`]: source-independent transaction shape for the ingest pipeline.
//! - [`import`]: CSV/Excel file import driven by saved profiles.
//! - [`redact`]: masks IBANs, tokens and ids in log lines (ADR 0003 safety net).
//! - [`normalize`], [`dedup`], [`transfers`], [`rules`], [`ingest`]: the ingest pipeline.
//!
//! Money is never a floating point number anywhere in this crate.

#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic)]

pub mod backup;
pub mod bank;
pub mod db;
pub mod dedup;
pub mod identity;
pub mod import;
pub mod ingest;
pub mod keystore;
pub mod ledger;
pub mod model;
pub mod money;
pub mod normalize;
pub mod redact;
pub mod rules;
pub mod transfers;

#[cfg(test)]
mod test_support;
