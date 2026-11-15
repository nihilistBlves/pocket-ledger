//! Domain core of Pocket Ledger.
//!
//! Pure Rust, no Tauri, no network. Modules:
//! - [`money`]: exact parsing/formatting of amounts as signed integer cents.
//! - [`db`]: encrypted SQLite (SQLCipher) access, schema and migrations.
//! - [`keystore`]: where secrets (the database key) are kept.
//! - [`model`]: source-independent transaction shape for the ingest pipeline.
//!
//! Money is never a floating point number anywhere in this crate.

#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic)]

pub mod db;
pub mod keystore;
pub mod model;
pub mod money;
