/** Shapes of the `ledger_*` Tauri commands (see docs/decisions/phase-6.md). Amounts are integer cents. */

export interface PeriodTotals {
  incomeCents: number;
  expenseCents: number;
  balanceCents: number;
}

export interface MonthTotals extends PeriodTotals {
  /** `YYYY-MM` */
  month: string;
}

export interface CategorySpend {
  categoryId: number | null;
  name: string;
  key: string | null;
  expenseCents: number;
  txCount: number;
}

export interface AccountBalance {
  id: number;
  name: string;
  kind: string;
  displaySuffix: string | null;
  balanceCents: number | null;
  currency: string;
  balanceDate: string | null;
  lastSyncedAt: string | null;
}

export type TxStatus = 'pending' | 'booked';
export type CategorySource = 'none' | 'rule' | 'learned' | 'manual' | 'transfer';

export interface TxRow {
  id: number;
  accountId: number;
  accountName: string;
  status: TxStatus;
  date: string;
  bookingDate: string | null;
  valueDate: string | null;
  amountCents: number;
  currency: string;
  description: string;
  counterparty: string | null;
  remittance: string | null;
  categoryId: number | null;
  categoryName: string | null;
  categoryKey: string | null;
  parentCategoryId: number | null;
  parentCategoryName: string | null;
  categorySource: CategorySource;
  isTransfer: boolean;
  transferPeerId: number | null;
  transferPeerAccountName: string | null;
  notes: string | null;
  tags: string[];
}

export interface Dashboard {
  month: string;
  isCurrentMonth: boolean;
  throughDay: number | null;
  totals: PeriodTotals;
  previous: PeriodTotals;
  previousLabelRange: { from: string; to: string };
  evolution: MonthTotals[];
  categories: CategorySpend[];
  accounts: AccountBalance[];
  recent: TxRow[];
  monthTxCount: number;
}

export interface TxFilter {
  from?: string;
  to?: string;
  accountId?: number;
  categoryId?: number;
  uncategorized?: boolean;
  text?: string;
  minCents?: number;
  maxCents?: number;
  direction?: 'in' | 'out';
  status?: 'pending' | 'booked';
  offset?: number;
  limit?: number;
}

export interface TxSummary {
  incomeCents: number;
  expenseCents: number;
  netCents: number;
  transferCount: number;
  pendingCount: number;
}

export interface TxPage {
  rows: TxRow[];
  total: number;
  summary: TxSummary;
}

export interface CategoryOption {
  id: number;
  parentId: number | null;
  key: string | null;
  name: string;
  kind: CategoryKind;
  /** Stored colour (null = default / inherit). */
  color: ColorToken | null;
}

export interface SetCategoryResult {
  row: TxRow;
  recategorized: number;
  ruleId: number | null;
}

// ---- Phase 7: categories, rules, budgets (see docs/decisions/phase-7.md) ----

export type ColorToken = 'cat-1' | 'cat-2' | 'cat-3' | 'cat-4' | 'cat-5' | 'cat-6' | 'cat-7' | 'cat-8' | 'other';
export type CategoryKind = 'income' | 'expense' | 'transfer' | 'system';

export interface CategoryNode {
  id: number;
  parentId: number | null;
  key: string | null;
  name: string;
  kind: CategoryKind;
  isSystem: boolean;
  sortOrder: number;
  /** Stored colour (null = default / inherit). */
  color: ColorToken | null;
  childCount: number;
  monthTxCount: number;
  monthAmountCents: number;
  totalTxCount: number;
  ruleCount: number;
  budget: 'own' | 'group' | 'children' | null;
}

export interface DeleteCategoryResult {
  movedTransactions: number;
  repointedRules: number;
  deletedRules: number;
  deletedBudgets: number;
}

export type RuleField = 'any' | 'description' | 'counterparty';
export type RuleMatchType = 'contains' | 'equals';
export type RuleDirection = 'any' | 'in' | 'out';

export interface RuleInput {
  pattern: string;
  field: RuleField;
  matchType: RuleMatchType;
  accountId: number | null;
  direction: RuleDirection;
  categoryId: number;
  priority: number;
}

export interface RuleRow extends RuleInput {
  id: number;
  order: number;
  origin: 'manual' | 'learned';
  accountName: string | null;
  categoryName: string;
  parentCategoryName: string | null;
  hits: number;
  lastHitAt: string | null;
  createdAt: string;
}

export interface SaveRuleResult {
  rule: RuleRow;
  recategorized: number;
}

export type RuleOutcome = 'change' | 'unchanged' | 'manual' | 'other_rule';

export interface RuleTestRow {
  txId: number;
  date: string;
  description: string;
  counterparty: string | null;
  amountCents: number;
  categoryId: number | null;
  categoryName: string | null;
  outcome: RuleOutcome;
  winningRuleId: number | null;
  winningRulePattern: string | null;
  winningRuleOrder: number | null;
}

export interface RuleTest {
  from: string;
  to: string;
  matched: number;
  change: number;
  unchanged: number;
  manual: number;
  otherRule: number;
  rows: RuleTestRow[];
}

export interface BudgetRow {
  categoryId: number;
  name: string;
  parentId: number | null;
  parentName: string | null;
  isGroup: boolean;
  childNames: string[];
  /** Effective colour (own, else the group's, else null). */
  color: ColorToken | null;
  amountCents: number;
  spentCents: number;
  remainingCents: number;
  /** spent * 10000 / amount; may exceed 10000 or be negative. */
  percentBp: number;
  status: 'ok' | 'warn' | 'over';
  avg3Cents: number;
  startMonth: string;
  previousAmountCents: number | null;
}

export interface UnbudgetedRow {
  categoryId: number;
  name: string;
  parentName: string | null;
  color: ColorToken | null;
  spentCents: number;
  avg3Cents: number;
}

export interface BudgetMonth {
  month: string;
  isCurrentMonth: boolean;
  dayOfMonth: number | null;
  daysInMonth: number;
  totals: {
    budgetCents: number;
    spentCents: number;
    remainingCents: number;
    overCount: number;
    overCents: number;
    count: number;
  };
  budgets: BudgetRow[];
  unbudgeted: UnbudgetedRow[];
  uncategorizedCents: number;
}
