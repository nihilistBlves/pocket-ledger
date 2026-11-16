/** es-ES formatting helpers. Amounts stay integer cents until they are turned into text. */

const MINUS = '−';
const pad = (n: number): string => String(n).padStart(2, '0');

/** `123456` -> `1.234,56 €`; negative amounts use the typographic minus. */
export function formatCents(cents: number, currency = 'EUR'): string {
  const abs = Math.abs(Math.trunc(cents));
  const whole = Math.floor(abs / 100)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, '.');
  const text = `${whole},${pad(abs % 100)}`;
  const symbol = currency === 'EUR' ? '€' : currency;
  return `${cents < 0 ? MINUS : ''}${text} ${symbol}`;
}

function toDate(value: string | Date): Date | null {
  // Date-only strings are calendar dates: read them in local time, not as UTC midnight.
  const m = typeof value === 'string' ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(value) : null;
  const d = m
    ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]))
    : value instanceof Date
      ? value
      : new Date(value);
  return Number.isNaN(d.getTime()) ? null : d;
}

/** `dd/MM/yyyy` in the local time zone; `—` when the value is missing or invalid. */
export function formatDate(value: string | Date | null | undefined): string {
  const d = value ? toDate(value) : null;
  return d ? `${pad(d.getDate())}/${pad(d.getMonth() + 1)}/${d.getFullYear()}` : '—';
}

/** `dd/MM/yyyy HH:mm` in the local time zone. */
export function formatDateTime(value: string | Date | null | undefined): string {
  const d = value ? toDate(value) : null;
  return d ? `${formatDate(d)} ${pad(d.getHours())}:${pad(d.getMinutes())}` : '—';
}

/** `3800` -> `3,8 s`. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return '—';
  return `${(Math.round(ms / 100) / 10).toFixed(1).replace('.', ',')} s`;
}

/** Like `formatCents` but with an explicit `+` for positive amounts: `+1.234,56 €`. */
export function formatSignedCents(cents: number, currency = 'EUR'): string {
  return `${cents > 0 ? '+' : ''}${formatCents(cents, currency)}`;
}

/** `23.4` -> `23,4 %` (absolute value; the caller adds sign or arrow). */
export function formatPercent(value: number, digits = 1): string {
  return `${Math.abs(value).toFixed(digits).replace('.', ',')} %`;
}

/** `14.7` -> `14,7 p. p.` (percentage points, absolute value). */
export function formatPoints(value: number, digits = 1): string {
  return `${Math.abs(value).toFixed(digits).replace('.', ',')} p. p.`;
}

const MONTHS_LONG = [
  'enero', 'febrero', 'marzo', 'abril', 'mayo', 'junio',
  'julio', 'agosto', 'septiembre', 'octubre', 'noviembre', 'diciembre',
];
const MONTHS_SHORT = ['ene', 'feb', 'mar', 'abr', 'may', 'jun', 'jul', 'ago', 'sep', 'oct', 'nov', 'dic'];

/** `2026-11` or `2026-11-15` -> year and zero-based month. */
function ym(value: string): { year: number; month: number } {
  const [y, m] = value.split('-');
  return { year: Number(y), month: Number(m) - 1 };
}

/** `noviembre 2026` */
export function formatMonthLong(month: string): string {
  const { year, month: m } = ym(month);
  return `${MONTHS_LONG[m]} ${year}`;
}

/** `nov` */
export function formatMonthShort(month: string): string {
  return MONTHS_SHORT[ym(month).month];
}

/** `nov 2026` */
export function formatMonthYearShort(month: string): string {
  const { year, month: m } = ym(month);
  return `${MONTHS_SHORT[m]} ${year}`;
}

/** `2026-11-05` -> `05/11` */
export function formatDayMonth(date: string): string {
  const [, m, d] = date.split('-');
  return `${d}/${m}`;
}

/** `YYYY-MM` of a date or ISO date string, in local time for Dates. */
export function monthOf(value: Date | string): string {
  if (typeof value === 'string') return value.slice(0, 7);
  return `${value.getFullYear()}-${pad(value.getMonth() + 1)}`;
}

/** `YYYY-MM-DD` in local time. */
export function isoDate(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** `shiftMonth('2026-01', -1)` -> `2025-12` */
export function shiftMonth(month: string, delta: number): string {
  const { year, month: m } = ym(month);
  const index = year * 12 + m + delta;
  return `${Math.floor(index / 12)}-${pad((index % 12) + 1)}`;
}

/** First and last day (`YYYY-MM-DD`) of a `YYYY-MM` month. */
export function monthRange(month: string): { from: string; to: string } {
  const { year, month: m } = ym(month);
  const last = new Date(year, m + 1, 0).getDate();
  return { from: `${month}-01`, to: `${month}-${pad(last)}` };
}

/**
 * Label of a comparison period: `1–15 oct` for a partial month,
 * `oct 2026` when the range covers the whole month.
 */
export function formatPeriodLabel(range: { from: string; to: string }): string {
  const month = monthOf(range.from);
  const full = monthRange(month);
  if (range.from === full.from && range.to === full.to) return formatMonthYearShort(month);
  return `${Number(range.from.slice(8))}–${Number(range.to.slice(8))} ${formatMonthShort(month)}`;
}

/** `hace 2 h` style relative time (minutes, hours, days) for a timestamp. */
export function formatRelative(value: string | Date, now: Date): string {
  const d = toDate(value);
  if (!d) return '—';
  const minutes = Math.max(0, Math.round((now.getTime() - d.getTime()) / 60000));
  if (minutes < 1) return 'ahora mismo';
  if (minutes < 60) return `hace ${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `hace ${hours} h`;
  return `hace ${Math.floor(hours / 24)} d`;
}

/**
 * Parses an amount typed in es-ES style into integer cents: `150`, `150,5`, `1.234,56`, `1234.56`, `150 €`.
 * Returns null for anything that is not a plain non-negative amount with at most two decimals.
 */
export function parseEuros(text: string): number | null {
  const s = text.replace(/[\s€]/g, '');
  let whole: string;
  let frac = '';
  if (s.includes(',')) {
    const parts = s.split(',');
    if (parts.length !== 2) return null;
    whole = parts[0].replace(/\./g, '');
    frac = parts[1];
    // Dots before a comma must be proper thousands separators.
    if (parts[0].includes('.') && !/^\d{1,3}(\.\d{3})+$/.test(parts[0])) return null;
  } else if (/^\d{1,3}(\.\d{3})+$/.test(s)) {
    whole = s.replace(/\./g, '');
  } else if (/^\d*\.\d{1,2}$/.test(s)) {
    [whole, frac] = s.split('.');
  } else {
    whole = s;
  }
  if (!/^\d+$/.test(whole || '0') || !/^\d{0,2}$/.test(frac)) return null;
  if (!whole && !frac) return null;
  return Number(whole || '0') * 100 + Number(frac.padEnd(2, '0') || '0');
}

/** `15000` -> `150,00` (input value, no symbol or thousands separator). */
export function formatEurosInput(cents: number): string {
  const abs = Math.abs(Math.trunc(cents));
  return `${Math.floor(abs / 100)},${pad(abs % 100)}`;
}

/** Basis points as a percentage with one decimal: `7810` -> `78,1 %`. */
export function formatBp(bp: number): string {
  return formatPercent(bp / 100);
}
