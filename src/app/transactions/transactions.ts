import { Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { ActivatedRoute } from '@angular/router';
import { BankService, toCommandError } from '../bank/bank.service';
import { CommandError } from '../bank/bank.models';
import { formatCents, formatDate, formatSignedCents, monthOf, monthRange } from '../core/format';
import { STRINGS } from '../core/strings';
import { NOW } from '../ledger/clock';
import { CategoryOption, TxFilter, TxRow, TxSummary } from '../ledger/ledger.models';
import { categoryColorClass } from '../ledger/colors';
import { LedgerService } from '../ledger/ledger.service';
import { MonthSelector } from '../ledger/month-selector';

export const PAGE_SIZE = 100;
const MAX_TAGS = 20;
const MAX_TAG_LENGTH = 40;
const SEARCH_DEBOUNCE_MS = 250;

export type Direction = 'all' | 'in' | 'out';

/** Builds the backend filter from the screen state. A date range overrides the month. */
export function buildFilter(s: {
  month: string | null;
  dateFrom: string;
  dateTo: string;
  q: string;
  accountId: number | null;
  category: string;
  direction: Direction;
  pendingOnly: boolean;
}): TxFilter {
  const f: TxFilter = {};
  if (s.dateFrom || s.dateTo) {
    if (s.dateFrom) f.from = s.dateFrom;
    if (s.dateTo) f.to = s.dateTo;
  } else if (s.month) {
    Object.assign(f, monthRange(s.month));
  }
  const text = s.q.trim();
  if (text) f.text = text;
  if (s.accountId !== null) f.accountId = s.accountId;
  if (s.category === 'none') f.uncategorized = true;
  else if (s.category) f.categoryId = Number(s.category);
  if (s.direction !== 'all') f.direction = s.direction;
  if (s.pendingOnly) f.status = 'pending';
  return f;
}

function sameTags(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((t, i) => t === b[i]);
}

@Component({
  selector: 'app-transactions',
  imports: [MonthSelector],
  templateUrl: './transactions.html',
  styleUrl: './transactions.css',
})
export class Transactions {
  protected readonly t = STRINGS.transactions;
  protected readonly l = STRINGS.ledger;
  protected readonly bank = inject(BankService);
  private readonly ledger = inject(LedgerService);
  private readonly now = inject(NOW);
  protected readonly fmt = { cents: formatCents, signed: formatSignedCents, date: formatDate };

  // Filters
  protected readonly month = signal<string | null>(monthOf(this.now()));
  protected readonly qInput = signal('');
  protected readonly q = signal('');
  protected readonly accountId = signal<number | null>(null);
  protected readonly category = signal('');
  protected readonly dateFrom = signal('');
  protected readonly dateTo = signal('');
  protected readonly direction = signal<Direction>('all');
  protected readonly pendingOnly = signal(false);
  private debounce: ReturnType<typeof setTimeout> | null = null;

  // Data
  protected readonly rows = signal<TxRow[]>([]);
  protected readonly total = signal(0);
  protected readonly summary = signal<TxSummary | null>(null);
  protected readonly loading = signal(true);
  protected readonly loadingMore = signal(false);
  protected readonly error = signal<CommandError | null>(null);
  protected readonly categories = signal<CategoryOption[]>([]);
  protected readonly tagSuggestions = signal<string[]>([]);
  protected readonly status = signal<{ kind: 'ok' | 'error'; text: string } | null>(null);
  private requestId = 0;

  // Selection and draft of the detail panel
  protected readonly selectedId = signal<number | null>(null);
  protected readonly draftCategory = signal<number | null>(null);
  protected readonly draftTags = signal<string[]>([]);
  protected readonly draftNotes = signal('');
  protected readonly createRule = signal(false);
  protected readonly tagInput = signal('');
  protected readonly saving = signal(false);

  protected readonly accounts = computed(() => this.bank.overview()?.accounts ?? []);
  protected readonly selected = computed(() => this.rows().find((r) => r.id === this.selectedId()) ?? null);
  protected readonly groups = computed(() => this.categories().filter((c) => c.parentId === null));

  protected readonly filter = computed(() =>
    buildFilter({
      month: this.month(),
      dateFrom: this.dateFrom(),
      dateTo: this.dateTo(),
      q: this.q(),
      accountId: this.accountId(),
      category: this.category(),
      direction: this.direction(),
      pendingOnly: this.pendingOnly(),
    }),
  );

  protected readonly summaryNote = computed(() => {
    const s = this.summary();
    const parts = [this.t.count(this.total())];
    if (s && s.transferCount > 0) parts.push(this.t.transfersExcluded(s.transferCount));
    if (s && s.pendingCount > 0) parts.push(this.t.includesPending(s.pendingCount));
    return parts.join(' · ');
  });

  protected readonly categoryChanged = computed(() => {
    const row = this.selected();
    const cat = this.draftCategory();
    return !!row && !row.isTransfer && cat !== null && cat !== row.categoryId;
  });
  protected readonly ruleRequested = computed(() => {
    const row = this.selected();
    return !!row && !row.isTransfer && this.createRule() && (this.draftCategory() ?? row.categoryId) !== null;
  });
  protected readonly notesChanged = computed(() => {
    const row = this.selected();
    return !!row && this.draftNotes().trim() !== (row.notes ?? '').trim();
  });
  protected readonly tagsChanged = computed(() => {
    const row = this.selected();
    return !!row && !sameTags(this.draftTags(), row.tags);
  });
  protected readonly dirty = computed(
    () => this.categoryChanged() || this.ruleRequested() || this.notesChanged() || this.tagsChanged(),
  );

  protected readonly ruleHint = computed(() => {
    const row = this.selected();
    if (!row) return '';
    if (row.isTransfer) return this.t.ruleUnavailable;
    const catId = this.draftCategory() ?? row.categoryId;
    const name = this.categories().find((c) => c.id === catId)?.name ?? row.categoryName ?? this.l.uncategorized;
    return this.t.ruleHint(row.counterparty ?? row.description, name);
  });

  constructor() {
    void this.ledger.start();
    void this.bank.start();
    inject(ActivatedRoute)
      .queryParamMap.pipe(takeUntilDestroyed())
      .subscribe((p) => {
        const month = p.get('month');
        const q = p.get('q');
        if (month && /^\d{4}-\d{2}$/.test(month)) this.month.set(month);
        else if (q) this.month.set(null);
        if (q !== null) {
          this.qInput.set(q);
          this.q.set(q);
        }
      });
    void this.loadOptions();
    effect(() => {
      const filter = this.filter();
      this.ledger.syncTick();
      untracked(() => void this.loadFirstPage(filter));
    });
  }

  private async loadOptions(): Promise<void> {
    try {
      const [categories, tags] = await Promise.all([this.ledger.categories(), this.ledger.tags()]);
      this.categories.set(categories);
      this.tagSuggestions.set(tags);
    } catch {
      // Filters still work without the lists; the page-level error covers real failures.
    }
  }

  private async loadFirstPage(filter: TxFilter, limit = PAGE_SIZE): Promise<void> {
    const id = ++this.requestId;
    this.loading.set(true);
    this.error.set(null);
    try {
      const page = await this.ledger.transactions({ ...filter, offset: 0, limit });
      if (id !== this.requestId) return;
      this.rows.set(page.rows);
      this.total.set(page.total);
      this.summary.set(page.summary);
      if (this.selectedId() !== null && !page.rows.some((r) => r.id === this.selectedId())) {
        this.selectedId.set(null);
      }
    } catch (e) {
      if (id === this.requestId) this.error.set(toCommandError(e));
    } finally {
      if (id === this.requestId) this.loading.set(false);
    }
  }

  protected async loadMore(): Promise<void> {
    if (this.loadingMore()) return;
    const id = this.requestId;
    this.loadingMore.set(true);
    try {
      const page = await this.ledger.transactions({ ...this.filter(), offset: this.rows().length, limit: PAGE_SIZE });
      if (id !== this.requestId) return;
      this.rows.update((r) => [...r, ...page.rows]);
      this.total.set(page.total);
      this.summary.set(page.summary);
    } catch (e) {
      this.error.set(toCommandError(e));
    } finally {
      this.loadingMore.set(false);
    }
  }

  /** Reloads what is on screen (rows and summary) after a mutation. */
  private refresh(): Promise<void> {
    return this.loadFirstPage(this.filter(), Math.min(500, Math.max(PAGE_SIZE, this.rows().length)));
  }

  protected retry(): void {
    void this.loadFirstPage(this.filter());
  }

  // Filter handlers

  protected onSearch(value: string): void {
    this.qInput.set(value);
    if (this.debounce) clearTimeout(this.debounce);
    this.debounce = setTimeout(() => this.q.set(value), SEARCH_DEBOUNCE_MS);
  }

  protected onAccount(value: string): void {
    this.accountId.set(value ? Number(value) : null);
  }

  protected clearFilters(): void {
    if (this.debounce) clearTimeout(this.debounce);
    this.qInput.set('');
    this.q.set('');
    this.accountId.set(null);
    this.category.set('');
    this.dateFrom.set('');
    this.dateTo.set('');
    this.direction.set('all');
    this.pendingOnly.set(false);
  }

  protected readonly hasFilters = computed(
    () =>
      !!(this.q() || this.accountId() !== null || this.category() || this.dateFrom() || this.dateTo() || this.pendingOnly()) ||
      this.direction() !== 'all',
  );

  // Rows

  protected select(row: TxRow): void {
    this.selectedId.set(row.id);
    this.status.set(null);
    this.resetDraft(row);
  }

  protected closeDetail(): void {
    this.selectedId.set(null);
  }

  protected onRowKey(event: KeyboardEvent, row: TxRow): void {
    const el = event.currentTarget as HTMLElement;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const next = (event.key === 'ArrowDown' ? el.nextElementSibling : el.previousElementSibling) as HTMLElement | null;
      next?.focus();
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      this.select(row);
    }
  }

  protected catClass(row: TxRow): string {
    return categoryColorClass(this.categories(), row.categoryId, row.parentCategoryId);
  }

  protected rowCategory(row: TxRow): string {
    if (row.isTransfer) return this.l.transfer;
    return row.categoryName ?? this.l.uncategorized;
  }

  protected subLine(row: TxRow): string {
    return row.counterparty && row.counterparty !== row.description ? row.counterparty : '';
  }

  protected rowTitle(row: TxRow): string {
    return [row.description, row.counterparty].filter(Boolean).join(' · ');
  }

  protected optionLabel(c: CategoryOption): string {
    return c.parentId === null ? c.name : `  ${c.name}`;
  }

  // Draft

  private resetDraft(row: TxRow): void {
    this.draftCategory.set(row.categoryId);
    this.draftTags.set([...row.tags]);
    this.draftNotes.set(row.notes ?? '');
    this.createRule.set(false);
    this.tagInput.set('');
  }

  protected discard(): void {
    const row = this.selected();
    if (row) this.resetDraft(row);
    this.status.set(null);
  }

  protected addTag(raw: string): void {
    const tag = raw.trim().slice(0, MAX_TAG_LENGTH);
    this.tagInput.set('');
    if (!tag || this.draftTags().length >= MAX_TAGS) return;
    if (this.draftTags().some((x) => x.toLowerCase() === tag.toLowerCase())) return;
    this.draftTags.update((tags) => [...tags, tag]);
  }

  protected onTagKey(event: KeyboardEvent, value: string): void {
    if (event.key === 'Enter' || event.key === ',') {
      event.preventDefault();
      this.addTag(value);
    } else if (event.key === 'Backspace' && !value) {
      this.draftTags.update((tags) => tags.slice(0, -1));
    }
  }

  protected removeTag(tag: string): void {
    this.draftTags.update((tags) => tags.filter((x) => x !== tag));
  }

  protected async save(): Promise<void> {
    const row = this.selected();
    if (!row || this.saving() || !this.dirty()) return;
    // A tag typed but not yet confirmed still counts.
    if (this.tagInput().trim()) this.addTag(this.tagInput());
    this.saving.set(true);
    this.status.set(null);
    let latest = row;
    let recategorized = 0;
    let ruleCreated = false;
    try {
      if (this.categoryChanged() || this.ruleRequested()) {
        const categoryId = this.draftCategory() ?? row.categoryId!;
        const result = await this.ledger.setCategory(row.id, categoryId, this.createRule());
        latest = result.row;
        recategorized = result.recategorized;
        ruleCreated = result.ruleId !== null;
      }
      if (this.notesChanged()) {
        latest = await this.ledger.setNotes(row.id, this.draftNotes().trim() || null);
      }
      if (this.tagsChanged()) {
        latest = await this.ledger.setTags(row.id, this.draftTags());
      }
      this.status.set({ kind: 'ok', text: ruleCreated ? this.t.savedRule(recategorized) : this.t.saved });
      this.resetDraft(latest);
      await this.refresh();
      const fresh = this.rows().find((r) => r.id === latest.id);
      if (fresh) this.resetDraft(fresh);
      void this.refreshTags();
    } catch (e) {
      this.status.set({ kind: 'error', text: `${this.t.saveError} ${toCommandError(e).message}` });
    } finally {
      this.saving.set(false);
    }
  }

  protected async unlink(): Promise<void> {
    const row = this.selected();
    if (!row || this.saving()) return;
    this.saving.set(true);
    this.status.set(null);
    try {
      await this.ledger.unlinkTransfer(row.id);
      this.status.set({ kind: 'ok', text: this.t.unlinked });
      await this.refresh();
      const fresh = this.rows().find((r) => r.id === row.id);
      if (fresh) this.resetDraft(fresh);
    } catch (e) {
      this.status.set({ kind: 'error', text: `${this.t.saveError} ${toCommandError(e).message}` });
    } finally {
      this.saving.set(false);
    }
  }

  private async refreshTags(): Promise<void> {
    try {
      this.tagSuggestions.set(await this.ledger.tags());
    } catch {
      // Suggestions are optional.
    }
  }
}
