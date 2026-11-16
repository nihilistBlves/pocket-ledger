---
title: Phase 9 decisions
---

# Phase 9 decisions (packaging and install)

Taken with the owner on 2026-11-16. Goal: run Pocket Ledger installed, without the dev setup (`pocket-ledger-dev.ps1`, `pnpm tauri dev`).

## Owner decisions

- **Installer: NSIS, per user** (`bundle.targets: ["nsis"]`, `installMode: "currentUser"`). Installs to `%LOCALAPPDATA%\Pocket Ledger` without administrator rights; Spanish/English, no language selector. MSI is not built.
- **Unsigned.** SmartScreen warns once per new installer ("More info → Run anyway"). A self-signed certificate would not remove the warning unless trusted as a root on this PC, so it adds work without benefit for a personal app.
- **Identifier unchanged: `dev.pocketledger.app`.** The installed app opens the existing real data in `%APPDATA%\dev.pocketledger.app` and the existing keyring entries (service `pocket-ledger`).
- **Bank key location: a path in Ajustes.** New optional field `key_path` in `enable-banking.json`, set from *Ajustes → Conexión bancaria* through a native file dialog driven from Rust (`.pem` only, absolute, must exist; the file is not opened). Resolution order: `ENABLE_BANKING_KEY_PATH` when set and non-empty (dev script) → `key_path` → `key_missing`. The UI only receives the file name, the source (`env` / `settings` / `none`) and whether the file exists; never the full path or the content. The path is not logged. The file is updated atomically (temp + rename) and keeps unknown fields; it is never created by this command (a missing or invalid `enable-banking.json` stays `not_configured`).
- **Updates: manual reinstall.** No Tauri updater (it would need an updater signing key to guard and a published `latest.json`).

## Release build

- `[profile.release]` in `src-tauri/Cargo.toml` (workspace root): `opt-level = 3`, `lto = true`, `codegen-units = 1`, `strip = true`; panics keep unwinding.
- CSP unchanged. The `tauri` crate does not enable the `devtools` feature, so release builds have no inspector.
- Release ignores `POCKET_LEDGER_DATA_DIR` and `POCKET_LEDGER_LOG` (both gated by `cfg!(debug_assertions)`); log level `info`.
- **Angular critical CSS inlining is off** (`optimization.styles.inlineCritical: false` in the production configuration). Beasties emits `<link media="print">` and switches it to `all` with an inline script, which the strict CSP (`script-src 'self'`) blocks: the first installed build rendered with only the critical CSS (unstyled navigation, black chart). Dev mode never showed it because the dev build does not optimize.
- `scripts/build-release.ps1`: `pnpm install --frozen-lockfile`, `pnpm tauri build`, prints installer path, size and SHA-256. Uses `CARGO_TARGET_DIR` if set, else `tmp\target-release`. Works under `C:\Dev\claude\build-quiet.ps1 "powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-release.ps1"`.
- Build note: from a worktree the path is too deep for the vendored OpenSSL build (MAX_PATH); use a short `CARGO_TARGET_DIR`. A clean release build took ~13.5 minutes and several GB of disk.

## Uninstall and data

The Tauri NSIS uninstaller offers a "Delete the application data" checkbox, unticked by default. Unticked, it removes only the program files, Start menu entry and uninstall registry key. Ticked, it deletes `%APPDATA%\dev.pocketledger.app` (database, config) and `%LOCALAPPDATA%\dev.pocketledger.app` (logs). Keyring entries are never removed. The README tells the owner to leave it unticked.

## Verification (this machine, 2026-11-16)

- Installer built twice with `scripts/build-release.ps1` (clean ~13.5 min, incremental after a frontend change ~3.5 min), 6.8 MB.
- Silent install (`/S`) → `%LOCALAPPDATA%\Pocket Ledger`, uninstall key under HKCU, no elevation.
- The installed app opened the existing real database without recovery mode: Resumen and Cuentas (Openbank connected, 3 accounts) rendered from local data; Ajustes → Conexión bancaria showed "No configurada". Nothing was synced: with no `key_path` and no environment variable, the automatic sync fails with `key_missing` before any network call or database write (logged once as a warning).
- Silent uninstall (`uninstall.exe /S`) removed the program files and left the data folder byte-identical (names, sizes, timestamps) and the logs in place; reinstalling picked the same data up again.
- Not verified: choosing the `.pem` from the installed app and a real sync with it (needs the owner's go-ahead).

## Not done

- No code signing, no auto-updater (owner decisions).
- `src/app/core/strings.ts` still has a "not configured" bank hint that mentions `ENABLE_BANKING_KEY_PATH` (asserted by `accounts.spec.ts`); left for after the parallel import-screen work, to avoid conflicts in `src/app`.
- `cargo clippy` was not run in this phase (disk full during the attempt).
