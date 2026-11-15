---
title: Phase 4 decisions
---

# Phase 4 decisions (Enable Banking connection)

Taken with the owner on 2026-11-15/16.

## What was built

- `ledger-core::bank::enable_banking`: RS256 JWT client (key read only at signing time from `ENABLE_BANKING_KEY_PATH`, token cached ≤ 10 min, redirects disabled), endpoints for ASPSPs, auth, sessions, account details/balances/transactions with `continuation_key` pagination, retries with backoff (POST `/auth` and `/sessions` only on 429/connect), typed errors, redacted `Debug`.
- `bank::callback` + `bank::connect`: loopback listener on `127.0.0.1:8080` per ADR 0004 (single use, 128-bit `state` compared in constant time, 5-minute timeout); session id + consent expiry only in the OS keyring (`enable-banking-session`); accounts linked by SHA-256 of the normalized IBAN (IBAN and holder name never stored).
- `bank::sync`: one `sync_runs` row per execution; per-account window `last_synced_to − 5 days` (90 days the first time) → ingest (dedup, pending→booked, transfers, rules) → stale-pending reconciliation → latest balance. Policy: automatic syncs ≥ 6 h apart and ≤ 4 per 24 h, manual syncs with a 60 s guard.
- Tauri commands `bank_overview`, `bank_connect` (also renewal), `bank_sync_now`, `bank_disconnect`; background scheduler every 30 min; "Cuentas y conexión" screen.

## Configuration

- `enable-banking.json` in the app data dir: `application_id`, `aspsp_name`, `aspsp_country` (not secret). Private key path from `ENABLE_BANKING_KEY_PATH`.
- Debug builds only: `POCKET_LEDGER_DATA_DIR` overrides the data dir so development runs stay inside the project's ignored `tmp/`.
- Sandbox: ASPSP name `Mock ASPSP` (ES), `maximum_consent_validity` 15552000 s (180 days). Production target: Openbank (ES), only after the owner's go-ahead.

## Sandbox checkpoint (2026-11-16)

Verified end to end with the sandbox application and the Mock ASPSP, the owner authorizing in the system browser:

- JWT accepted by the API (`GET /aspsps`).
- `POST /auth` accepted the GitHub Pages bridge as `redirect_url`; the bridge forwarded to the loopback listener; `state` validated; `POST /sessions` succeeded.
- Accounts linked, manual sync stored balances and transactions, run recorded as `ok`.
- The app log contains no tokens, session ids or IBANs.

## Still unverified

- **Is the authorization `code` bound to the application JWT?** (ADR 0004.) Testing it needs a second sandbox application; the owner has only one. The `code` keeps being treated as a secret (state check, never logged, single-use listener).
- Redirect URL matching rules beyond "the registered URL works" (exact match vs prefix).
- How the API/bank distinguishes user-present syncs (PSU headers are optional and not sent).
- Bank-synced and imported accounts converge only if the import path uses `AccountIdentity::from_iban`; phase 3 import does not derive an identity yet.
