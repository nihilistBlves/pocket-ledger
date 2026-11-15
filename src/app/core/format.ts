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
  const d = value instanceof Date ? value : new Date(value);
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
