/** Shapes of the `import_*` Tauri commands. Serde casing is mixed on purpose: it mirrors the Rust side. */

export type CsvEncoding = 'Utf8' | 'Windows1252';

export type FileFormat =
  | { type: 'Csv'; delimiter: string; encoding: CsvEncoding; quote: string }
  | { type: 'Excel'; sheet: string | null };

/** A column by header name or by 0-based position. */
export type ColumnRef = { Name: string } | { Index: number };

export type AmountColumns =
  | { Single: { column: ColumnRef } }
  | { DebitCredit: { debit: ColumnRef; credit: ColumnRef } };

export interface Columns {
  booking_date: ColumnRef;
  value_date: ColumnRef | null;
  description: ColumnRef;
  counterparty: ColumnRef | null;
  remittance: ColumnRef | null;
  entry_reference: ColumnRef | null;
  amount: AmountColumns;
}

export interface DecimalSpec {
  decimal: string;
  thousands: string | null;
}

export interface ImportProfile {
  format: FileFormat;
  skip_rows: number | null;
  has_header: boolean;
  columns: Columns;
  date_formats: string[];
  decimal: DecimalSpec;
  currency: string;
}

export interface ProfileView {
  name: string;
  builtin: boolean;
  profile: ImportProfile;
}

export interface PickedImport {
  fileName: string;
  sizeBytes: number;
  detectedProfile: string | null;
  suggestedFormat: FileFormat;
  profiles: ProfileView[];
}

export interface RawTable {
  rows: string[][];
  rowNumbers: number[];
  totalRows: number;
  columnCount: number;
}

export type AmountProblem = 'empty' | 'malformed' | 'too_many_decimals' | 'overflow';

export type RowErrorReason =
  | { kind: 'missing_value'; column: string }
  | { kind: 'invalid_date'; column: string }
  | { kind: 'invalid_amount'; column: string; problem: AmountProblem }
  | { kind: 'ambiguous_amount' };

export interface RowError {
  rowNumber: number;
  reason: RowErrorReason;
}

export type RowStatus = 'new' | 'duplicate' | 'updated';

export interface PreviewRow {
  rowNumber: number;
  date: string;
  description: string;
  amountCents: number;
  currency: string;
  status: RowStatus | null;
}

export type IdentityCheck =
  | { status: 'no_identity' | 'match' | 'will_adopt' | 'mismatch' | 'account_not_found' }
  | { status: 'belongs_to_other'; existingAccountId: number };

export interface ImportCounts {
  wouldInsert: number;
  duplicates: number;
  updated: number;
  transfersMatched: number;
  categorized: number;
  rejected: number;
}

export interface ImportPreview {
  totalRows: number;
  validRows: number;
  parseErrors: RowError[];
  parseErrorCount: number;
  rows: PreviewRow[];
  dateFrom: string | null;
  dateTo: string | null;
  hasIdentity: boolean;
  matchedAccountId: number | null;
  identity: IdentityCheck | null;
  counts: ImportCounts | null;
}

export interface ImportCommitResult {
  batchId: number;
  accountId: number;
  inserted: number;
  duplicates: number;
  updated: number;
  invalidRows: number;
  transfersMatched: number;
  categorized: number;
  profileSaved: boolean;
}

export interface ImportBatch {
  id: number;
  accountId: number;
  accountName: string;
  profile: string;
  fileName: string | null;
  importedAt: string;
  rowsTotal: number;
  inserted: number;
  updated: number;
  duplicates: number;
  errors: number;
  undoneAt: string | null;
  canUndo: boolean;
}

export interface UndoReport {
  deleted: number;
  transfersUnlinked: number;
}
