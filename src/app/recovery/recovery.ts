import { Component, OnDestroy, computed, inject, signal } from '@angular/core';
import { BackupService } from '../backup/backup.service';
import { CommandError } from '../bank/bank.models';
import { toCommandError } from '../bank/bank.service';
import { AppModeService } from '../core/app-mode.service';
import { STRINGS } from '../core/strings';

type Choice = 'unlock' | 'restore';

/**
 * Shown instead of the whole app when the database key is missing or does not open
 * the database. Nothing else is rendered, so no bank screen can run.
 */
@Component({
  selector: 'app-recovery',
  templateUrl: './recovery.html',
  styleUrl: './recovery.css',
})
export class Recovery implements OnDestroy {
  private readonly service = inject(BackupService);
  private readonly mode = inject(AppModeService);
  protected readonly t = STRINGS.recovery;
  protected readonly appName = STRINGS.appName;

  protected readonly choice = signal<Choice | null>(null);
  protected readonly pickedName = signal<string | null>(null);
  protected readonly passphrase = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal<CommandError | null>(null);

  protected readonly reasonText = computed(() => {
    switch (this.mode.reason()) {
      case 'key_malformed':
        return this.t.keyMalformed;
      case 'key_wrong':
        return this.t.keyWrong;
      default:
        return this.t.keyMissing;
    }
  });

  protected canRun(kind: Choice): boolean {
    return this.choice() === kind && this.passphrase().length > 0 && !this.busy();
  }

  ngOnDestroy(): void {
    this.passphrase.set('');
  }

  protected onPassphrase(value: string): void {
    this.passphrase.set(value);
  }

  protected async pick(kind: Choice): Promise<void> {
    if (this.busy()) return;
    this.error.set(null);
    try {
      const picked = await this.service.pickFile();
      if (picked) {
        this.choice.set(kind);
        this.pickedName.set(picked.file_name);
        this.passphrase.set('');
      }
    } catch (e) {
      this.error.set(toCommandError(e));
    }
  }

  protected async run(kind: Choice): Promise<void> {
    if (!this.canRun(kind)) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const passphrase = this.passphrase();
      if (kind === 'unlock') await this.service.recoveryUnlock(passphrase);
      else await this.service.recoveryRestore(passphrase);
      this.passphrase.set('');
      await this.mode.load();
    } catch (e) {
      this.error.set(toCommandError(e));
      this.passphrase.set('');
    } finally {
      this.busy.set(false);
    }
  }

  protected dismissError(): void {
    this.error.set(null);
  }
}
