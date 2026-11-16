import { Component, OnInit, inject, signal } from '@angular/core';
import { toCommandError } from '../bank/bank.service';
import { CommandError } from '../bank/bank.models';
import { STRINGS } from '../core/strings';
import { BankKeyService, BankKeyStatus } from './bank-key.service';

/** "Ajustes" section "Conexión bancaria": where the Enable Banking private key file is. */
@Component({
  selector: 'app-bank-key',
  templateUrl: './bank-key.html',
  styleUrl: './bank-key.css',
})
export class BankKey implements OnInit {
  private readonly service = inject(BankKeyService);
  protected readonly k = STRINGS.settings.bankKey;

  protected readonly status = signal<BankKeyStatus | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal<CommandError | null>(null);

  ngOnInit(): void {
    void this.run(() => this.service.status());
  }

  protected pick(): Promise<void> {
    return this.run(() => this.service.pick());
  }

  protected clear(): Promise<void> {
    return this.run(() => this.service.clear());
  }

  protected dismissError(): void {
    this.error.set(null);
  }

  private async run(action: () => Promise<BankKeyStatus>): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      this.status.set(await action());
    } catch (e) {
      this.error.set(toCommandError(e));
    } finally {
      this.busy.set(false);
    }
  }
}
