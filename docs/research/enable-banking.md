---
title: Enable Banking API notes
---

# Enable Banking API notes

Researched 2026-11-15 from the official docs. Items marked **UNVERIFIED** must be checked against the OpenAPI spec (<https://enablebanking.com/docs/api/reference/enablebanking-api.yaml>) or the sandbox before relying on them.

## Authentication

- Every request: `Authorization: Bearer <JWT>`.
- JWT header: `typ: JWT`, `alg: RS256`, `kid: <application id>`.
- Claims: `iss: "enablebanking.com"`, `aud: "api.enablebanking.com"`, `iat`, `exp`. Max TTL 86400 s; we use short-lived tokens (≤ 1 h), minted per sync.
- Source: [quick start](https://enablebanking.com/docs/api/quick-start/), [FAQ](https://enablebanking.com/docs/faq/).

## Flow

1. `GET /aspsps?country=ES&psu_type=personal` → find Openbank; read `maximum_consent_validity`.
2. `POST /auth` `{ access: { valid_until }, aspsp: { name, country }, state, redirect_url, psu_type }` → `{ url, authorization_id }`. `valid_until` ≤ now + `maximum_consent_validity`.
3. Owner authenticates at the bank in the system browser → redirect to `redirect_url?code=…&state=…`.
4. `POST /sessions { code }` → `{ session_id, accounts[], access.valid_until }`.
5. `GET /accounts/{uid}/details | /balances | /transactions?date_from&date_to&continuation_key&transaction_status`. Paginate until `continuation_key` is absent.
6. `DELETE /sessions/{session_id}` to revoke.

Account `uid` is only valid while the session is authorized: store our own stable account identity (derived from account details) and map `uid` per session.

## Transaction fields relevant to the model

`entry_reference`, `transaction_id` (**not unique, may change**), `booking_date`, `value_date`, `credit_debit_indicator` (`CRDT`/`DBIT`), `transaction_amount { amount: string, currency }`, `creditor`, `debtor`, `remittance_information[]`, `status` (`BOOK`, `PDNG`, `HOLD`, `CNCL`, `RJCT`, `SCHD`, `OTHR`).

Implications:
- Amount arrives as an unsigned decimal string; sign comes from `credit_debit_indicator`. Parse exactly to cents.
- Deduplication cannot rely on `transaction_id`. Use `entry_reference` when present, else a fingerprint (account, booking/value date, signed amount, normalized counterparty + remittance) with an occurrence index for identical same-day rows. Pending → booked transitions must update, not duplicate.

## Limits and errors

- HTTP 429 (`ASPSP_RATE_LIMIT_EXCEEDED`) on any endpoint → exponential backoff, respect `Retry-After` if present.
- PSD2: many banks allow **4 unattended fetches/day**; docs recommend ≥ 6 h between automatic fetches. User-initiated syncs while the user is present are separate (**UNVERIFIED** how the API distinguishes; likely `Psu-*` headers).
- Session errors: `EXPIRED_SESSION`, `REVOKED_SESSION`, `CLOSED_SESSION` → prompt re-authorization. Also `ASPSP_TIMEOUT`, `REDIRECT_URI_NOT_ALLOWED`.

## Consent

- Max validity is bank-specific, usually **180 days**. Store `access.valid_until`; show days remaining; offer renewal before expiry.

## Sandbox

- Separate sandbox application required. **Mock ASPSP** available for all countries, no credentials; transactions in batches of 10. Exact name: check `GET /aspsps` in sandbox.
- Openbank availability in sandbox: **UNVERIFIED** (not listed). Spanish sandbox banks with public test logins exist (e.g. BBVA, Sabadell).

## Restricted production ("linked accounts")

- Activate the production app by linking own accounts in the Control Panel. Data limited to linked accounts; the normal auth flow is still required. Personal/evaluation use only.

## Open questions (UNVERIFIED)

1. Is `code` bound to the application (i.e. useless without our signed JWT)? Docs only say `POST /sessions` is authenticated. **We treat `code` as a secret** regardless: `state` check, no logging, single-use listener.
2. Redirect URL rules (HTTPS required? exact match?). Only "whitelisted" is documented. We register the GitHub Pages bridge URL and send the identical string.

## Official samples

JS, C#, Go, PHP, Python, Ruby in <https://github.com/enablebanking/enablebanking-api-samples>. No official Rust or TypeScript SDK; we implement a small client.
