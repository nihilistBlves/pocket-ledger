import { Injectable, inject } from '@angular/core';
import { TAURI_INVOKE } from '../core/tauri';

/** Where the private key path comes from. `env` has priority over `settings`. */
export type BankKeySource = 'env' | 'settings' | 'none';

/** Only the file name reaches the UI: never the full path or the content. */
export interface BankKeyStatus {
  configured: boolean;
  source: BankKeySource;
  file_name: string | null;
  exists: boolean;
}

/** Thin wrapper over the `bank_key_*` Tauri commands. */
@Injectable({ providedIn: 'root' })
export class BankKeyService {
  private readonly invoke = inject(TAURI_INVOKE);

  status(): Promise<BankKeyStatus> {
    return this.invoke<BankKeyStatus>('bank_key_status');
  }

  /** Opens the native file dialog; resolves with the unchanged status when cancelled. */
  pick(): Promise<BankKeyStatus> {
    return this.invoke<BankKeyStatus>('bank_key_pick');
  }

  clear(): Promise<BankKeyStatus> {
    return this.invoke<BankKeyStatus>('bank_key_clear');
  }
}
