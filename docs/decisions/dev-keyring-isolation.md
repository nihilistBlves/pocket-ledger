---
title: Development keyring isolation
---

# Development keyring isolation

Decided on 2026-11-16, found during phase 5a.

## Problem

Debug builds accept `POCKET_LEDGER_DATA_DIR` to keep the database and config of a development run inside the project's ignored `tmp/` (the sandbox script `pocket-ledger-dev.ps1 -Sandbox` uses `tmp/sandbox-e2e`). The OS keyring service, however, was always `pocket-ledger`. A sandbox run therefore read the production database key and the production `enable-banking-session` entry, and connecting to the sandbox bank would have overwritten the real Openbank session.

## Decision

- `OsKeyring` takes its service name (`OsKeyring::new(service)`); `OsKeyring::default()` keeps the production name `pocket-ledger`.
- `keystore::service_for(data_dir_overridden)` picks the service: `pocket-ledger-dev` when the data dir override is active, `pocket-ledger` otherwise. The Tauri setup passes it to `AppState::init`.
- The override only exists in debug builds (`data_dir_override` returns `None` in release), so release builds always use `pocket-ledger`. Nothing changes for the installed app or its existing entries.
- A plain debug run without the override (`pnpm tauri dev` with no variable) still uses the production app data dir and therefore the production service: the two always move together.

## Alternatives considered

- **Service derived from the data dir** (e.g. a hash of the path): keeps every dev data dir fully separate, but leaves unreadable, orphaned entries in Credential Manager for each throwaway dir. All development data dirs share `pocket-ledger-dev` instead: they reuse one database key (each new dev database is simply created with it) and one sandbox bank session, which is acceptable for test data.

## Notes

- Dev entries can be removed from Windows Credential Manager (generic credentials whose name starts with `pocket-ledger-dev`) without affecting the production app.
- Tests never write to the real keyring: they use `MemoryStore` and check the service selection; the ignored `os_keyring_roundtrip` test uses the dev service.
