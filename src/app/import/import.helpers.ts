import { STRINGS } from '../core/strings';
import {
  ColumnRef,
  DecimalSpec,
  FileFormat,
  ImportProfile,
  RawTable,
  RowErrorReason,
} from './import.models';

const t = STRINGS.import;

// ---------------------------------------------------------------- date formats

/** Date formats offered in the mapping form: display text -> backend pattern (`%d %m %Y %y`). */
export const DATE_FORMATS: ReadonlyArray<{ label: string; pattern: string }> = [
  { label: 'dd/mm/aaaa', pattern: '%d/%m/%Y' },
  { label: 'dd-mm-aaaa', pattern: '%d-%m-%Y' },
  { label: 'aaaa-mm-dd', pattern: '%Y-%m-%d' },
  { label: 'dd/mm/aa', pattern: '%d/%m/%y' },
  { label: 'mm/dd/aaaa', pattern: '%m/%d/%Y' },
];

/** `%d/%m/%Y` -> `dd/mm/aaaa` (works for any supported pattern). */
export function dateFormatLabel(pattern: string): string {
  return pattern.replace(/%([dmYy])/g, (_, c: string) => ({ d: 'dd', m: 'mm', Y: 'aaaa', y: 'aa' })[c] ?? c);
}

/** `dd/mm/aaaa` -> `%d/%m/%Y`; null for text that is not one of the offered labels. */
export function datePatternFor(label: string): string | null {
  return DATE_FORMATS.find((f) => f.label === label)?.pattern ?? null;
}

/** The offered formats plus `current` when it is none of them (e.g. from a stored profile). */
export function dateFormatOptions(current: string | undefined): Array<{ label: string; pattern: string }> {
  const list = [...DATE_FORMATS];
  if (current && !list.some((f) => f.pattern === current)) {
    list.push({ label: dateFormatLabel(current), pattern: current });
  }
  return list;
}

// --------------------------------------------------------------- decimal style

export type AmountFormatKey = 'es' | 'en' | 'plain';

export const AMOUNT_FORMATS: ReadonlyArray<{ key: AmountFormatKey; label: string; spec: DecimalSpec }> = [
  { key: 'es', label: '1.234,56', spec: { decimal: ',', thousands: '.' } },
  { key: 'en', label: '1,234.56', spec: { decimal: '.', thousands: ',' } },
  { key: 'plain', label: '1234.56', spec: { decimal: '.', thousands: null } },
];

export function decimalSpecFor(key: AmountFormatKey): DecimalSpec {
  return { ...AMOUNT_FORMATS.find((f) => f.key === key)!.spec };
}

/** Which offered format a spec is; null for a custom one. */
export function amountFormatKey(spec: DecimalSpec): AmountFormatKey | null {
  return AMOUNT_FORMATS.find((f) => f.spec.decimal === spec.decimal && f.spec.thousands === spec.thousands)?.key ?? null;
}

// ----------------------------------------------------------------- column refs

/** Lowercase, no accents, trimmed: how the backend matches header names. */
export function fold(text: string): string {
  return text.normalize('NFD').replace(/\p{M}/gu, '').trim().toLowerCase();
}

export function isName(ref: ColumnRef): ref is { Name: string } {
  return 'Name' in ref;
}

export function sameRef(a: ColumnRef | null, b: ColumnRef | null): boolean {
  if (a === null || b === null) return a === b;
  if (isName(a) && isName(b)) return fold(a.Name) === fold(b.Name);
  if (!isName(a) && !isName(b)) return a.Index === b.Index;
  return false;
}

/** `0` -> `A`, `25` -> `Z`, `26` -> `AA`. */
export function columnLetter(index: number): string {
  let n = index;
  let out = '';
  do {
    out = String.fromCharCode(65 + (n % 26)) + out;
    n = Math.floor(n / 26) - 1;
  } while (n >= 0);
  return out;
}

// ---------------------------------------------------------------- mapping form

export type AmountMode = 'single' | 'split';

