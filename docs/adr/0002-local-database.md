---
title: "ADR 0002: Local encrypted database"
---

# ADR 0002: Local encrypted database

- **Status:** Proposed (depends on ADR 0001)
- **Date:** 2026-11-15

## Context

All financial data must live only on the owner's device, preferably encrypted at rest. Money must never be stored as floating point.

## Decision (proposed)

- **SQLite with SQLCipher** (AES-256 page encryption) via `rusqlite` with the `bundled-sqlcipher-vendored-openssl` feature, so no system library is required.
- **Database key:** 256-bit random key generated on first run and stored in the OS keyring (ADR 0003). Never written to disk, logs or config.
- **Location:** the per-user app data directory (`%APPDATA%\pocket-ledger\` on Windows), never inside the repo.
- **Migrations:** embedded, versioned SQL files applied at startup (`rusqlite_migration`), forward-only, covered by tests.
- **Money:** `INTEGER` amounts in minor units (cents) with a `currency` column (`EUR`). Signed: negative = outflow. Conversion from API strings (`"12.34"`) is done with exact decimal parsing, never `f64`.
- **Dates:** ISO-8601 `TEXT` (`YYYY-MM-DD` for booking/value dates; UTC timestamps for audit fields).
- **WAL mode**, foreign keys on, single writer (the Rust core).

## Consequences

- Losing the keyring entry means losing the database: the encrypted backup feature (phase 5) must export the key protected by a user passphrase.
- Tests use in-memory or temp-dir databases with fictitious data and a fixed test key.
