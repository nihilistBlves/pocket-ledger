# pocket-ledger

Local-first, single-user personal finance desktop app. Read-only bank access via Enable Banking (PSD2). All data stays on the device.

> **Public repository.** Never commit secrets, keys, tokens, IBANs or real banking data. Use fictitious data in tests and fixtures.

- [Privacy Policy](docs/PRIVACY.md) — published at https://nihilistblves.github.io/pocket-ledger/PRIVACY.html
- [Terms of Use](docs/TERMS.md) — published at https://nihilistblves.github.io/pocket-ledger/TERMS.html
- OAuth redirect bridge: [`docs/callback.html`](docs/callback.html) (GitHub Pages) forwards the bank redirect, query string included, to the app's temporary loopback listener at `http://127.0.0.1:8080/callback`.

Status: early setup (phase 0).
