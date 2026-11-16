# pocket-ledger

Local-first, single-user personal finance desktop app. Read-only bank access via Enable Banking (PSD2). All data stays on the device.

> **Public repository.** Never commit secrets, keys, tokens, IBANs or real banking data. Use fictitious data in tests and fixtures.

- [Privacy Policy](docs/PRIVACY.md) — published at https://nihilistblves.github.io/pocket-ledger/PRIVACY.html
- [Terms of Use](docs/TERMS.md) — published at https://nihilistblves.github.io/pocket-ledger/TERMS.html
- OAuth redirect bridge: [`docs/callback.html`](docs/callback.html) (GitHub Pages) forwards the bank redirect, query string included, to the app's temporary loopback listener at `http://127.0.0.1:8080/callback`.

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

- Normal runs: `%LOCALAPPDATA%\dev.pocketledger.app\logs\pocket-ledger.log`
- Debug runs with `POCKET_LEDGER_DATA_DIR` set: `<that dir>\logs\pocket-ledger.log`

Level `info`; debug builds also print to stdout and accept `POCKET_LEDGER_LOG=debug` (or `trace`, `warn`, `error`). Log calls only carry endpoint templates, status codes, durations and error codes, and every line passes through a redaction filter that masks IBANs, JWT/bearer tokens, UUIDs (session ids) and long digit runs (see `docs/decisions/phase-5a.md`).
