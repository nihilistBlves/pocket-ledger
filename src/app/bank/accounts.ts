import { Component, OnDestroy, OnInit, computed, inject, signal } from '@angular/core';
import { formatCents, formatDate, formatDateTime, formatDuration } from '../core/format';
import { STRINGS } from '../core/strings';
import { AccountView, RunView } from './bank.models';
import { BankService } from './bank.service';
import { errorDetail, movementsText } from './run-detail';

/** Days left under which the renewal banner is shown. */
export const RENEW_WARNING_DAYS = 14;

@Component({
  selector: 'app-accounts',
  templateUrl: './accounts.html',
  styleUrl: './accounts.css',
})
export class Accounts implements OnInit, OnDestroy {
  protected readonly bank = inject(BankService);
  protected readonly t = STRINGS.accounts;

  protected readonly confirming = signal(false);
  protected readonly overview = this.bank.overview;
  protected readonly connection = computed(() => this.overview()?.connection ?? null);
  protected readonly bankName = computed(
    () => this.overview()?.aspsp_name ?? this.t.bankFallback,
  );
  protected readonly configured = computed(() => this.overview()?.configured ?? false);
  protected readonly isConnected = computed(() => this.connection()?.kind === 'connected');
  protected readonly isExpired = computed(() => this.connection()?.kind === 'expired');
  protected readonly canConnect = computed(
    () => this.configured() && this.bank.busy() === null,
  );
  protected readonly canSync = computed(() => this.isConnected() && this.bank.busy() === null);

  protected readonly validUntil = computed(() => {
    const c = this.connection();
    return c && c.kind !== 'not_connected' ? c.valid_until : null;
  });

  /** Set while connected and under the warning threshold. */
  protected readonly expiring = computed(() => {
    const c = this.connection();
    return c?.kind === 'connected' && c.days_remaining < RENEW_WARNING_DAYS
      ? { days: c.days_remaining, validUntil: c.valid_until }
      : null;
  });

  protected readonly progress = computed(() => {
    const c = this.connection();
    if (!c || c.kind === 'not_connected') return null;
    const total = Math.max(c.consent_days_total, 1);
    const days = c.kind === 'connected' ? Math.min(Math.max(c.days_remaining, 0), total) : 0;
    return { days, total, percent: (days * 100) / total };
  });

  protected readonly total = computed(() => {
    const withBalance = (this.overview()?.accounts ?? []).filter((a) => a.balance_cents !== null);
    const currencies = new Set(withBalance.map((a) => a.currency));
    if (withBalance.length < 2 || currencies.size !== 1) return null;
    const cents = withBalance.reduce((sum, a) => sum + (a.balance_cents ?? 0), 0);
    return formatCents(cents, withBalance[0].currency);
  });

  protected readonly nextAuto = computed(() => {
    const at = this.overview()?.next_auto_sync_at;
    return at ? this.t.nextAuto(formatDateTime(at)) : null;
  });

  protected readonly date = formatDate;
  protected readonly dateTime = formatDateTime;
  protected readonly duration = formatDuration;
  protected readonly detail = errorDetail;
  protected readonly movements = movementsText;

  ngOnInit(): void {
    void this.bank.start();
  }

  ngOnDestroy(): void {
    this.bank.stop();
  }

  protected money(a: AccountView): string {
    return a.balance_cents === null ? '—' : formatCents(a.balance_cents, a.currency);
  }

  protected mask(a: AccountView): string | null {
    return a.display_suffix ? `•••• ${a.display_suffix}` : null;
  }

  protected kind(a: AccountView): string {
    return this.t.kinds[a.kind] ?? this.t.kinds.other;
  }

  protected resultLabel(r: RunView): string {
    return this.t.result[r.status];
  }

  protected askDisconnect(): void {
    this.confirming.set(true);
  }

  protected cancelDisconnect(): void {
    this.confirming.set(false);
  }

  protected async confirmDisconnect(): Promise<void> {
    this.confirming.set(false);
    await this.bank.disconnect();
  }
}
