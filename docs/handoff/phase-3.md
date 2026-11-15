---
title: Handoff — phase 3 (data core)
---

# Handoff: phase 3 — data core

_Written 2026-11-15 at the end of phases 0–2._

## State

- Repo: `nihilistBlves/pocket-ledger` (public). GitHub Pages serves `docs/` (privacy, terms, `callback.html` bridge).
- Branches: `main` (phase 0, pushed) → `phase-1/architecture` → `phase-2/design` (local).
- Accepted ADRs: [0001 stack](../adr/0001-desktop-stack.md) (Tauri 2 + Angular, Rust owns data/secrets), [0002 DB](../adr/0002-local-database.md) (SQLCipher, cents), [0003 secrets](../adr/0003-secrets-storage.md) (keyring, `.pem` via `ENABLE_BANKING_KEY_PATH`), [0004 callback](../adr/0004-authorization-callback.md).
- API notes: [research/enable-banking.md](../research/enable-banking.md) (dedup must not rely on `transaction_id`).
- Design: [DESIGN.md](../../DESIGN.md) + approved mocks in `design/mocks/`.
- Subagents: `.claude/agents/` → `dev` (sonnet), `scout` (haiku), `chore` (haiku).
- Toolchain (via `C:\Dev\tools`, visible from Bash): Node 24, pnpm 12, Rust/cargo 1.99. No .NET SDK.

## Phase 3 goal

Scaffold the Tauri 2 + Angular app (pnpm, `src/` + `src-tauri/`) and build the Rust data core:

1. Schema + migrations: accounts, transactions (signed cents, booking/value date, description, counterparty, external id / fingerprint, status pending|booked), categories (tree), rules, budgets, recurring, tags (+ join), goals, sync_runs, import_batches.
2. Domain: money parsing (decimal string → cents, no float), dedup (entry_reference else fingerprint + occurrence index; pending→booked updates), internal-transfer detection (own accounts, opposite amounts, ±N days), rule-based categorization that learns from corrections.
3. CSV/Excel import through the same pipeline + dedup.
4. Tests for all of the above with fictitious data.

## Decisions still to take with the owner (AskUserQuestion)

- Default category tree (Spanish, seeded) — propose one.
- Transfer-matching window and tie-breaking.
- Which CSV/Excel layouts to support first (bank export format).
- Angular version / UI component approach (plain CSS tokens from DESIGN.md vs a headless lib).

## Rules reminder

No secrets/real data in repo, logs or tests. Ask before installing global deps, pushing, or any bank action. Opus reviews money, dedup, crypto and sync code.
