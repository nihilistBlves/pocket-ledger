import { Injectable, inject, signal } from '@angular/core';
import { SYNCED_EVENT } from '../bank/bank.service';
import { TAURI_INVOKE, TAURI_LISTEN } from '../core/tauri';
import {
  CategoryOption,
  Dashboard,
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
}
