---
title: "ADR 0003: Secrets storage"
---

# ADR 0003: Secrets storage

- **Status:** Proposed (depends on ADR 0001)
- **Date:** 2026-11-15

## Context

The app handles: the Enable Banking private key (`.pem`), the Enable Banking application ID, bank session identifiers, and the database key. The repository is public.

## Decision (proposed)

| Item | Where | Notes |
|---|---|---|
| Private key `.pem` | Stays where the owner put it, outside the repo; path in env var `ENABLE_BANKING_KEY_PATH` | Read by the Rust core only at JWT-signing time, kept in memory only as long as needed. Never logged, copied, or sent to the UI. |
| Application ID | Local config file in app data dir | Not a secret per se, but kept out of the repo. |
| Enable Banking `session_id` + consent `valid_until` | OS keyring | Session ID grants data access when combined with a valid JWT. |
| Database key | OS keyring | See ADR 0002. |

- **OS keyring:** `keyring` crate → Windows Credential Manager (macOS Keychain / Secret Service later if needed). Service name `pocket-ledger`.
- **Logging:** structured logs with a redaction layer; request/response bodies, tokens, IBANs, names and amounts are never logged. Only endpoint, status code, duration and opaque error codes.
- **UI boundary:** no Tauri command returns secrets; the UI only learns states such as "connected, consent expires in N days".

## Threat model (short)

In scope: accidental leaks (repo, logs, screenshots, crash dumps), stolen disk/backup, malicious web content in the WebView. Out of scope: malware running as the same Windows user (it could read Credential Manager and the `.pem` anyway).

## Consequences

- If the `.pem` is moved, the app reports a clear configuration error instead of failing silently.
- Rotating the Enable Banking key only requires updating the file and env var.