/** Editable state of the generic mapping, flattened from an `ImportProfile`. */
export interface MappingForm {
  format: FileFormat;
  /** `null` = find the header automatically. */
  skipRows: number | null;
  hasHeader: boolean;
  /** The first one is the chosen format; the rest come from a stored profile and are kept. */
  dateFormats: string[];
  decimal: DecimalSpec;
  currency: string;
  bookingDate: ColumnRef | null;
  valueDate: ColumnRef | null;
  description: ColumnRef | null;
  counterparty: ColumnRef | null;
  amountMode: AmountMode;
  amount: ColumnRef | null;
  debit: ColumnRef | null;
  credit: ColumnRef | null;
  /** Columns the form has no control for; preserved as they came. */
  remittance: ColumnRef | null;
  entryReference: ColumnRef | null;
}

export function formFromProfile(p: ImportProfile): MappingForm {
  const a = p.columns.amount;
  const single = 'Single' in a;
  return {
    format: structuredClone(p.format),
    skipRows: p.skip_rows,
    hasHeader: p.has_header,
    dateFormats: [...p.date_formats],
    decimal: { ...p.decimal },
    currency: p.currency,
    bookingDate: p.columns.booking_date,
    valueDate: p.columns.value_date,
    description: p.columns.description,
    counterparty: p.columns.counterparty,
    amountMode: single ? 'single' : 'split',
    amount: single ? a.Single.column : null,
    debit: single ? null : a.DebitCredit.debit,
    credit: single ? null : a.DebitCredit.credit,
    remittance: p.columns.remittance,
    entryReference: p.columns.entry_reference,
  };
}

/** Builds the profile the backend runs; null while a required column is missing. */
export function profileFromForm(f: MappingForm): ImportProfile | null {
  if (!f.bookingDate || !f.description || f.dateFormats.length === 0) return null;
  let amount: ImportProfile['columns']['amount'];
  if (f.amountMode === 'single') {
    if (!f.amount) return null;
    amount = { Single: { column: f.amount } };
  } else {
    if (!f.debit || !f.credit) return null;
    amount = { DebitCredit: { debit: f.debit, credit: f.credit } };
  }
  return {
    format: structuredClone(f.format),
    skip_rows: f.skipRows,
    has_header: f.hasHeader,
    columns: {
      booking_date: f.bookingDate,
      value_date: f.valueDate,
      description: f.description,
      counterparty: f.counterparty,
      remittance: f.remittance,
      entry_reference: f.entryReference,
      amount,
    },
    date_formats: [...f.dateFormats],
    decimal: { ...f.decimal },
    currency: f.currency,
  };
}

/** Header names the form refers to (those that need a header row to be found). */
function requiredNames(f: MappingForm): string[] {
  const refs = [f.bookingDate, f.description, ...(f.amountMode === 'single' ? [f.amount] : [f.debit, f.credit])];
  return refs.filter((r): r is { Name: string } => r !== null && isName(r)).map((r) => r.Name);
}

/**
 * Position of the header row among the raw rows (the backend counts positions the same way);
 * -1 when the form has no header or the automatic search finds nothing in the rows shown.
 */
export function headerRowIndex(rows: string[][], f: MappingForm): number {
  if (!f.hasHeader) return -1;
  if (f.skipRows !== null) return f.skipRows < rows.length ? f.skipRows : -1;
  const names = requiredNames(f).map(fold);
  if (names.length === 0) return -1;
  return rows.findIndex((row) => {
    const cells = row.map(fold);
    return names.every((n) => cells.includes(n));
  });
}

/** First row with at least two filled cells: a good guess for the header of a file nobody described. */
export function guessHeaderRow(rows: string[][]): number {
  const i = rows.findIndex((r) => r.filter((c) => c.trim() !== '').length >= 2);
  return i >= 0 ? i : 0;
}

export interface ColumnOption {
  index: number;
  /** `A · Fecha` or just `A`. */
  label: string;
  ref: ColumnRef;
}

/** Options of the column selects: by header name when there is a header, by position otherwise. */
export function columnOptions(raw: RawTable | null, f: MappingForm): ColumnOption[] {
  if (!raw) return [];
  const header = headerRowIndex(raw.rows, f);
  const headerRow = header >= 0 ? raw.rows[header] : null;
  const out: ColumnOption[] = [];
  for (let i = 0; i < raw.columnCount; i++) {
    const text = headerRow?.[i]?.trim() ?? '';
    out.push({
      index: i,
      label: text ? `${columnLetter(i)} · ${text}` : columnLetter(i),
      ref: f.hasHeader && text ? { Name: text } : { Index: i },
    });
  }
  return out;
}

/**
 * Re-expresses the column refs when "first row is the header" is toggled, so by-name refs
 * never meet a profile without header (the backend refuses that).
 */
