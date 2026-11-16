import { Component, OnDestroy, OnInit, computed, inject, signal } from '@angular/core';
import { BackupService } from '../backup/backup.service';
import { BackupStatus, PassphraseCheck } from '../backup/backup.models';
import { toCommandError } from '../bank/bank.service';
import { CommandError } from '../bank/bank.models';
import { formatDate } from '../core/format';
import { STRINGS } from '../core/strings';
import { BankKey } from './bank-key';

/** Pause after the last keystroke before the strength is checked. */
export const CHECK_DEBOUNCE_MS = 300;

/** "Ajustes" page, section "Copias de seguridad". Passphrases live only in the form signals. */
@Component({
  selector: 'app-backups',
  imports: [BankKey],
  templateUrl: './backups.html',
  styleUrl: './backups.css',
})
export class Backups implements OnInit, OnDestroy {
  private readonly service = inject(BackupService);
  protected readonly t = STRINGS.settings;
  protected readonly b = STRINGS.settings.backups;

  protected readonly status = signal<BackupStatus | null>(null);

  // Create
  protected readonly passphrase = signal('');
  protected readonly confirmation = signal('');
  protected readonly check = signal<PassphraseCheck | null>(null);
  protected readonly checking = signal(false);
  protected readonly creating = signal(false);
  protected readonly createdName = signal<string | null>(null);
  protected readonly createError = signal<CommandError | null>(null);

  // Restore
  protected readonly pickedName = signal<string | null>(null);
  protected readonly restorePassphrase = signal('');
  protected readonly confirming = signal(false);
  protected readonly restoring = signal(false);
  protected readonly restored = signal<{ date: string; kept: string | null } | null>(null);
  protected readonly restoreError = signal<CommandError | null>(null);

  protected readonly matches = computed(
    () => this.passphrase().length > 0 && this.passphrase() === this.confirmation(),
  );
  protected readonly showMismatch = computed(
    () => this.confirmation().length > 0 && this.passphrase() !== this.confirmation(),
  );
  protected readonly canCreate = computed(
    () => !this.creating() && !this.restoring() && !this.checking() && this.matches() && this.check()?.ok === true,
  );
  protected readonly canRestore = computed(
    () => !this.restoring() && !this.creating() && this.restorePassphrase().length > 0,
  );
  protected readonly strengthText = computed(() => {
    const c = this.check();
    return c ? this.b.strength[Math.min(Math.max(c.score, 0), 4)] : null;
  });
  protected readonly lastDate = computed(() => formatDate(this.status()?.last_backup_at));

  protected readonly meterSteps = [1, 2, 3, 4];

  private timer: ReturnType<typeof setTimeout> | null = null;
  /** Ignores answers of checks that were superseded by newer typing. */
  private checkSeq = 0;

  ngOnInit(): void {
    void this.loadStatus();
  }

  ngOnDestroy(): void {
    this.clearTimer();
    this.passphrase.set('');
    this.confirmation.set('');
    this.restorePassphrase.set('');
  }

  protected onPassphrase(value: string): void {
    this.passphrase.set(value);
    this.createdName.set(null);
    this.createError.set(null);
    this.scheduleCheck(value);
  }

  protected onConfirmation(value: string): void {
    this.confirmation.set(value);
  }

  protected onRestorePassphrase(value: string): void {
    this.restorePassphrase.set(value);
  }

  protected async create(): Promise<void> {
    if (!this.canCreate()) return;
    this.creating.set(true);
    this.createdName.set(null);
    this.createError.set(null);
    let clear = true;
    try {
      const info = await this.service.create(this.passphrase(), this.confirmation());
      if (info === null) {
        clear = false; // Save dialog cancelled: keep the form to try again.
      } else {
        this.createdName.set(info.file_name);
      }
    } catch (e) {
      this.createError.set(toCommandError(e));
    } finally {
      this.creating.set(false);
      if (clear) this.resetCreateForm();
    }
    await this.loadStatus();
  }

  protected async pick(): Promise<void> {
    this.restoreError.set(null);
    this.restored.set(null);
    try {
      const picked = await this.service.pickFile();
      if (picked) {
        this.pickedName.set(picked.file_name);
        this.restorePassphrase.set('');
        this.confirming.set(false);
      }
    } catch (e) {
      this.restoreError.set(toCommandError(e));
    }
  }

  protected askRestore(): void {
    if (this.canRestore()) this.confirming.set(true);
  }

  protected cancelRestore(): void {
    this.confirming.set(false);
  }

  protected async confirmRestore(): Promise<void> {
    this.confirming.set(false);
    this.restoring.set(true);
    this.restoreError.set(null);
    try {
      const r = await this.service.restore(this.restorePassphrase());
      this.restored.set({ date: formatDate(r.backup_created_at), kept: r.pre_restore_file_name });
      this.pickedName.set(null);
    } catch (e) {
      this.restoreError.set(toCommandError(e));
    } finally {
      this.restoring.set(false);
      this.restorePassphrase.set('');
    }
    await this.loadStatus();
  }

  protected dismissCreateError(): void {
    this.createError.set(null);
  }

  protected dismissRestoreError(): void {
    this.restoreError.set(null);
  }

  private async loadStatus(): Promise<void> {
    try {
      this.status.set(await this.service.status());
    } catch {
      // Outside the desktop shell there is no status.
    }
  }

  private resetCreateForm(): void {
    this.clearTimer();
    this.checkSeq++;
    this.checking.set(false);
    this.passphrase.set('');
    this.confirmation.set('');
    this.check.set(null);
  }

  private clearTimer(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }

  private scheduleCheck(value: string): void {
    this.clearTimer();
    const seq = ++this.checkSeq;
    if (value.length === 0) {
      this.check.set(null);
      this.checking.set(false);
      return;
    }
    this.checking.set(true);
    this.timer = setTimeout(async () => {
      this.timer = null;
      let result: PassphraseCheck | null = null;
      try {
        result = await this.service.checkPassphrase(value);
      } catch {
        // No verdict: the button stays disabled.
      }
      if (seq === this.checkSeq) {
        this.check.set(result);
        this.checking.set(false);
      }
    }, CHECK_DEBOUNCE_MS);
  }
}
