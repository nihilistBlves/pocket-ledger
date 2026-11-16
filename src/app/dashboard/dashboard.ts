import { Component, ElementRef, computed, effect, inject, signal, untracked, viewChild } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { CommandError } from '../bank/bank.models';
import { BankService, toCommandError } from '../bank/bank.service';
import {
  formatCents,
  formatDate,
  formatRelative,
  formatDayMonth,
  formatMonthLong,
  formatMonthShort,
  formatMonthYearShort,
  formatPercent,
  formatPeriodLabel,
  formatPoints,
  formatSignedCents,
  isoDate,
  monthOf,
} from '../core/format';
import { STRINGS } from '../core/strings';
import { NOW } from '../ledger/clock';
import { Dashboard as DashboardData, PeriodTotals, TxRow } from '../ledger/ledger.models';
import { LedgerService } from '../ledger/ledger.service';
import { MonthSelector } from '../ledger/month-selector';
import { buildChart, formatAxis } from './evolution-chart';

export interface Delta {
  arrow: '▲' | '▼' | '';
  /** Screen-reader word for the direction. */
  word: string;
  cls: 'good' | 'bad' | '';
  text: string;
}

function toDelta(change: number, goodWhenUp: boolean, fmt: (v: number) => string): Delta {
  const text = fmt(change);
  if (Math.abs(change) < 0.05) return { arrow: '', word: '', cls: '', text };
  const up = change > 0;
  return {
    arrow: up ? '▲' : '▼',
    word: up ? STRINGS.dashboard.rises : STRINGS.dashboard.falls,
    cls: up === goodWhenUp ? 'good' : 'bad',
    text,
  };
}

/**
 * Change from `prev` to `cur` as a percentage. `goodWhenUp` is false for expenses (going down is good).
 * Null when there is nothing to compare against (previous value of zero).
 */
export function percentDelta(cur: number, prev: number, goodWhenUp: boolean): Delta | null {
  if (prev === 0) return null;
  return toDelta(((cur - prev) / Math.abs(prev)) * 100, goodWhenUp, formatPercent);
}

/** Change in percentage points (savings rate). */
export function pointsDelta(cur: number, prev: number): Delta {
  return toDelta(cur - prev, true, formatPoints);
}

/** Savings rate in percent, or null without income. */
export function savingsRate(t: PeriodTotals): number | null {
  return t.incomeCents > 0 ? (t.balanceCents / t.incomeCents) * 100 : null;
}

interface Kpi {
  key: string;
  label: string;
  value: string;
  delta: Delta | null;
}

interface CatRow {
  name: string;
  count: number | null;
  amount: string;
  pct: string;
  width: number;
  cls: string;
}

@Component({
  selector: 'app-dashboard',
  imports: [RouterLink, MonthSelector],
  templateUrl: './dashboard.html',
  styleUrl: './dashboard.css',
})
export class Dashboard {
  protected readonly t = STRINGS.dashboard;
  protected readonly l = STRINGS.ledger;
  protected readonly bank = inject(BankService);
  private readonly ledger = inject(LedgerService);
  private readonly router = inject(Router);
  private readonly now = inject(NOW);
  private readonly chartBox = viewChild<ElementRef<HTMLElement>>('chartBox');
  private readonly chartWidth = signal(760);
  protected readonly fmt = {
    cents: formatCents,
    signed: formatSignedCents,
    axis: formatAxis,
    date: formatDate,
    dayMonth: formatDayMonth,
  };

  protected readonly month = signal(monthOf(this.now()));
  protected readonly data = signal<DashboardData | null>(null);
  protected readonly loading = signal(true);
  protected readonly error = signal<CommandError | null>(null);
  protected readonly query = signal('');
  private requestId = 0;

  protected readonly monthLabel = computed(() => formatMonthLong(this.month()));
  protected readonly monthShort = computed(() => formatMonthYearShort(this.month()));
  protected readonly canSync = computed(
    () => this.bank.overview()?.connection?.kind === 'connected' && this.bank.busy() === null,
  );

  protected readonly vsLabel = computed(() => {
    const d = this.data();
    return d ? this.t.vs(formatPeriodLabel(d.previousLabelRange)) : '';
  });

  protected readonly kpis = computed<Kpi[]>(() => {
    const d = this.data();
    if (!d) return [];
    const { totals: c, previous: p } = d;
    const rate = savingsRate(c);
    const prevRate = savingsRate(p);
    return [
      { key: 'income', label: this.l.income, value: formatCents(c.incomeCents), delta: percentDelta(c.incomeCents, p.incomeCents, true) },
      { key: 'expense', label: this.l.expense, value: formatCents(c.expenseCents), delta: percentDelta(c.expenseCents, p.expenseCents, false) },
      { key: 'balance', label: this.l.balance, value: formatSignedCents(c.balanceCents), delta: percentDelta(c.balanceCents, p.balanceCents, true) },
      {
        key: 'savings',
        label: this.t.savingsRate,
        value: rate === null ? '—' : formatPercent(rate),
        delta: rate !== null && prevRate !== null ? pointsDelta(rate, prevRate) : null,
      },
    ];
  });