export function convertRefs(f: MappingForm, raw: RawTable | null, toHeader: boolean): MappingForm {
  const headerIdx = raw ? headerRowIndex(raw.rows, { ...f, hasHeader: true }) : -1;
  const header = headerIdx >= 0 && raw ? raw.rows[headerIdx] : [];
  const conv = (r: ColumnRef | null): ColumnRef | null => {
    if (r === null) return null;
    if (toHeader) {
      if (isName(r)) return r;
      const text = header[r.Index]?.trim();
      return text ? { Name: text } : r;
    }
    if (!isName(r)) return r;
    const idx = header.findIndex((c) => fold(c) === fold(r.Name));
    return idx >= 0 ? { Index: idx } : null;
  };
  return {
    ...f,
    hasHeader: toHeader,
    bookingDate: conv(f.bookingDate),
    valueDate: conv(f.valueDate),
    description: conv(f.description),
    counterparty: conv(f.counterparty),
    amount: conv(f.amount),
    debit: conv(f.debit),
    credit: conv(f.credit),
    remittance: conv(f.remittance),
    entryReference: conv(f.entryReference),
  };
}

/** Role label for each column of the raw table head ("Fecha de operación", "Concepto"...). */
export function columnRoles(options: ColumnOption[], f: MappingForm, roleNames: Record<string, string>): string[] {
  const roles: Array<[ColumnRef | null, string]> = [
    [f.bookingDate, roleNames['bookingDate']],
    [f.valueDate, roleNames['valueDate']],
    [f.description, roleNames['description']],
    [f.counterparty, roleNames['counterparty']],
    ...(f.amountMode === 'single'
      ? ([[f.amount, roleNames['amount']]] as Array<[ColumnRef | null, string]>)
      : ([[f.debit, roleNames['debit']], [f.credit, roleNames['credit']]] as Array<[ColumnRef | null, string]>)),
  ];
  return options.map((o) => roles.find(([ref]) => ref !== null && sameRef(ref, o.ref))?.[1] ?? '');
}

/** Only the part of the format that changes what `import_raw_rows` returns. */
export function formatKey(format: FileFormat): string {
  return JSON.stringify(format);
}

// ------------------------------------------------------------------ profile text

/** `Fechas dd/mm/aaaa, importe con signo, cabecera en la fila 6.` */
export function describeProfile(p: ImportProfile): string {
  const d = t.describeProfile;
  const dates = d.dates(p.date_formats.map(dateFormatLabel).join(' o '));
  const amount = 'Single' in p.columns.amount ? d.amountSingle : d.amountSplit;
  const header = !p.has_header ? d.noHeader : p.skip_rows === null ? d.headerAuto : d.headerAt(p.skip_rows + 1);
  return `${dates}, ${amount}, ${header}.`;
}

// ------------------------------------------------------------------ row errors

/** Spanish reason of a rejected row from its kind, column and problem only: never cell content. */
export function rowErrorText(reason: RowErrorReason): string {
  const e = t.rowErrors;
  switch (reason.kind) {
    case 'missing_value':
      return e.missing(reason.column);
    case 'invalid_date':
      return e.date(reason.column);
    case 'invalid_amount':
      return e.amount(reason.column, e.problems[reason.problem] ?? e.problems.malformed);
    case 'ambiguous_amount':
      return e.ambiguous;
    default:
      return e.problems.malformed;
  }
}

// ------------------------------------------------------------------ misc text

/** `2048` -> `2,0 KB`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const unit = bytes < 1024 * 1024 ? 'KB' : 'MB';
  const value = bytes / (unit === 'KB' ? 1024 : 1024 * 1024);
  return `${value.toFixed(1).replace('.', ',')} ${unit}`;
}

/** History cell text: `38 nuevos · 3 duplicados`, `Sin cambios · 44 duplicados`. */
export function batchCounts(b: { inserted: number; updated: number; duplicates: number }): string {
  const parts: string[] = [];
  if (b.inserted > 0) parts.push(t.newCount(b.inserted));
  if (b.updated > 0) parts.push(t.updatedCount(b.updated));
  if (b.duplicates > 0) parts.push(t.dupCount(b.duplicates));
  if (b.inserted === 0 && b.updated === 0) return t.noChanges(b.duplicates);
  return parts.join(' · ');
}
