import { Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { CommandError } from '../bank/bank.models';
import { BankService, toCommandError } from '../bank/bank.service';
import { formatCents, formatDate, isoDate } from '../core/format';
import { STRINGS } from '../core/strings';
import { NOW } from '../ledger/clock';
import { categoryColorClass } from '../ledger/colors';
import {
  CategoryOption,
  RuleDirection,
  RuleField,
  RuleInput,
  RuleMatchType,
  RuleRow,
  RuleTest,
} from '../ledger/ledger.models';
import { LedgerService } from '../ledger/ledger.service';
import { Modal } from '../ledger/modal';

export type Origin = 'all' | 'manual' | 'learned';
export type Level = 'high' | 'normal' | 'low';

const LEVEL_VALUE: Record<Level, number> = { high: 10, normal: 0, low: -10 };
const MAX_PATTERN = 100;
const SEARCH_FOLD = /[̀-ͯ]/g;

export function levelOf(priority: number): Level {
  return priority > 0 ? 'high' : priority < 0 ? 'low' : 'normal';
}

/** Date `months` before `today` (`YYYY-MM-DD`), clamping the day to the target month's length. */
export function monthsBefore(today: string, months: number): string {
  const [y, m, d] = today.split('-').map(Number);
  const target = new Date(y, m - 1 - months, 1);
  const last = new Date(target.getFullYear(), target.getMonth() + 1, 0).getDate();
  return isoDate(new Date(target.getFullYear(), target.getMonth(), Math.min(d, last)));
}

/** A rule that has never hit, or whose last hit is older than six months. */
export function isUnused(rule: Pick<RuleRow, 'lastHitAt'>, today: string): boolean {
  return rule.lastHitAt === null || rule.lastHitAt.slice(0, 10) < monthsBefore(today, 6);
}

const fold = (s: string): string => s.normalize('NFD').replace(SEARCH_FOLD, '').toLowerCase();

interface Draft {
  field: RuleField;
  matchType: RuleMatchType;
  pattern: string;
  accountId: number | null;
  direction: RuleDirection;
  categoryId: number | null;
  level: Level;
}

const EMPTY: Draft = {
  field: 'any',
  matchType: 'contains',
  pattern: '',
  accountId: null,
  direction: 'any',
  categoryId: null,
  level: 'normal',
};

@Component({
  selector: 'app-rules',
  imports: [Modal],
  templateUrl: './rules.html',
  styleUrl: './rules.css',
})
export class Rules {
  protected readonly t = STRINGS.rules;
  protected readonly c = STRINGS.common;
  protected readonly l = STRINGS.ledger;
  protected readonly maxPattern = MAX_PATTERN;
  protected readonly bank = inject(BankService);
  private readonly ledger = inject(LedgerService);
  private readonly now = inject(NOW);
  protected readonly fmt = { cents: formatCents, date: formatDate };

  protected readonly rules = signal<RuleRow[]>([]);
  protected readonly categories = signal<CategoryOption[]>([]);
  protected readonly loading = signal(true);
  protected readonly error = signal<CommandError | null>(null);
  protected readonly status = signal<{ kind: 'ok' | 'error'; text: string } | null>(null);
  protected readonly busy = signal(false);
  private requestId = 0;

  // Filters
  protected readonly q = signal('');
  protected readonly origin = signal<Origin>('all');
  protected readonly categoryFilter = signal('');
  protected readonly unusedOnly = signal(false);

  // Editor: null = closed; id null = new rule
  protected readonly editingId = signal<number | null | undefined>(undefined);
  protected readonly draft = signal<Draft>({ ...EMPTY });
  private originalPriority = 0;
  protected readonly applyOnSave = signal(true);
  protected readonly testing = signal(false);
  private readonly testState = signal<{ key: string; result: RuleTest } | null>(null);
  protected readonly deleting = signal<RuleRow | null>(null);

  protected readonly today = computed(() => isoDate(this.now()));
  protected readonly accounts = computed(() => this.bank.overview()?.accounts ?? []);
  protected readonly counts = computed(() => {
    const all = this.rules();
    return { all: all.length, manual: all.filter((r) => r.origin === 'manual').length, learned: all.filter((r) => r.origin === 'learned').length };
  });
  protected readonly filtered = computed(() => {
    const q = fold(this.q().trim());
    const origin = this.origin();
    const cat = this.categoryFilter();
    const unused = this.unusedOnly();
    const today = this.today();
    return this.rules().filter(
      (r) =>
        (origin === 'all' || r.origin === origin) &&
        (!cat || r.categoryId === Number(cat)) &&
        (!unused || isUnused(r, today)) &&
        (!q || fold(r.pattern).includes(q) || fold(r.categoryName).includes(q)),
    );
  });
  protected readonly hasFilters = computed(() => !!(this.q().trim() || this.origin() !== 'all' || this.categoryFilter() || this.unusedOnly()));
  protected readonly editing = computed(() => {
    const id = this.editingId();
    return typeof id === 'number' ? (this.rules().find((r) => r.id === id) ?? null) : null;
  });
  protected readonly isOpen = computed(() => this.editingId() !== undefined);
  /** Categories a rule may point to: not transfer or system. */
  protected readonly targetOptions = computed(() => this.categories().filter((x) => x.kind === 'income' || x.kind === 'expense'));

  private readonly draftKey = computed(() => JSON.stringify([this.editingId(), this.draft()]));
  protected readonly test = computed(() => {
    const s = this.testState();
    return s && s.key === this.draftKey() ? s.result : null;
  });
  protected readonly targetName = computed(() => this.categories().find((x) => x.id === this.draft().categoryId)?.name ?? '');
  protected readonly canSave = computed(() => this.draft().pattern.trim().length > 0 && this.draft().categoryId !== null);

  constructor() {
    void this.ledger.start();
    void this.bank.start();
    void this.loadCategories();
    effect(() => {
      this.ledger.syncTick();
      untracked(() => void this.load());
    });
  }

  private async loadCategories(): Promise<void> {
    try {
      this.categories.set((await this.ledger.categories()) ?? []);
    } catch {
      // The page-level error covers real failures.
    }
  }

  protected async load(): Promise<void> {
    const id = ++this.requestId;
    this.loading.set(true);
    this.error.set(null);
    try {
      const rules = await this.ledger.rules();
      if (id === this.requestId) this.rules.set(rules ?? []);
    } catch (e) {
      if (id === this.requestId) this.error.set(toCommandError(e));
    } finally {
      if (id === this.requestId) this.loading.set(false);
    }
  }

  protected retry(): void {
    void this.load();
  }

  // Display helpers

  protected colorOf(r: RuleRow): string {
    return categoryColorClass(this.categories(), r.categoryId, null);
  }

  protected scope(r: RuleRow): string {
    const parts: string[] = [];
    if (r.field === 'description') parts.push(this.t.scopeDescription);
    else if (r.field === 'counterparty') parts.push(this.t.scopeCounterparty);
    if (r.direction !== 'any') {
      const out = r.direction === 'out';
      parts.push(parts.length ? (out ? this.t.dirOut : this.t.dirIn) : out ? this.t.onlyDirOut : this.t.onlyDirIn);
    }
    if (r.accountName) parts.push(`${this.t.account} ${r.accountName}`);
    return parts.join(' · ');
  }

  protected priorityLabel(p: number): string {
    return p > 0 ? this.t.priorityHigh : p < 0 ? this.t.priorityLow : this.t.priorityNormal;
  }

  protected optionLabel(c: CategoryOption): string {
    return c.parentId === null ? c.name : `  ${c.name}`;
  }

  protected rowKey(event: KeyboardEvent, r: RuleRow): void {
    const el = event.currentTarget as HTMLElement;
    if (event.target !== el) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      ((event.key === 'ArrowDown' ? el.nextElementSibling : el.previousElementSibling) as HTMLElement | null)?.focus();
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      this.edit(r);
    }
  }

  protected clearFilters(): void {
    this.q.set('');
    this.origin.set('all');
    this.categoryFilter.set('');
    this.unusedOnly.set(false);
  }

  // Editor

  protected edit(r: RuleRow): void {
    this.editingId.set(r.id);
    this.originalPriority = r.priority;
    this.draft.set({
      field: r.field,
      matchType: r.matchType,
      pattern: r.pattern,
      accountId: r.accountId,
      direction: r.direction,
      categoryId: r.categoryId,
      level: levelOf(r.priority),
    });
    this.applyOnSave.set(true);
    this.status.set(null);
  }

  protected openNew(): void {
    this.editingId.set(null);
    this.originalPriority = 0;
    this.draft.set({ ...EMPTY });
    this.applyOnSave.set(true);
    this.status.set(null);
  }

  protected closeEditor(): void {
    this.editingId.set(undefined);
  }

  protected patch(p: Partial<Draft>): void {
    this.draft.update((d) => ({ ...d, ...p }));
  }

  private toInput(): RuleInput {
    const d = this.draft();
    return {
      pattern: d.pattern.trim(),
      field: d.field,
      matchType: d.matchType,
      accountId: d.accountId,
      direction: d.direction,
      categoryId: d.categoryId!,
      // Keep an unusual stored priority unless the user picked another level.
      priority: levelOf(this.originalPriority) === d.level ? this.originalPriority : LEVEL_VALUE[d.level],
    };
  }

  protected async runTest(): Promise<void> {
    if (!this.canSave() || this.testing()) return;
    const key = this.draftKey();
    this.testing.set(true);
    try {
      const result = await this.ledger.testRule(this.editingId() ?? null, this.toInput(), this.today());
      this.testState.set({ key, result });
    } catch (e) {
      this.status.set({ kind: 'error', text: this.errorText(e, this.t.errors.testConflict) });
    } finally {
      this.testing.set(false);
    }
  }

  protected async save(): Promise<void> {
    if (this.busy()) return;
    const d = this.draft();
    if (!d.pattern.trim() || d.pattern.trim().length > MAX_PATTERN) {
      this.status.set({ kind: 'error', text: this.t.errors.patternEmpty });
      return;
    }
    if (d.categoryId === null) {
      this.status.set({ kind: 'error', text: this.t.errors.categoryMissing });
      return;
    }
    this.busy.set(true);
    this.status.set(null);
    try {
      const result = await this.ledger.saveRule(this.editingId() ?? null, this.toInput(), this.applyOnSave(), this.today());
      await this.load();
      this.status.set({ kind: 'ok', text: this.t.savedRule(result.recategorized) });
      const saved = this.rules().find((r) => r.id === result.rule.id) ?? result.rule;
      this.edit(saved);
      this.status.set({ kind: 'ok', text: this.t.savedRule(result.recategorized) });
    } catch (e) {
      this.status.set({ kind: 'error', text: this.errorText(e, this.t.errors.saveConflict) });
    } finally {
      this.busy.set(false);
    }
  }

  private errorText(e: unknown, conflict: string): string {
    const err = toCommandError(e);
    return err.code === 'conflict' ? conflict : `${this.c.saveError} ${err.message}`;
  }

  // Delete

  protected askDelete(r: RuleRow): void {
    this.deleting.set(r);
  }

  protected closeDelete(): void {
    this.deleting.set(null);
  }

  protected async confirmDelete(): Promise<void> {
    const r = this.deleting();
    if (!r || this.busy()) return;
    this.busy.set(true);
    try {
      await this.ledger.deleteRule(r.id);
      this.deleting.set(null);
      if (this.editingId() === r.id) this.editingId.set(undefined);
      await this.load();
      this.status.set({ kind: 'ok', text: this.t.deletedRule });
    } catch (e) {
      this.deleting.set(null);
      this.status.set({ kind: 'error', text: this.errorText(e, this.t.errors.deleteConflict) });
    } finally {
      this.busy.set(false);
    }
  }

  // Apply to uncategorized

  protected async applyUncategorized(): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.status.set(null);
    try {
      const res = await this.ledger.applyRulesUncategorized();
      await this.load();
      this.status.set({ kind: 'ok', text: this.t.applied(res?.recategorized ?? 0) });
    } catch (e) {
      this.status.set({ kind: 'error', text: this.errorText(e, this.t.errors.applyConflict) });
    } finally {
      this.busy.set(false);
    }
  }

  // Test rendering

  protected needsReview(categoryId: number | null): boolean {
    if (categoryId === null) return false;
    const cat = this.categories().find((x) => x.id === categoryId);
    return !!cat && cat.key !== 'uncategorized' && !cat.key?.startsWith('uncategorized');
  }
}
