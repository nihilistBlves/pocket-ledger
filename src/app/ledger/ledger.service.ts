import { Injectable, inject, signal } from '@angular/core';
import { SYNCED_EVENT } from '../bank/bank.service';
import { TAURI_INVOKE, TAURI_LISTEN } from '../core/tauri';
import {
  BudgetMonth,
  CategoryNode,
  CategoryOption,
  ColorToken,
  Dashboard,
  DeleteCategoryResult,
  RuleInput,
  RuleRow,
  RuleTest,
  SaveRuleResult,
  SetCategoryResult,
  TxFilter,
  TxPage,
  TxRow,
} from './ledger.models';

/** Wraps the `ledger_*` Tauri commands. `syncTick` changes whenever a bank sync touched the data. */
@Injectable({ providedIn: 'root' })
export class LedgerService {
  private readonly invoke = inject(TAURI_INVOKE);
  private readonly listen = inject(TAURI_LISTEN);
  private started = false;

  /** Incremented on every `bank://synced` event; screens reload when it changes. */
  readonly syncTick = signal(0);

  /** Starts listening for sync events (idempotent). */
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    try {
      await this.listen(SYNCED_EVENT, () => this.syncTick.update((n) => n + 1));
    } catch {
      // Outside the desktop shell there are no events.
    }
  }

  dashboard(month: string, today: string): Promise<Dashboard> {
    return this.invoke<Dashboard>('ledger_dashboard', { month, today });
  }

  transactions(filter: TxFilter): Promise<TxPage> {
    return this.invoke<TxPage>('ledger_transactions', { filter });
  }

  categories(): Promise<CategoryOption[]> {
    return this.invoke<CategoryOption[]>('ledger_categories');
  }

  tags(): Promise<string[]> {
    return this.invoke<string[]>('ledger_tags');
  }

  setCategory(txId: number, categoryId: number, createRule: boolean): Promise<SetCategoryResult> {
    return this.invoke<SetCategoryResult>('ledger_set_category', { txId, categoryId, createRule });
  }

  setNotes(txId: number, notes: string | null): Promise<TxRow> {
    return this.invoke<TxRow>('ledger_set_notes', { txId, notes });
  }

  setTags(txId: number, tags: string[]): Promise<TxRow> {
    return this.invoke<TxRow>('ledger_set_tags', { txId, tags });
  }

  unlinkTransfer(txId: number): Promise<TxRow[]> {
    return this.invoke<TxRow[]>('ledger_unlink_transfer', { txId });
  }

  // Categories

  categoryTree(month: string): Promise<CategoryNode[]> {
    return this.invoke<CategoryNode[]>('ledger_category_tree', { month });
  }

  createCategory(parentId: number | null, name: string, kind: 'income' | 'expense' | null): Promise<CategoryNode> {
    return this.invoke<CategoryNode>('ledger_create_category', { parentId, name, kind });
  }

  updateCategory(id: number, name: string, parentId: number | null, color: ColorToken | null): Promise<CategoryNode> {
    return this.invoke<CategoryNode>('ledger_update_category', { id, name, parentId, color });
  }

  deleteCategory(id: number, moveTo: number, deleteRules: boolean): Promise<DeleteCategoryResult> {
    return this.invoke<DeleteCategoryResult>('ledger_delete_category', { id, moveTo, deleteRules });
  }

  // Rules

  rules(): Promise<RuleRow[]> {
    return this.invoke<RuleRow[]>('ledger_rules');
  }

  saveRule(id: number | null, rule: RuleInput, applyToExisting: boolean, today: string): Promise<SaveRuleResult> {
    return this.invoke<SaveRuleResult>('ledger_save_rule', { id, rule, applyToExisting, today });
  }

  deleteRule(id: number): Promise<void> {
    return this.invoke<void>('ledger_delete_rule', { id });
  }

  testRule(id: number | null, rule: RuleInput, today: string): Promise<RuleTest> {
    return this.invoke<RuleTest>('ledger_test_rule', { id, rule, today });
  }

  applyRulesUncategorized(): Promise<{ recategorized: number }> {
    return this.invoke<{ recategorized: number }>('ledger_apply_rules_uncategorized');
  }

  // Budgets

  budgets(month: string, today: string): Promise<BudgetMonth> {
    return this.invoke<BudgetMonth>('ledger_budgets', { month, today });
  }

  setBudget(categoryId: number, month: string, amountCents: number): Promise<void> {
    return this.invoke<void>('ledger_set_budget', { categoryId, month, amountCents });
  }

  removeBudget(categoryId: number, month: string): Promise<void> {
    return this.invoke<void>('ledger_remove_budget', { categoryId, month });
  }
}
