import { Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { RouterLink } from '@angular/router';
import { CommandError } from '../bank/bank.models';
import { toCommandError } from '../bank/bank.service';
import { formatCents, formatMonthShort, formatSignedCents, monthOf } from '../core/format';
import { STRINGS } from '../core/strings';
import { NOW } from '../ledger/clock';
import { COLOR_PALETTE, categoryColorClass, colorClass } from '../ledger/colors';
import { CategoryNode, ColorToken } from '../ledger/ledger.models';
import { LedgerService } from '../ledger/ledger.service';
import { Modal } from '../ledger/modal';
import { MonthSelector } from '../ledger/month-selector';

const MAX_NAME = 60;

interface CreateTarget {
  /** The group a subcategory goes into, or null for a new group. */
  parent: CategoryNode | null;
}

@Component({
  selector: 'app-categories',
  imports: [MonthSelector, Modal, RouterLink],
  templateUrl: './categories.html',
  styleUrl: './categories.css',
})
export class Categories {
  protected readonly t = STRINGS.categories;
  protected readonly c = STRINGS.common;
  protected readonly l = STRINGS.ledger;
  protected readonly palette = COLOR_PALETTE;
  protected readonly maxName = MAX_NAME;
  private readonly ledger = inject(LedgerService);
  private readonly now = inject(NOW);
  protected readonly fmt = { cents: formatCents, signed: formatSignedCents, colorClass };

  protected readonly month = signal(monthOf(this.now()));
  protected readonly nodes = signal<CategoryNode[]>([]);
  protected readonly loading = signal(true);
  protected readonly error = signal<CommandError | null>(null);
  protected readonly status = signal<{ kind: 'ok' | 'error'; text: string } | null>(null);
  private requestId = 0;

  protected readonly expanded = signal<ReadonlySet<number>>(new Set());
  protected readonly selectedId = signal<number | null>(null);
  protected readonly saving = signal(false);

  // Draft of the panel
  protected readonly draftName = signal('');
  protected readonly draftParent = signal<number | null>(null);
  protected readonly draftColor = signal<ColorToken | null>(null);

  // Dialogs
  protected readonly createTarget = signal<CreateTarget | null>(null);
  protected readonly createName = signal('');
  protected readonly createKind = signal<'expense' | 'income'>('expense');
  protected readonly createError = signal<string | null>(null);
  protected readonly deleting = signal<CategoryNode | null>(null);
  protected readonly moveTo = signal<number | null>(null);
  protected readonly deleteRules = signal(false);
  protected readonly dialogError = signal<string | null>(null);

  protected readonly monthShort = computed(() => `${formatMonthShort(this.month())}.`);
  protected readonly groups = computed(() => this.nodes().filter((n) => n.parentId === null));
  private readonly childrenOf = computed(() => {
    const map = new Map<number, CategoryNode[]>();
    for (const n of this.nodes()) {
      if (n.parentId === null) continue;
      const list = map.get(n.parentId) ?? [];
      list.push(n);
      map.set(n.parentId, list);
    }
    return map;
  });

  /** Rows in display order: every group, followed by its subcategories when expanded. */
  protected readonly rows = computed(() => {
    const out: CategoryNode[] = [];
    for (const g of this.groups()) {
      out.push(g);
      if (this.expanded().has(g.id)) out.push(...(this.childrenOf().get(g.id) ?? []));
    }
    return out;
  });

  protected readonly caption = computed(() => {
    const groups = this.groups().filter((g) => !g.isSystem).length;
    const subs = this.nodes().filter((n) => n.parentId !== null).length;
    return this.t.treeCaption(groups, subs);
  });

  protected readonly selected = computed(() => this.nodes().find((n) => n.id === this.selectedId()) ?? null);
  protected readonly selectedParent = computed(() => {
    const s = this.selected();
    return s?.parentId != null ? (this.nodes().find((n) => n.id === s.parentId) ?? null) : null;
  });
  /** Groups a subcategory can move to: same kind, not system. */
  protected readonly parentOptions = computed(() => {
    const s = this.selected();
    return s ? this.groups().filter((g) => !g.isSystem && g.kind === s.kind) : [];
  });
  protected readonly dirty = computed(() => {
    const s = this.selected();
    if (!s) return false;
    return (
      (!s.isSystem && this.draftName().trim() !== s.name) ||
      (!s.isSystem && s.parentId !== null && this.draftParent() !== s.parentId) ||
      this.draftColor() !== s.color
    );
  });
  /** Dot colour the "Del grupo" swatch shows: the group's effective colour. */
  protected readonly groupSwatchClass = computed(() => {
    const parentId = this.draftParent() ?? this.selected()?.parentId ?? null;
    return parentId === null ? '' : categoryColorClass(this.nodes(), parentId, null);
  });

  protected readonly deleteTargets = computed(() => {
    const d = this.deleting();
    if (!d) return [];
    const out: CategoryNode[] = [];
    for (const g of this.groups()) {
      if (g.kind !== 'transfer' && g.id !== d.id) out.push(g);
      for (const ch of this.childrenOf().get(g.id) ?? []) if (ch.id !== d.id) out.push(ch);
    }
    return out;
  });

  /** Rules cannot point to a system category: choosing one as the target deletes them. */
  protected readonly targetIsSystem = computed(() => {
    const id = this.moveTo();
    return id !== null && !!this.nodes().find((n) => n.id === id)?.isSystem;
  });

  constructor() {
    void this.ledger.start();
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
      const nodes = await this.ledger.categoryTree(month);
      if (id !== this.requestId) return;
      this.nodes.set(nodes);
      const sel = this.selectedId();
      if (sel !== null) {
        const node = nodes.find((n) => n.id === sel);
        if (node) this.resetDraft(node);
        else this.selectedId.set(null);
      }
    } catch (e) {
      if (id === this.requestId) this.error.set(toCommandError(e));
    } finally {
      if (id === this.requestId) this.loading.set(false);
    }
  }

  protected retry(): void {
    void this.load();
  }

  protected colorOf(n: CategoryNode): string {
    return categoryColorClass(this.nodes(), n.id, n.parentId);
  }

  // Tree

  protected toggle(group: CategoryNode): void {
    this.expanded.update((s) => {
      const next = new Set(s);
      if (!next.delete(group.id)) next.add(group.id);
      return next;
    });
  }

  protected isExpanded(group: CategoryNode): boolean {
    return this.expanded().has(group.id);
  }

  protected kindLabel(n: CategoryNode): string {
    if (n.isSystem || n.kind === 'transfer' || n.kind === 'system') return this.t.kindSystem;
    return n.kind === 'income' ? this.t.kindIncome : this.t.kindExpense;
  }

  protected amountText(n: CategoryNode): string {
    if (n.monthTxCount === 0) return '—';
    if (n.kind === 'transfer') return `⇄ ${formatCents(Math.abs(n.monthAmountCents))}`;
    return formatSignedCents(n.monthAmountCents);
  }

  protected select(n: CategoryNode): void {
    this.selectedId.set(n.id);
    this.status.set(null);
    this.resetDraft(n);
  }

  protected closePanel(): void {
    this.selectedId.set(null);
  }

  protected onRowKey(event: KeyboardEvent, n: CategoryNode): void {
    const el = event.currentTarget as HTMLElement;
    if (event.target !== el) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const next = (event.key === 'ArrowDown' ? el.nextElementSibling : el.previousElementSibling) as HTMLElement | null;
      next?.focus();
    } else if (event.key === 'ArrowRight' && n.parentId === null && n.childCount > 0 && !this.isExpanded(n)) {
      event.preventDefault();
      this.toggle(n);
    } else if (event.key === 'ArrowLeft' && n.parentId === null && this.isExpanded(n)) {
      event.preventDefault();
      this.toggle(n);
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      this.select(n);
    }
  }

  // Panel

  private resetDraft(n: CategoryNode): void {
    this.draftName.set(n.name);
    this.draftParent.set(n.parentId);
    this.draftColor.set(n.color);
  }

  protected discard(): void {
    const s = this.selected();
    if (s) this.resetDraft(s);
    this.status.set(null);
  }

  protected pickColor(token: ColorToken | null): void {
    this.draftColor.set(token);
  }

  protected async save(): Promise<void> {
    const s = this.selected();
    if (!s || this.saving() || !this.dirty()) return;
    const name = s.isSystem ? s.name : this.draftName().trim();
    if (name.length < 1 || name.length > MAX_NAME) {
      this.status.set({ kind: 'error', text: this.t.errors.nameEmpty });
      return;
    }
    this.saving.set(true);
    this.status.set(null);
    try {
      const parentId = s.isSystem || s.parentId === null ? s.parentId : this.draftParent();
      const updated = await this.ledger.updateCategory(s.id, name, parentId, this.draftColor());
      if (updated?.parentId) this.expanded.update((e) => new Set(e).add(updated.parentId!));
      await this.load();
      this.status.set({ kind: 'ok', text: this.c.saved });
    } catch (e) {
      this.status.set({
        kind: 'error',
        text: this.errorText(e, s.parentId !== this.draftParent() ? this.t.errors.moveConflict : this.t.errors.conflict),
      });
    } finally {
      this.saving.set(false);
    }
  }

  private errorText(e: unknown, conflict: string): string {
    const err = toCommandError(e);
    return err.code === 'conflict' ? conflict : `${this.c.saveError} ${err.message}`;
  }

  // Delete

  protected deleteBlocked(n: CategoryNode): boolean {
    return n.childCount > 0;
  }

  protected openDelete(n: CategoryNode): void {
    if (n.isSystem || this.deleteBlocked(n)) return;
    this.deleting.set(n);
    this.deleteRules.set(false);
    this.dialogError.set(null);
    const parent = n.parentId;
    const uncategorized = this.nodes().find((x) => x.isSystem && x.kind !== 'transfer');
    this.moveTo.set(parent ?? uncategorized?.id ?? this.deleteTargets()[0]?.id ?? null);
  }

  protected closeDelete(): void {
    this.deleting.set(null);
  }

  protected targetLabel(n: CategoryNode): string {
    const d = this.deleting();
    if (n.parentId === null) return d?.parentId === n.id ? this.t.deleteTheGroup(n.name) : n.name;
    return `  ${n.name}`;
  }

  protected async confirmDelete(): Promise<void> {
    const d = this.deleting();
    const target = this.moveTo();
    if (!d || target === null || this.saving()) return;
    this.saving.set(true);
    this.dialogError.set(null);
    try {
      await this.ledger.deleteCategory(d.id, target, d.ruleCount > 0 && (this.deleteRules() || this.targetIsSystem()));
      this.deleting.set(null);
      this.selectedId.set(null);
      await this.load();
      this.status.set({ kind: 'ok', text: this.t.deleted(d.name) });
    } catch (e) {
      const err = toCommandError(e);
      this.dialogError.set(err.code === 'conflict' ? this.t.errors.deleteConflict : err.message);
    } finally {
      this.saving.set(false);
    }
  }

  // Create

  protected openCreate(parent: CategoryNode | null): void {
    this.createTarget.set({ parent });
    this.createName.set('');
    this.createKind.set('expense');
    this.createError.set(null);
  }

  protected closeCreate(): void {
    this.createTarget.set(null);
  }

  protected async confirmCreate(): Promise<void> {
    const target = this.createTarget();
    const name = this.createName().trim();
    if (!target || this.saving()) return;
    if (name.length < 1 || name.length > MAX_NAME) {
      this.createError.set(this.t.errors.nameEmpty);
      return;
    }
    this.saving.set(true);
    this.createError.set(null);
    try {
      const node = await this.ledger.createCategory(
        target.parent?.id ?? null,
        name,
        target.parent ? null : this.createKind(),
      );
      this.createTarget.set(null);
      if (target.parent) this.expanded.update((e) => new Set(e).add(target.parent!.id));
      await this.load();
      if (node) {
        const fresh = this.nodes().find((n) => n.id === node.id);
        if (fresh) this.select(fresh);
      }
      this.status.set({ kind: 'ok', text: this.t.created(name) });
    } catch (e) {
      const err = toCommandError(e);
      this.createError.set(err.code === 'conflict' ? this.t.errors.conflict : err.message);
    } finally {
      this.saving.set(false);
    }
  }
}