  protected readonly evolution = computed(() => {
    const d = this.data();
    return d ? buildChart(d.evolution, this.chartWidth()) : null;
  });

  protected readonly evolutionCaption = computed(() => {
    const e = this.data()?.evolution ?? [];
    return e.length
      ? this.t.evolutionCaption(formatMonthYearShort(e[0].month), formatMonthYearShort(e[e.length - 1].month))
      : '';
  });

  protected readonly partialNote = computed(() => {
    const d = this.data();
    if (!d?.isCurrentMonth || d.throughDay === null) return '';
    const name = formatMonthLong(d.month).split(' ')[0];
    return this.t.partialNote(name.charAt(0).toUpperCase() + name.slice(1), d.throughDay);
  });

  protected readonly chartDesc = computed(() => {
    const d = this.data();
    return d ? this.t.evolutionChartDesc(d.isCurrentMonth ? d.throughDay : null, formatMonthLong(d.month)) : '';
  });

  protected readonly catRows = computed<CatRow[]>(() => {
    const cats = this.data()?.categories ?? [];
    const total = cats.reduce((s, c) => s + c.expenseCents, 0);
    if (total <= 0) return [];
    const max = cats[0].expenseCents;
    const row = (name: string, cents: number, count: number | null, cls: string): CatRow => ({
      name,
      count,
      amount: formatCents(cents),
      pct: formatPercent((cents / total) * 100),
      width: Math.min(100, (cents / max) * 100),
      cls,
    });
    const rows = cats
      .slice(0, 6)
      .map((c, i) => row(c.categoryId === null ? this.l.uncategorized : c.name, c.expenseCents, null, `c${i + 1}`));
    const rest = cats.slice(6);
    if (rest.length) {
      rows.push(row(this.t.others, rest.reduce((s, c) => s + c.expenseCents, 0), rest.length, ''));
    }
    return rows;
  });

  protected readonly accountsTotal = computed(() =>
    (this.data()?.accounts ?? []).reduce((s, a) => s + (a.balanceCents ?? 0), 0), // null balances are excluded
  );

  protected readonly isEmptyAccounts = computed(() => {
    const d = this.data();
    return !!d && d.accounts.length === 0;
  });
  protected readonly isEmptyMonth = computed(() => {
    const d = this.data();
    return !!d && d.accounts.length > 0 && d.monthTxCount === 0;
  });

  constructor() {
    void this.ledger.start();
    void this.bank.start();
    effect((onCleanup) => {
      const box = this.chartBox()?.nativeElement;
      if (!box || typeof ResizeObserver === 'undefined') return;
      const ro = new ResizeObserver((entries) => {
        const w = Math.round(entries[0]?.contentRect.width ?? 0);
        if (w > 0) this.chartWidth.set(w);
      });
      ro.observe(box);
      onCleanup(() => ro.disconnect());
    });
    effect(() => {
      const month = this.month();
      this.ledger.syncTick();
      untracked(() => void this.load(month));
    });
  }

  protected async load(month: string = this.month()): Promise<void> {
    const id = ++this.requestId;
    this.loading.set(true);
    this.error.set(null);
    try {
      const data = await this.ledger.dashboard(month, isoDate(this.now()));
      if (id === this.requestId) this.data.set(data);
    } catch (e) {
      if (id === this.requestId) this.error.set(toCommandError(e));
    } finally {
      if (id === this.requestId) this.loading.set(false);
    }
  }

  protected setMonth(month: string): void {
    this.month.set(month);
  }

  protected retry(): void {
    void this.load();
  }

  protected submitSearch(event: Event): void {
    event.preventDefault();
    const q = this.query().trim();
    void this.router.navigate(['/movimientos'], { queryParams: q ? { q } : {} });
  }

  protected sync(): void {
    void this.bank.syncNow();
  }

  protected isCurrent(month: string): boolean {
    const d = this.data();
    return !!d && d.isCurrentMonth && d.month === month;
  }

  protected axisLabel(month: string): string {
    return formatMonthShort(month) + (this.isCurrent(month) ? '*' : '');
  }

  protected rowCategory(row: TxRow): string {
    if (row.isTransfer) return this.l.transfer;
    return row.categoryName ?? this.l.uncategorized;
  }

  protected rowClass(row: TxRow): string {
    const name = row.parentCategoryName ?? row.categoryName;
    const idx = (this.data()?.categories ?? []).slice(0, 6).findIndex((c) => c.name === name);
    return idx >= 0 ? `c${idx + 1}` : '';
  }

  protected rowTitle(row: TxRow): string {
    return [row.description, row.counterparty].filter(Boolean).join(' · ');
  }

  protected accountSub(a: { displaySuffix: string | null; balanceCents: number | null; lastSyncedAt: string | null }): string {
    const parts: string[] = [];
    if (a.displaySuffix) parts.push(`•••• ${a.displaySuffix}`);
    if (a.lastSyncedAt) parts.push(formatRelative(a.lastSyncedAt, this.now()));
    if (a.balanceCents === null) parts.push(this.t.noBalance);
    return parts.join(' · ');
  }
}
