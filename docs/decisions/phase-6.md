---
title: Phase 6 decisions
---

# Phase 6 decisions (Resumen and Movimientos)

Taken on 2026-11-16 while building the dashboard (`/resumen`) and the transaction list (`/movimientos`) from the approved mocks `design/mocks/dashboard.html` and `design/mocks/transactions.html`.

## Accounting rules for the screens

- **Effective date** of a transaction: `COALESCE(booking_date, value_date, date(created_at))`. Pending rows count in the month of their effective date.
- **Pending rows are included** in totals and lists (the mock shows "incluye 1 pendiente"); the UI marks them with a "Pendiente" badge and italic description.
- **Internal transfers are excluded** from income, expense, balance and category totals. A row is a transfer when its category has `kind = 'transfer'` (this covers both auto-linked pairs and rows the owner moved to "Transferencias internas" by hand). Rows in the list still show, with `⇄` and a muted amount.
- **Income** = sum of positive non-transfer amounts; **expense** = absolute sum of negative non-transfer amounts. Sign decides, not the category kind (a refund in an expense category is income for the month's totals). Simple and predictable; revisit with budgets.
- **Savings rate** = balance / income, shown only when income > 0. Its delta is in percentage points (`p. p.`).
- **Month-over-month delta**: for the current (incomplete) month compare day 1..today with the same day range of the previous month (clamped to that month's length), labelled `vs. 1–15 oct`. For past months compare full months, labelled `vs. oct 2026`. Expense going down is "good" (olive), up is "bad" (terracotta); arrows and sign always shown.
- **Spending by category** groups by **top-level category** (the group, e.g. "Alimentación"), sorted by amount, top 6 plus "Otros (n)". Uncategorized expense shows as "Sin categoría". Colours follow the DESIGN.md categorical order by rank.
- **Evolution chart**: last 12 months ending at the selected month; current month marked `*` and "hasta el N".
- **Balance per account**: the latest bank balance (`account_balances`), not a sum of transactions.
- Amounts are integer cents end to end; formatting happens only in the UI (`core/format.ts`).
- Multi-currency: the app is EUR-only today; queries sum `amount_cents` without conversion and DTOs carry `currency`.

## API contract (Tauri commands)

All take/return camelCase JSON (serde `rename_all = "camelCase"`), cents as integers, dates `YYYY-MM-DD`. No secrets, IBANs, provider ids or raw hashes. Errors are the existing `CommandError { code, message }`.

```ts
// ledger_dashboard({ month: 'YYYY-MM', today: 'YYYY-MM-DD' }) -> Dashboard
interface Dashboard {
  month: string;                 // 'YYYY-MM'
  isCurrentMonth: boolean;
  throughDay: number | null;     // day of month the current-month figures run to; null for past months
  totals: PeriodTotals;          // selected month (to throughDay when current)
  previous: PeriodTotals;        // comparison period (same day range of previous month, or full previous month)
  previousLabelRange: { from: string; to: string }; // dates of the comparison period
  evolution: MonthTotals[];      // 12 entries, oldest first, ending at `month` (full months except the last)
  categories: CategorySpend[];   // top-level groups, desc by amount (UI folds after 6)
  accounts: AccountBalance[];
  recent: TxRow[];               // latest 6 of the selected month, newest first
  monthTxCount: number;          // transactions in the selected month (for "Ver todos (n)")
}
interface PeriodTotals { incomeCents: number; expenseCents: number; balanceCents: number; }
interface MonthTotals extends PeriodTotals { month: string; }
interface CategorySpend { categoryId: number | null; name: string; key: string | null; expenseCents: number; txCount: number; }
interface AccountBalance { id: number; name: string; kind: string; displaySuffix: string | null;
  balanceCents: number | null; currency: string; balanceDate: string | null; lastSyncedAt: string | null; }

// ledger_transactions(filter: TxFilter) -> TxPage
interface TxFilter {
  from?: string; to?: string;            // inclusive effective dates
  accountId?: number; categoryId?: number; // categoryId matches the category or any child of it
  uncategorized?: boolean;
  text?: string;                          // accent/case-insensitive match on description, counterparty, remittance, notes
  minCents?: number; maxCents?: number;   // on the absolute amount
  direction?: 'in' | 'out';
  status?: 'pending' | 'booked';
  offset?: number; limit?: number;        // limit default 100, max 500
}
interface TxPage {
  rows: TxRow[];                          // newest first (effective date desc, id desc)
  total: number;                          // rows matching the filter
  summary: { incomeCents: number; expenseCents: number; netCents: number; transferCount: number; pendingCount: number; };
}
interface TxRow {
  id: number; accountId: number; accountName: string;
  status: 'pending' | 'booked';
  date: string;                           // effective date
  bookingDate: string | null; valueDate: string | null;
  amountCents: number; currency: string;
  description: string; counterparty: string | null; remittance: string | null;
  categoryId: number | null; categoryName: string | null; categoryKey: string | null;
  parentCategoryId: number | null; parentCategoryName: string | null;
  categorySource: 'none' | 'rule' | 'learned' | 'manual' | 'transfer';
  isTransfer: boolean; transferPeerId: number | null; transferPeerAccountName: string | null;
  notes: string | null; tags: string[];
}

// ledger_categories() -> CategoryOption[]   (tree flattened, groups first then children, sort_order)
interface CategoryOption { id: number; parentId: number | null; key: string | null; name: string; kind: 'income'|'expense'|'transfer'|'system'; }
// ledger_tags() -> string[]                 (all tag names, for suggestions)

// Mutations; each returns the updated TxRow (plus side effects)
// ledger_set_category({ txId, categoryId, createRule: boolean }) -> { row: TxRow; recategorized: number; ruleId: number | null }
//   -> rules::learn_from_correction; if createRule also rules::create_manual_rule(NewRule::contains(<normalized merchant>, categoryId))
// ledger_set_notes({ txId, notes: string | null }) -> TxRow   (trimmed; empty -> null; max 2000 chars)
// ledger_set_tags({ txId, tags: string[] }) -> TxRow          (trimmed, deduped case-insensitively, max 20 tags x 40 chars; unused tags are kept)
// ledger_unlink_transfer({ txId }) -> TxRow[]                 (-> transfers::unlink_transfer, both rows after re-categorization)
```

Mutations run inside a SQLite transaction. The UI refreshes the affected rows and the summary after each mutation and listens to `bank://synced` to reload.

## Implementation notes (as built)

- Command arguments are top-level: `ledger_dashboard({ month, today })`, `ledger_transactions({ filter })`, `ledger_set_category({ txId, categoryId, createRule })`, `ledger_set_notes({ txId, notes })`, `ledger_set_tags({ txId, tags })`, `ledger_unlink_transfer({ txId })`.
- New error codes: `invalid_input` (bad month/date, limit > 500, notes or tags too long) and `not_found`.
- All queries cover own accounts only (`is_own = 1`). "Sin categoría" (dashboard group and list filter) means a null category or the seeded `uncategorized` category.
- For the current month, totals, categories and the last evolution point run to `today`; `recent` and `monthTxCount` cover the whole calendar month (what "Ver todos" opens).
- `ruleId` in the set-category result is the manual rule's id when `createRule` is set (an identical manual rule is repointed, not duplicated); learned rules stay internal.
- Text search normalizes in Rust (`normalize_text`, accent/case-insensitive) and filters by id through `json_each`, since rusqlite's `functions` feature is not enabled. Date filters use the effective-date expression and cannot use `ix_transactions_booking`; fine at personal-data sizes.
- The Cuenta filter takes its options from `bank_overview().accounts` (same account ids).
- `/movimientos?q=` without `month` searches the whole history ("Todo el historial"); with `month` it filters that month. "Fechas" (Desde/Hasta) overrides the month.
- The evolution chart is hand-written SVG in pixel coordinates (fixed 260px plot, 12px labels, width from a `ResizeObserver`), with a visually hidden data table.
- Category colours: by rank on the dashboard (DESIGN.md categorical order) and by top-level group position in the list, so the same group can differ between screens. Unify when the Categorías screen lets the owner pick colours (`categories.color`).
- The detail panel cannot set a row back to "Sin categoría" (`categoryId` is required).
- Global `.card` now lives in `src/styles/components.css`; the shell's encapsulated `app.css` keeps its own copy. The shell's "Estado de la aplicación" card still shows above every page; it belongs to the shell work.

## Build note

Inside a deep git worktree (`.claude\worktrees\…`) the vendored OpenSSL build of SQLCipher fails on Windows MAX_PATH (C1083). Workaround: build with `CARGO_TARGET_DIR` pointing to the main checkout's `src-tauri\target`.

## Verification

`cargo test --workspace` (251 tests), `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm build` and `pnpm test` (48 tests) pass. Checked in the running app against the Enable Banking sandbox (Mock ASPSP) only: both screens at 1216px and 2576px wide in the dark theme, row selection, and saving a tag and a note.

## Scope cuts (deferred)

- **Exportar CSV** (mock button): needs a save-file dialog plugin and a file-write capability; deferred to the import/export phase.
- Sidebar icons, sync status and theme toggle from the mocks belong to the shell; the Ajustes/shell work runs in another session, so this phase only fills the two routes.
