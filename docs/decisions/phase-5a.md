---
title: Phase 5a decisions
---

# Phase 5a decisions (account identity convergence, redacted logging)

Taken on 2026-11-16 as follow-ups of phase 4.

## 1. Account identity shared by sync and import

- `AccountIdentity` moved from `bank::connect` to the neutral `ledger_core::identity` (re-exported from `bank::connect`). Unchanged rule: SHA-256 hex of the IBAN without spaces, upper case → `accounts.identity_hash`; last 4 → `display_suffix`. The IBAN is never stored.
- **Where import finds the IBAN:** in the preamble, i.e. the rows before the header row (or before `skip_rows` when there is no header). A cell counts if, ignoring whitespace (NBSP included), it is IBAN-shaped: 2 letters, 2 digits, 11–30 alphanumerics. Check digits are **not** verified, so fictitious test IBANs work and the hash matches whatever the bank sends. This is profile-agnostic: no label matching, so it works for Openbank's "Número de cuenta" row and any other bank that prints an IBAN above the table.
  - More than one distinct IBAN in the preamble → no identity (ambiguous; never guess).
  - IBANs inside data rows (e.g. a transfer concept) are ignored.
  - UNVERIFIED: that Openbank's real export prints the account number as a full IBAN (the owner described "account number"; the fixtures use `ES00 …`). If it is a 20-digit CCC, no identity is derived and import behaves as in phase 3.
- **`import_file` reconciliation** (before anything is written):

  | Target account `identity_hash` | Another account has the file's identity | Result |
  | --- | --- | --- |
  | equal to the file's | — | import |
  | different | — | `ImportError::AccountMismatch` |
  | NULL | no | target adopts it (`identity_hash`, `display_suffix` if empty), import |
  | NULL | yes | `ImportError::AccountBelongsToOther { existing_account_id }` |
  | target id does not exist | — | `ImportError::AccountNotFound` |

- **Refuse, not merge.** Merging two account rows would mean moving transactions with re-dedup across both histories, re-pointing transfer pairs, import batches, sync links and balances. That is not trivially safe, so the import is refused with a typed error carrying the existing account id; the UI can offer "import into that account instead". A target whose identity came from the Enable Banking identification-hash fallback (no IBAN) is also refused as a mismatch: wrong-account imports are worse than a false refusal.
- The identity update and the ingest are separate transactions (ingest opens its own). If ingest fails after the identity was adopted, the identity stays: it is a fact read from the file, not a partial import.
- Error messages never contain the IBAN or its suffix.

## 2. Redacted logging

- **Logger:** `tauri-plugin-log` 2.10, registered at the start of `setup` so later setup errors are logged. Rust side only: the plugin's `log` IPC command is not granted in the capability, so the WebView cannot write to the log.
- **Location:** `TargetKind::Folder` at the app log dir (`%LOCALAPPDATA%\dev.pocketledger.app\logs`), or `<data dir>\logs` when the debug-only `POCKET_LEDGER_DATA_DIR` override is set, so development runs keep logs inside `tmp/`. File `pocket-ledger.log`, 1 MB, `KeepSome(5)`, UTC timestamps. Stdout as well in debug builds.
- **Level:** `info`. Debug builds read `POCKET_LEDGER_LOG`; release builds ignore it (same policy as the data dir override). Only `pocket_ledger*` and `ledger_core` may log below `info`; third-party crates (reqwest, hyper, rustls, tao, wry) are capped at `info`, so HTTP tracing with URLs or headers can never reach the file.
- **Redaction (`ledger_core::redact`)**, applied to every formatted line in the plugin formatter: `Bearer <x>` → `Bearer [redacted]`; JWT (`eyJ….….…`) → `[jwt]`; UUID → `[uuid]` (Enable Banking session ids and account uids); IBAN-shaped (compact or in groups of 4) → `[iban]`; 13+ digit runs → `[number]`. Over-redaction is accepted (a word of ≤ 3 characters right after a spaced IBAN may be swallowed).
- The filter is a safety net, not the rule. ADR 0003 still governs call sites: endpoint templates, status, durations, opaque codes. Amounts, names and free text cannot be detected reliably by a filter and must never be passed to `log` calls; existing calls (`enable_banking::client`, `bank::connect`, `scheduler`) were reviewed and comply.

## Build note

The worktree path is deep enough that building the vendored OpenSSL hits the Windows 260-character path limit (`C1083`). Use a short `CARGO_TARGET_DIR` (e.g. the project's ignored `tmp\t5a`) when building from a worktree.

## Not done

- No UI for the new import errors (UI files were out of scope for this phase; there is no import screen yet).
- The app was not run end to end; the logger is covered by unit tests of the formatter, filter and redaction. A live run was skipped on purpose, see next point.
- **Follow-up found (not fixed here):** the OS keyring service is always `pocket-ledger`, whatever `POCKET_LEDGER_DATA_DIR` says. A debug run with another data dir (including `pocket-ledger-dev.ps1 -Sandbox`) therefore reads the production database key and the production `enable-banking-session` entry; connecting in sandbox would overwrite the production session. The debug data dir override should also namespace the keyring entries (e.g. service `pocket-ledger-dev`).
