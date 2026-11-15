---
title: "ADR 0004: Authorization callback"
---

# ADR 0004: Authorization callback via HTTPS bridge + temporary loopback listener

- **Status:** Accepted (2026-11-15)
- **Date:** 2026-11-15

## Context

After the owner authorizes access at the bank, Enable Banking redirects the browser to a whitelisted `redirect_url` with `code` and `state`. A desktop app has no public HTTPS endpoint. Running local HTTPS needs a self-signed certificate trusted by the browser, which is fragile.

## Decision

1. **Registered redirect URL:** `https://nihilistblves.github.io/pocket-ledger/callback.html`, a static page on GitHub Pages ([docs/callback.html](../callback.html)) that does `location.replace("http://127.0.0.1:8080/callback" + location.search)`. It stores and sends nothing; `no-referrer` and `noindex` are set.
2. **`redirect_url` sent in `POST /auth`** is that exact string, byte for byte.
3. **Loopback listener** in the Rust core:
   - Binds `127.0.0.1:8080` only (never `0.0.0.0`, never `localhost`, which can resolve to `::1`).
   - Started immediately before opening the bank URL in the system browser; stopped after the first valid callback, an error, or a **5-minute timeout**.
   - Accepts only `GET /callback`; anything else → 404.
   - Validates `state` (128-bit random, single use, constant-time compare). Mismatch → reject and abort the flow.
   - Exchanges `code` via `POST /sessions` immediately; `code` and `session_id` are never logged.
   - Responds with a minimal static "You can close this tab" page; no external resources.
4. **Port conflict:** if 8080 is busy, fail with a clear message (the port is fixed because the bridge page hard-codes it).
5. **Browser:** the system default browser via the Tauri opener plugin; the app never renders bank pages in its own WebView and never handles bank credentials.

## Consequences

- The authorization `code` transits a public static page: the browser's request to `github.io` includes the query string, so it may appear in GitHub's access logs. `no-referrer` prevents onward leaks; the page itself loads nothing else. Risk accepted because `code` is short-lived, single-use, and exchanging it requires our signed JWT (**to be verified** in the sandbox: attempt `POST /sessions` with a code using a JWT from a different application should fail).
- If Enable Banking rejects the `github.io` URL, fallback: register `https://localhost:8080/callback` and serve local HTTPS with a per-install self-signed certificate.
