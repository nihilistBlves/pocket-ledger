---
title: "ADR 0001: Desktop stack"
---

# ADR 0001: Desktop stack

- **Status:** Accepted (2026-11-15)
- **Date:** 2026-11-15

## Context

pocket-ledger is a single-user, local-first desktop app that holds read-only bank data, a signing key reference and bank session tokens. The owner is a full-stack developer (.NET, Angular, SQL Server). The dev environment (`C:\Dev\tools`) already has Node 24 + pnpm, Rust + VS Build Tools (set up for Tauri) and no .NET SDK.

Criteria, in order: security, maintainability for the owner, packaging/size.

## Options

| | Tauri 2 + Angular | Electron + Angular | Avalonia (.NET) |
|---|---|---|---|
| Backend language | Rust | Node/TypeScript | C# |
| UI | Angular in system WebView2 | Angular in bundled Chromium | XAML |
| Security model | No Node in UI; IPC only through explicitly declared commands and capabilities; strict CSP | Node in main process; needs contextIsolation, sandbox, preload discipline; larger attack surface | Native process, no web surface; standard .NET hardening |
| Installer size (approx.) | 5–15 MB | 90–150 MB | 40–80 MB self-contained |
| Owner familiarity | Angular yes; Rust new | High (TS everywhere) | High (C#) |
| Applying DESIGN.md / HTML mocks | Direct (CSS tokens) | Direct (CSS tokens) | Manual translation to XAML styles |
| Toolchain ready | Yes | Yes | Needs .NET SDK installed in `C:\Dev\tools` |
| Charts / tables | Mature web ecosystem | Mature web ecosystem | LiveCharts2 / ScottPlot, fewer options |

## Decision

**Tauri 2 + Angular**, with a deliberately thin, boring Rust core:

- **Rust owns everything sensitive or stateful:** Enable Banking client (JWT RS256 signing, HTTP), loopback callback listener, encrypted SQLite, OS keyring, sync, import pipeline, domain rules (money, dedup, internal transfers, categorization).
- **Angular is presentation only:** it calls typed Tauri commands and never sees the private key, tokens or the database key.
- Tauri capabilities restricted to the commands we define; no shell, no arbitrary filesystem or HTTP from the WebView; strict CSP with no remote origins.

Rationale: best security posture and smallest footprint; the UI stays in the owner's strongest skill (Angular) and the web design workflow (DESIGN.md, HTML mocks) carries over unchanged. The Rust surface is small and conventional (serde, reqwest, rusqlite, jsonwebtoken, keyring), which keeps the learning curve bounded.

## Consequences

- Owner must read and review Rust for the core. Mitigation: small modules, extensive tests, no clever generics, docs per module.
- WebView2 (Evergreen) is present on Windows 11; macOS/Linux builds are possible later but out of scope.
- If Rust proves too costly to maintain, the fallback is **Avalonia/.NET**: domain logic and schema transfer directly; only the UI would be rewritten.

## Alternatives rejected

- **Electron + Angular:** familiar, but ~10× larger and a weaker default security model for an app holding bank tokens.
- **Avalonia/.NET:** most familiar backend, but loses the web design workflow, needs a new SDK install, and data-viz options are thinner.
