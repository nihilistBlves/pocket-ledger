# pocket-ledger

Local-first, single-user personal finance desktop app. Read-only bank access via Enable Banking (PSD2). All data stays on the device.

> **Public repository.** Never commit secrets, keys, tokens, IBANs or real banking data. Use fictitious data in tests and fixtures.

- [Privacy Policy](docs/PRIVACY.md) — published at https://nihilistblves.github.io/pocket-ledger/PRIVACY.html
- [Terms of Use](docs/TERMS.md) — published at https://nihilistblves.github.io/pocket-ledger/TERMS.html
- OAuth redirect bridge: [`docs/callback.html`](docs/callback.html) (GitHub Pages) forwards the bank redirect, query string included, to the app's temporary loopback listener at `http://127.0.0.1:8080/callback`.

## Install (Windows)

The app ships as an unsigned, per-user NSIS installer (no administrator rights needed). See [docs/decisions/phase-9.md](docs/decisions/phase-9.md).

1. Build it (see [Release build](#release-build)) or take an installer you built earlier: `Pocket Ledger_<version>_x64-setup.exe`.
2. Run it. Because it is unsigned, SmartScreen shows "Windows protected your PC" the first time: *More info → Run anyway*.
3. The app installs to `%LOCALAPPDATA%\Pocket Ledger` and adds a Start menu entry "Pocket Ledger".
4. Bank connection: in **Ajustes → Conexión bancaria** choose the Enable Banking private key (`.pem`). Only its path is stored (field `key_path` in `enable-banking.json`); the key is never copied. The environment variable `ENABLE_BANKING_KEY_PATH`, when set, takes priority (dev script).

**Updating:** run the newer installer over the installed one. Data and settings are kept. There is no auto-updater.

**Uninstalling:** *Settings → Apps → Pocket Ledger → Uninstall*. Leave the uninstaller's "Delete the application data" box **unticked**: if ticked, it deletes `%APPDATA%\dev.pocketledger.app` (the database) and `%LOCALAPPDATA%\dev.pocketledger.app` (logs). The OS keyring entries (service `pocket-ledger`, which hold the database key) are never removed by the uninstaller.

### Where data lives

| What | Where |
| --- | --- |
| Encrypted database (`pocket-ledger.db`, `-wal`, `-shm`) and `enable-banking.json` | `%APPDATA%\dev.pocketledger.app` |
| Logs (`pocket-ledger.log`, rotated) | `%LOCALAPPDATA%\dev.pocketledger.app\logs` |
| Database key and bank session | Windows Credential Manager, generic credentials of service `pocket-ledger` |
| Program files | `%LOCALAPPDATA%\Pocket Ledger` |

### Backups

Use **Ajustes → Copias de seguridad**: it writes an encrypted, passphrase-protected backup file wherever you choose (see [ADR 0005](docs/adr/0005-backups.md)). Copying the data folder alone is **not** a backup: the database is encrypted with a key kept in Credential Manager, so the files are useless on another machine or after the keyring entry is lost.

## Release build

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\build-release.ps1
```

It installs JS dependencies from the lockfile, runs `pnpm tauri build` (release profile: LTO, one codegen unit, stripped symbols) and prints the installer path, size and SHA-256. The installer lands in `<CARGO_TARGET_DIR>\release\bundle\nsis\`; by default `CARGO_TARGET_DIR` is `tmp\target-release`. From a deep checkout path (e.g. a git worktree) set `CARGO_TARGET_DIR` to a short folder first: the vendored OpenSSL build fails beyond the Windows 260-character path limit. A clean build takes about 15 minutes and needs several GB of free disk. Installers and build outputs are never committed.

## Development

Prerequisites: Node 24, pnpm, Rust (stable) with the MSVC build tools, and WebView2 (preinstalled on Windows 11).

```sh
pnpm install                  # install JS dependencies
pnpm tauri dev                # run the desktop app with live reload
pnpm build                    # build the Angular frontend (dist/pocket-ledger/browser)
pnpm ng test --watch=false    # frontend unit tests
cargo test --manifest-path src-tauri/Cargo.toml --workspace   # Rust tests
pnpm tauri build --no-bundle  # release build of the desktop binary
```

Debug builds only: set `POCKET_LEDGER_DATA_DIR` to keep a development run's database and config in another folder (e.g. the project's ignored `tmp/`). While it is set, secrets go to the OS keyring service `pocket-ledger-dev` instead of `pocket-ledger`, so the run never reads the production database key or touches the production bank session. Release builds ignore the variable. See [docs/decisions/dev-keyring-isolation.md](docs/decisions/dev-keyring-isolation.md).

### Logs

The app writes a size-rotated log file (1 MB per file, 5 old files kept):

- Installed app and normal runs: `%LOCALAPPDATA%\dev.pocketledger.app\logs\pocket-ledger.log`
- Debug runs with `POCKET_LEDGER_DATA_DIR` set: `<that dir>\logs\pocket-ledger.log`

Level `info`; debug builds also print to stdout and accept `POCKET_LEDGER_LOG=debug` (or `trace`, `warn`, `error`). Log calls only carry endpoint templates, status codes, durations and error codes, and every line passes through a redaction filter that masks IBANs, JWT/bearer tokens, UUIDs (session ids) and long digit runs (see `docs/decisions/phase-5a.md`).
