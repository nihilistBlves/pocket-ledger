---
title: Phase 7 decisions
---

# Phase 7 decisions (Categorías, Reglas, Presupuestos)

Taken with the owner on 2026-11-16. Approved mocks: `design/mocks/categories.html`, `rules.html`, `budgets.html` (shared pieces in `design/mocks/extras.css`). Branch stacked on phase 6 (PR #6), which was still open when this phase started.

## Owner decisions

- **Budget level:** a budget goes on an expense **group or on its subcategories, never both** (no double counting). A group budget covers the group's own rows and all its subcategories.
- **Colour:** groups and subcategories store an optional colour token from a fixed 9-colour palette (`cat-1`…`cat-8`, `other`; DESIGN.md chart palette). A subcategory without its own colour inherits its group's ("Del grupo").
- **Pending rows count** toward budget spend, like every other total (phase 6).
- **Internal transfers never count** (category `kind = 'transfer'`), as in phase 6.

## Design rules from the mocks

### Categories
- Two levels only: groups (top level) and subcategories. A group cannot become a subcategory; a subcategory can move to another group **of the same kind** (`income`/`expense`).
- **System categories** (`is_system = 1`: "Transferencias internas", "Sin categoría") cannot be renamed, moved, deleted or receive subcategories; only their colour can change.
- New group: name + kind (`expense` default or `income`). New subcategory: name; kind inherited from the group. Names are trimmed, 1–60 chars, unique per parent (case-insensitive check in code; the DB index is case-sensitive).
- **Delete:** not allowed for system categories or for a group that still has subcategories. Transactions move to a chosen target (default: the parent group; any non-transfer category, including "Sin categoría"); `category_source` is kept, so manual rows stay manual. Rules pointing to the category are **repointed** to the target (default) or **deleted**. Budgets of the deleted category are removed (FK cascade); the dialog says so when it has one.
- Default colours when nothing is stored: top-level groups by position in the DESIGN.md categorical order (same as the Movimientos list), so seeded data looks the same as before.

### Rules
- Shown in **evaluation order** (unchanged from phase 3): manual before learned, then priority desc, pattern length desc, id desc.
- Priority in the UI: **Alta = 10, Normal = 0, Baja = −10** (any integer is accepted by the core).
- **Editing a learned rule turns it manual.** Saving a rule identical (pattern, field, match type, account, direction) to another one is refused (`conflict`).
- **Test a rule** against the last 90 days (to `today`): each matching own-account row gets an outcome: `change` (would move to the rule's category), `unchanged` (already there), `manual` (categorized by hand or a transfer: never touched), `other_rule` (a rule earlier in the order wins; its id and pattern are returned). The test simulates the rule as it would be saved (manual origin, new values).
- **Apply on save** (checkbox, default on): re-runs the rules on the rows of the same 90-day window that the saved rule matches and whose source is `none`, `rule` or `learned`. Manual and transfer rows are never modified.
- **"Aplicar a sin categoría"** runs all rules over own-account rows with `category_source = 'none'` or the `uncategorized` category, all history.
- Deleting a rule keeps the categories it already set (`rule_id` becomes NULL by FK).

### Budgets
- Monthly only. A budget row is valid from `start_month` to `end_month` (NULL = open). For a month M the active budget of a category is the row with `start_month <= M` and (`end_month` NULL or `>= M`). At most one active row per category per month.
- **Set amount from month M** ("Aplicar desde"): if the active row starts at M, update its amount; else close the active row at M−1 and insert a new row from M. Earlier months keep their amount.
- **Remove from month M:** a row starting at M is deleted; an older row is closed at M−1. History is kept.
- Only `expense` categories can have a budget. Setting one on a category whose group or subcategories already have an active budget in any month ≥ M is refused (`conflict`).
- **Spent** = −(sum of `amount_cents`) of non-transfer own-account rows (pending included) whose effective date is in M, over the category (plus its subcategories for a group). Refunds reduce spend; spent can be negative and is shown as is.
- **Status:** `over` when spent > amount, `warn` when spent ≥ 90 % of amount, else `ok` (DESIGN.md progress bar). Percent in basis points (integer).
- **Pace marker** (current month only): day of month / days in month, computed in the UI from `today`.
- **Average 3 months:** net spend over the three full months before M, divided by 3 (integer, rounded half away from zero).
- **Gasto sin presupuesto:** expense categories with positive spend in M not covered by any active budget (neither their own, their group's nor — for a group's own rows — any subcategory's). Listed at the level the spend happened (a subcategory, or a group for rows directly on the group). "Sin categoría" spend is reported separately.

## API contract (Tauri commands)

Same conventions as phase 6: camelCase JSON, cents as integers, dates `YYYY-MM-DD`, months `YYYY-MM`, top-level arguments, errors as `CommandError { code, message }`. New error code **`conflict`** (duplicate name/rule, budget overlap, delete blocked) with a generic message; the UI shows a specific Spanish text per action.

```ts
type ColorToken = 'cat-1'|'cat-2'|'cat-3'|'cat-4'|'cat-5'|'cat-6'|'cat-7'|'cat-8'|'other';
type CategoryKind = 'income'|'expense'|'transfer'|'system';

// ledger_category_tree({ month }) -> CategoryNode[]   (groups in sort_order, each followed by its children)
interface CategoryNode {
  id: number; parentId: number | null; key: string | null; name: string;
  kind: CategoryKind; isSystem: boolean; sortOrder: number;
  color: ColorToken | null;          // stored value (null = default / inherit)
  childCount: number;
  monthTxCount: number;              // rows in `month` (groups include their children)
  monthAmountCents: number;          // signed net sum in `month`; for kind 'transfer' the absolute sum of outflows
  totalTxCount: number;              // all history (groups include children)
  ruleCount: number;                 // rules pointing to this category (groups include children)
  budget: 'own' | 'group' | 'children' | null; // active budget in `month`: on itself, on its group, on its subcategories
}
// ledger_create_category({ parentId: number | null, name: string, kind: 'income'|'expense' | null }) -> CategoryNode
//   kind required for a group, ignored for a subcategory
// ledger_update_category({ id, name: string, parentId: number | null, color: ColorToken | null }) -> CategoryNode
// ledger_delete_category({ id, moveTo: number, deleteRules: boolean })
//   -> { movedTransactions: number; repointedRules: number; deletedRules: number; deletedBudgets: number }

// ledger_rules() -> RuleRow[]  (evaluation order; `order` is 1-based)
interface RuleInput {
  pattern: string;                   // normalized by the core; 1–100 chars after normalization
  field: 'any'|'description'|'counterparty';
  matchType: 'contains'|'equals';
  accountId: number | null;
  direction: 'any'|'in'|'out';
  categoryId: number;                // not a transfer/system category
  priority: number;
}
interface RuleRow extends RuleInput {
  id: number; order: number; origin: 'manual'|'learned';
  accountName: string | null; categoryName: string; parentCategoryName: string | null;
  hits: number; lastHitAt: string | null; createdAt: string;
}
// ledger_save_rule({ id: number | null, rule: RuleInput, applyToExisting: boolean, today: string })
//   -> { rule: RuleRow; recategorized: number }
// ledger_delete_rule({ id }) -> void
// ledger_test_rule({ id: number | null, rule: RuleInput, today: string }) -> RuleTest
interface RuleTest {
  from: string; to: string;          // the 90-day window
  matched: number; change: number; unchanged: number; manual: number; otherRule: number;
  rows: RuleTestRow[];               // max 50: change, other_rule, manual, unchanged; newest first within each
}
interface RuleTestRow {
  txId: number; date: string; description: string; counterparty: string | null; amountCents: number;
  categoryId: number | null; categoryName: string | null;
  outcome: 'change'|'unchanged'|'manual'|'other_rule';
  winningRuleId: number | null; winningRulePattern: string | null; winningRuleOrder: number | null;
}
// ledger_apply_rules_uncategorized() -> { recategorized: number }

// ledger_budgets({ month, today }) -> BudgetMonth
interface BudgetMonth {
  month: string; isCurrentMonth: boolean; dayOfMonth: number | null; daysInMonth: number;
  totals: { budgetCents: number; spentCents: number; remainingCents: number; overCount: number; overCents: number; count: number };
  budgets: BudgetRow[];              // sorted by percent desc
  unbudgeted: UnbudgetedRow[];       // sorted by spent desc
  uncategorizedCents: number;
}
interface BudgetRow {
  categoryId: number; name: string; parentId: number | null; parentName: string | null;
  isGroup: boolean; childNames: string[]; color: ColorToken | null; // effective colour (own, else group's, else null)
  amountCents: number; spentCents: number; remainingCents: number;
  percentBp: number;                 // spent * 10000 / amount, may exceed 10000 or be negative
  status: 'ok'|'warn'|'over';
  avg3Cents: number; startMonth: string; previousAmountCents: number | null; // amount active in the month before startMonth
}
interface UnbudgetedRow { categoryId: number; name: string; parentName: string | null; color: ColorToken | null; spentCents: number; avg3Cents: number; }
// ledger_set_budget({ categoryId, month, amountCents }) -> void   (amount 1 .. 100_000_000 cents)
// ledger_remove_budget({ categoryId, month }) -> void
```

`ledger_categories()` (phase 6) also gains `color: ColorToken | null` (stored value) so the dashboard and the list can use picked colours.

## As built

- **Rust:** `ledger-core/src/ledger/categories_admin.rs`, `rules_admin.rs`, `budgets.rs` (+ `admin_tests.rs`); `rules.rs` only exposes `StoredRule`, `TxView`, `load_rules` and `rule_matches` to the crate (and loads `priority`), behaviour unchanged. Commands in `src-tauri/src/ledger_admin_commands.rs`. No migration: `categories.color` and `budgets` already existed.
- **Angular:** `src/app/categories/`, `src/app/rules/`, `src/app/budgets/`; shared `ledger/colors.ts` (token → `c0`…`c8` class with the group-position default) and `ledger/modal.ts` (CDK focus trap, Escape/backdrop close). Resumen and Movimientos use a stored colour when there is one.
- `ledger_create_category` / `ledger_update_category` return the node with month figures at 0/null (no month in the call); the UI reloads the tree after every mutation.
- `totalTxCount` counts all accounts (it must match what a delete moves); month figures are own accounts only.
- Delete errors: a group with subcategories → `conflict`; a system category or a transfer target → `invalid_input`. Moving a subcategory into a group whose budget overlaps in time → `conflict`.
- Deleting with "Sin categoría" (system) as target always deletes the rules (rules cannot point to system categories); the dialog disables "Apuntarlas" and says so.
- New groups get the last `sort_order` (after the system groups).
- Rule test window: `today − 89` … `today`, inclusive. `recategorized` counts rows whose category actually changed. Patterns are stored normalized (upper case, no accents).
- `winningRuleOrder` in a test is the position in the *simulated* order (with the edited rule in its new place), so it can differ by one from the table when the edit moves the rule.
- Budgets: `percentBp` truncates; `warn`/`over` compare in integers. Setting a budget from M inherits the end of the row it splits; with nothing active at M the new row ends just before the next later row of that category. Removing from M also deletes rows scheduled after M.
- `uncategorizedCents` = outflows with no category or in "Sin categoría".
- A saved rule keeps its exact priority unless the owner picks another level (Alta/Normal/Baja send 10/0/−10).
- **Formatting:** the repo has no `rustfmt.toml` and existing code is not rustfmt-clean, so only the new files were formatted; existing files were edited by hand to keep the diff to real changes.

## Verification

`cargo test --workspace` (260 ledger-core + 17 app tests), `cargo clippy --workspace --all-targets -- -D warnings`, `pnpm build` and `pnpm test` (85 tests) pass.

## Known gaps

- The category panel shows the rule count without the manual/learned split (not in the contract).
- The rule test does not show the "sin cambio" count on its own.
- Sidebar footer items from the mocks (sync status, consent days, theme) still belong to the shell work.
