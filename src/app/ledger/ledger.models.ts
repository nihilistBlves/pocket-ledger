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
  kind: 'income' | 'expense' | 'transfer' | 'system';
}

export interface SetCategoryResult {
  row: TxRow;
  recategorized: number;
  ruleId: number | null;
}
