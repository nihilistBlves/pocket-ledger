import { Injectable, inject, signal } from '@angular/core';
import { STRINGS } from '../core/strings';
import { TAURI_INVOKE, TAURI_LISTEN } from '../core/tauri';
import { BankBusy, BankOverview, CommandError } from './bank.models';

/** Emitted by the Rust side after a sync run touched the database. */
export const SYNCED_EVENT = 'bank://synced';
/** Emitted after a backup restore replaced the database. */
export const REPLACED_EVENT = 'data://replaced';

export function toCommandError(e: unknown): CommandError {
  if (typeof e === 'object' && e !== null) {
    const { code, message } = e as Partial<CommandError>;
    if (typeof code === 'string' && typeof message === 'string') return { code, message };
  }
  return { code: 'internal', message: STRINGS.accounts.errors.internal };
}

/** State of the bank connection, wrapping the `bank_*` Tauri commands. */
@Injectable({ providedIn: 'root' })
export class BankService {
  private readonly invoke = inject(TAURI_INVOKE);
  private readonly listen = inject(TAURI_LISTEN);
  private unlisten: (() => void) | null = null;
  private started = false;

  readonly overview = signal<BankOverview | null>(null);
  /** The operation in progress, if any. */
  readonly busy = signal<BankBusy | null>(null);
  readonly error = signal<CommandError | null>(null);
  readonly loaded = signal(false);

  /** Loads the overview and keeps it fresh when a sync finishes. */
  async start(): Promise<void> {
    if (this.started) return;
    this.started = true;
    // The first load does not wait for the event listeners to be registered.
    const listening = Promise.all([
      this.listen(SYNCED_EVENT, () => void this.refresh()),
      this.listen(REPLACED_EVENT, () => void this.refresh()),
    ]).then(
      ([offSynced, offReplaced]) => {
        if (!this.started) {
          // stop() ran while the listeners were being registered.
          offSynced();
          offReplaced();
          return;
        }
        this.unlisten = () => {
          offSynced();
          offReplaced();
        };
      },
      () => {
        // Outside the desktop shell there are no events.
      },
    );
    await Promise.all([this.refresh(), listening]);
  }

  stop(): void {
    this.unlisten?.();
    this.unlisten = null;
    this.started = false;
  }

  async refresh(): Promise<void> {
    try {
      this.overview.set(await this.invoke<BankOverview>('bank_overview'));
    } catch (e) {
      this.error.set(toCommandError(e));
    } finally {
      this.loaded.set(true);
    }
  }

  /** Authorizes at the bank (system browser); also renews the consent. */
  connect(): Promise<void> {
    return this.run('connect', 'bank_connect');
  }

  syncNow(): Promise<void> {
    return this.run('sync', 'bank_sync_now');
  }

  disconnect(): Promise<void> {
    return this.run('disconnect', 'bank_disconnect');
  }

  clearError(): void {
    this.error.set(null);
  }

  private async run(kind: BankBusy, command: string): Promise<void> {
    if (this.busy() !== null) return;
    this.busy.set(kind);
    this.error.set(null);
    try {
      await this.invoke(command);
    } catch (e) {
      this.error.set(toCommandError(e));
    } finally {
      this.busy.set(null);
    }
    // Also after a failure: a failed run is part of the history.
    await this.refresh();
  }
}
