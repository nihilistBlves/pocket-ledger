---
title: Phase 5b decisions
---

# Phase 5b decisions (encrypted backup, restore and recovery)

Taken with the owner on 2026-11-16. Design in [ADR 0005](../adr/0005-backups.md).

## Decisions confirmed with the owner

- **Contents:** database (still SQLCipher-encrypted) + its key + `enable-banking.json`. Never the `.pem` or the bank session; restoring on another computer needs a new bank consent.
- **When/where:** manual only, native Save dialog defaulting to `Documents\Pocket Ledger\Copias`, reminder in Ajustes after 7 days without a verified backup. The app never deletes backups.
- **Recovery mode:** yes, in this phase.
- **Passphrase:** at least 12 characters and zxcvbn score of 3 or more, never stored.

## What was built

- `ledger-core::backup`: `.plbackup` v1 format (72-byte authenticated header, Argon2id m=256 MiB t=3 p=4, XChaCha20-Poly1305), create with read-back verification, restore in two steps (`prepare_restore` off to the side, then `swap_in` with rollback), `startup_check`, `unlock_with_backup`, `recovery_restore`, `cleanup_stale`, passphrase gate. Crates: `argon2` 0.6, `chacha20poly1305` 0.11, `zxcvbn` 3.1 (RustCrypto, default features off plus `alloc`/`zeroize`).
- `db::open_unmigrated` and `db::latest_schema_version`.
- Tauri: `backup_commands.rs` (`app_mode`, `backup_status`, `backup_check_passphrase`, `backup_create`, `backup_pick_file`, `backup_restore`, `recovery_unlock`, `recovery_restore`). Native dialogs run from Rust through `tauri-plugin-dialog`. The WebView capability is still `core:default` only, and the UI never sends file paths: Rust keeps the picked path and returns only its file name. `AppState` gets `mode`, `db_key` and `picked_backup`. `try_busy()` and the bank `lock()` refuse in recovery mode, so the scheduler does too.
- UI: Ajustes → Copias de seguridad (status, reminder, create with strength meter and mandatory warning, restore with inline confirmation), and a recovery screen that replaces the whole shell when `app_mode` is `recovery`. Screens refresh on the `data://replaced` event.

## Review notes (orchestrator)

- Crypto reviewed by hand. The header is checked (magic, version, ids, parameter bounds, exact length) before Argon2 runs. The AAD is the whole header. Salt and nonce are fresh per file. A wrong passphrase and a tampered file give one opaque error. Plaintext and keys live only in `Zeroizing` buffers. The SQLCipher key goes into `ATTACH … KEY ?` as a bound parameter, never into SQL text.
- Changed after review: the recovery restore first builds and checks the replacement under a fresh in-memory key, and only then touches the keyring and renames the unreadable file. A failure before the swap leaves both untouched (test `failed_recovery_restore_leaves_store_and_file_untouched`).

## Tests

Round trip into another installation with a different key; wrong passphrase; tampering (ciphertext, header, truncation); unsupported version and unsafe parameters rejected before the KDF; newer schema refused; forced failure after the swap rolled back; no key, passphrase or row text in any file left on disk, or in `Debug`/error output; NFC passphrases; startup and recovery cases; one end-to-end run with the production parameters; command-level tests with `MemoryStore` and temp dirs; Angular specs for the backups and recovery screens. Fictitious data only, and the real data dir and Credential Manager were never touched.

Results on 2026-11-16: `cargo test --workspace` (ledger-core 221, app 35), `cargo clippy --workspace --all-targets -D warnings`, `pnpm test` (29) and `pnpm build` all pass.

## Known limitations and still unverified

- **Not run in the real app yet.** The native Save/Open dialogs have not been checked by hand (that needs the owner's app and keyring), and neither has Argon2 staying off the UI thread (it runs in `spawn_blocking`). Suggested manual check: create a backup, restore it, and check the reminder.
- If a restore fails **and** its rollback also fails (`restore_failed`), the app keeps a placeholder connection until restart. The original stays on disk as `pocket-ledger.db.pre-restore-<UTC>`, and the next startup handles the rest. Mode is not switched, so bank commands fail with an internal error until then.
- The passphrase exists briefly as a plain `String` while Tauri deserializes it, before it moves into `Zeroizing`. SQLCipher's page cache is not wiped.
- One-shot AEAD keeps the whole backup in memory. That is fine for a few MB; past ~1 GiB the format needs a version 2 (chunked STREAM).
- Bank commands added by other phases must use `try_busy()` or the bank `lock()` helper (or call `require_ready()`) so they honour recovery mode.
