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
