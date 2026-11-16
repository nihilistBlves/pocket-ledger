import { MonthTotals } from '../ledger/ledger.models';

/** Fixed plot height in px; the width follows the container. Pure geometry so it can be unit tested. */
export const CHART_HEIGHT = 260;
export const CHART_MIN_WIDTH = 320;

export interface ChartTick {
  cents: number;
  y: number;
}

export interface ChartSlot {
  month: string;
  /** Slot centre. */
  cx: number;
  slotX: number;
  slotWidth: number;
  income: { x: number; width: number; y: number; height: number; path: string } | null;
  expense: { x: number; width: number; y: number; height: number; path: string } | null;
  balanceY: number;
  data: MonthTotals;
}

export interface ChartModel {
  width: number;
  height: number;
  left: number;
  right: number;
  top: number;
  bottom: number;
  ticks: ChartTick[];
  zeroY: number;
  slots: ChartSlot[];
  linePoints: string;
}

const NICE = [1, 2, 2.5, 5, 10];

/** Round axis step (in cents) so that about `count` intervals cover `span`. */
export function niceStep(span: number, count: number): number {
  const raw = Math.max(span, 1) / count;
  const pow = 10 ** Math.floor(Math.log10(raw));
  for (const n of NICE) if (n * pow >= raw) return n * pow;
  return 10 * pow;
}

/** Bar with its top corners rounded (4px) and its base flat on the baseline. */
function barPath(x: number, width: number, top: number, base: number): string {
  const h = base - top;
  const r = Math.min(4, h / 2, width / 2);
  return (
    `M${x},${base}V${top + r}Q${x},${top} ${x + r},${top}H${x + width - r}` +
    `Q${x + width},${top} ${x + width},${top + r}V${base}Z`
  );
}

/** Builds the chart geometry. Negative balances extend the domain below the zero line. */
export function buildChart(months: readonly MonthTotals[], width = 760, height = CHART_HEIGHT): ChartModel {
  width = Math.max(width, CHART_MIN_WIDTH);
  const top = 16;
  const bottom = height - 28;
  const left = 56;
  const right = width - 8;
  const values = months.flatMap((m) => [m.incomeCents, m.expenseCents, m.balanceCents]);
  const rawMax = Math.max(0, ...values);
  const rawMin = Math.min(0, ...values);
  const step = niceStep(rawMax - rawMin, 4);
  const max = Math.ceil(rawMax / step) * step || step;
  const min = Math.floor(rawMin / step) * step;
  const y = (cents: number) => bottom - ((cents - min) / (max - min)) * (bottom - top);

  const ticks: ChartTick[] = [];
  for (let c = min; c <= max; c += step) ticks.push({ cents: c, y: y(c) });

  const slotWidth = (right - left) / Math.max(months.length, 1);
  const barWidth = Math.max(4, Math.min(18, (slotWidth - 8) / 2 - 1));
  const zeroY = y(0);
  const bar = (x: number, cents: number) => {
    if (cents <= 0) return null;
    const yTop = y(cents);
    const height = zeroY - yTop;
    return { x, width: barWidth, y: yTop, height, path: barPath(x, barWidth, yTop, zeroY) };
  };

  const slots = months.map((m, i): ChartSlot => {
    const slotX = left + i * slotWidth;
    const cx = slotX + slotWidth / 2;
    return {
      month: m.month,
      cx,
      slotX,
      slotWidth,
      income: bar(cx - barWidth - 1, m.incomeCents),
      expense: bar(cx + 1, m.expenseCents),
      balanceY: y(m.balanceCents),
      data: m,
    };
  });

  return {
    width, height, left, right, top, bottom,
    ticks,
    zeroY,
    slots,
    linePoints: slots.map((s) => `${s.cx.toFixed(1)},${s.balanceY.toFixed(1)}`).join(' '),
  };
}

/** `1500000` -> `15 k €`, `250000` -> `2,5 k €`, `50000` -> `500 €` (axis labels). */
export function formatAxis(cents: number): string {
  const euros = cents / 100;
  const abs = Math.abs(euros);
  const sign = euros < 0 ? '−' : '';
  if (abs >= 1000) return `${sign}${(abs / 1000).toString().replace('.', ',')} k €`;
  return `${sign}${abs.toString().replace('.', ',')} €`;
}
