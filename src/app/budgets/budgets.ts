import { Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { CommandError } from '../bank/bank.models';
import { toCommandError } from '../bank/bank.service';
import {
  formatBp,
  formatCents,
  formatEurosInput,
  formatMonthLong,
  formatPercent,
  isoDate,
  monthOf,
  parseEuros,
  shiftMonth,
} from '../core/format';
import { STRINGS } from '../core/strings';
import { NOW } from '../ledger/clock';
import { colorClass } from '../ledger/colors';
import { BudgetMonth, BudgetRow, CategoryOption } from '../ledger/ledger.models';
import { LedgerService } from '../ledger/ledger.service';
import { Modal } from '../ledger/modal';
import { MonthSelector } from '../ledger/month-selector';

export const MAX_BUDGET_CENTS = 100_000_000;

interface Dialog {
  /** Null while a category still has to be picked (add mode). */
  categoryId: number | null;
  /** Existing budget row (edit mode). */
  row: BudgetRow | null;
  avg3Cents: number | null;
}

@Component({
  selector: 'app-budgets',
  imports: [MonthSelector, Modal],
  templateUrl: './budgets.html',
  styleUrl: './budgets.css',
})
export class Budgets {
  protected readonly t = STRINGS.budgets;
  protected readonly c = STRINGS.common;
  protected readonly l = STRINGS.ledger;
  private readonly ledger = inject(LedgerService);
  private readonly now = inject(NOW);
  protected readonly fmt = { cents: formatCents, bp: formatBp, colorClass };

  protected readonly month = signal(monthOf(this.now()));
  protected readonly data = signal<BudgetMonth | null>(null);
  protected readonly categories = signal<CategoryOption[]>([]);
  protected readonly loading = signal(true);
  protected readonly error = signal<CommandError | null>(null);
  protected readonly status = signal<{ kind: 'ok' | 'error'; text: string } | null>(null);
  protected readonly saving = signal(false);
  private requestId = 0;

  // Dialog
  protected readonly dialog = signal<Dialog | null>(null);
  protected readonly pickedCategory = signal<number | null>(null);
  protected readonly amountText = signal('');
  protected readonly fromNext = signal(false);
  protected readonly dialogError = signal<string | null>(null);

  protected readonly monthLabel = computed(() => formatMonthLong(this.month()));
  protected readonly nextMonthLabel = computed(() => formatMonthLong(shiftMonth(this.month(), 1)));
  protected readonly fromMonth = computed(() => (this.fromNext() ? shiftMonth(this.month(), 1) : this.month()));
  protected readonly isCurrent = computed(() => this.month() === monthOf(this.now()));

  protected readonly kpis = computed(() => {
    const d = this.data();
    if (!d) return null;
    const tot = d.totals;
    const pct = tot.budgetCents > 0 ? (tot.spentCents / tot.budgetCents) * 100 : 0;
    const daysLeft = d.isCurrentMonth && d.dayOfMonth !== null ? d.daysInMonth - d.dayOfMonth : 0;
    const perDay = tot.remainingCents > 0 && daysLeft > 0 ? Math.round(tot.remainingCents / daysLeft) : null;
    return { tot, pct: formatPercent(pct), perDay, day: d.isCurrentMonth ? d.dayOfMonth : null };
  });

  protected readonly pace = computed(() => {
    const d = this.data();
    return d?.isCurrentMonth && d.dayOfMonth !== null ? (d.dayOfMonth / d.daysInMonth) * 100 : null;
  });

  /** Expense categories that can receive a budget (the backend refuses overlaps with `conflict`). */
  protected readonly expenseOptions = computed(() => {
    const budgeted = new Set(this.data()?.budgets.map((b) => b.categoryId) ?? []);
    return this.categories().filter((x) => x.kind === 'expense' && !budgeted.has(x.id));
  });

  protected readonly dialogCategory = computed(() => {
    const d = this.dialog();
    const id = d?.row?.categoryId ?? d?.categoryId ?? this.pickedCategory();
    return id === null || id === undefined ? null : (this.categories().find((x) => x.id === id) ?? null);
  });

  protected readonly dialogName = computed(() => {
    const d = this.dialog();
    return d?.row?.name ?? this.dialogCategory()?.name ?? '';
  });

  protected readonly dialogNote = computed(() => {
    const d = this.dialog();
    const row = d?.row;
    if (!row) return '';
    return row.isGroup || row.parentName === null ? this.t.dialogNoteGroup : this.t.dialogNoteSub(row.parentName);
  });

  protected readonly previousHint = computed(() => {
    const row = this.dialog()?.row;
    if (!row) return '';
    const amount = this.fromMonth() > row.startMonth ? row.amountCents : row.previousAmountCents;
    return amount === null ? this.t.previousNone : this.t.previousHint(formatCents(amount));
  });

  constructor() {
    void this.ledger.start();
    void this.loadCategories();
    effect(() => {
      const month = this.month();
      this.ledger.syncTick();
      untracked(() => void this.load(month));
    });
  }

  private async loadCategories(): Promise<void> {
    try {
      this.categories.set((await this.ledger.categories()) ?? []);
    } catch {
      // Only the add dialog needs the list.
    }
  }

  protected async load(month: string = this.month()): Promise<void> {
    const id = ++this.requestId;
    this.loading.set(true);
    this.error.set(null);
    try {
      const data = await this.ledger.budgets(month, isoDate(this.now()));
      if (id === this.requestId) this.data.set(data);
    } catch (e) {
      if (id === this.requestId) this.error.set(toCommandError(e));
    } finally {
      if (id === this.requestId) this.loading.set(false);
    }
  }

  protected retry(): void {
    void this.load();
  }

  protected setMonth(month: string): void {
    this.status.set(null);
    this.month.set(month);
  }

  // Row helpers

  protected fillWidth(r: BudgetRow): number {
    return Math.max(0, Math.min(100, r.percentBp / 100));
  }

  protected pctValue(r: BudgetRow): number {
    return Math.round(r.percentBp / 10) / 10;
  }

  protected rowLabel(r: BudgetRow): string {
    return this.t.progressLabel(r.name, this.fmt.bp(r.percentBp));
  }

  // Dialog

  protected openEdit(row: BudgetRow): void {
    this.dialog.set({ categoryId: row.categoryId, row, avg3Cents: row.avg3Cents });
    this.amountText.set(formatEurosInput(row.amountCents));
    this.fromNext.set(false);
    this.dialogError.set(null);
  }

  protected openAdd(categoryId: number | null = null, avg3Cents: number | null = null): void {
    this.dialog.set({ categoryId, row: null, avg3Cents });
    this.pickedCategory.set(categoryId);
    this.amountText.set(avg3Cents && avg3Cents > 0 ? formatEurosInput(avg3Cents) : '');
    this.fromNext.set(false);
    this.dialogError.set(null);
  }

  protected closeDialog(): void {
    this.dialog.set(null);
  }

  protected pickCategory(value: string): void {
    this.pickedCategory.set(value ? Number(value) : null);
  }

  private targetId(): number | null {
    const d = this.dialog();
    return d?.row?.categoryId ?? d?.categoryId ?? this.pickedCategory();
  }

  protected async saveBudget(): Promise<void> {
    const id = this.targetId();
    if (id === null) {
      this.dialogError.set(this.t.errors.pickCategory);
      return;
    }
    const cents = parseEuros(this.amountText());
    if (cents === null || cents < 1) {
      this.dialogError.set(this.t.errors.amountInvalid);
      return;
    }
    if (cents > MAX_BUDGET_CENTS) {
      this.dialogError.set(this.t.errors.amountTooBig);
      return;
    }
    if (this.saving()) return;
    this.saving.set(true);
    this.dialogError.set(null);
    try {
      await this.ledger.setBudget(id, this.fromMonth(), cents);
      this.dialog.set(null);
      await this.load();
      this.status.set({ kind: 'ok', text: this.t.saved });
    } catch (e) {
      const err = toCommandError(e);
      this.dialogError.set(err.code === 'conflict' ? this.t.errors.conflict : err.message);
    } finally {
      this.saving.set(false);
    }
  }

  protected async removeBudget(): Promise<void> {
    const row = this.dialog()?.row;
    if (!row || this.saving()) return;
    this.saving.set(true);
    this.dialogError.set(null);
    try {
      await this.ledger.removeBudget(row.categoryId, this.fromMonth());
      this.dialog.set(null);
      await this.load();
      this.status.set({ kind: 'ok', text: this.t.removed });
    } catch (e) {
      const err = toCommandError(e);
      this.dialogError.set(err.code === 'conflict' ? this.t.errors.conflictRemove : err.message);
    } finally {
      this.saving.set(false);
    }
  }
}
