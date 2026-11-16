import { Injectable, inject } from '@angular/core';
import { TAURI_INVOKE } from '../core/tauri';
import {
  BackupInfo,
  BackupStatus,
  PassphraseCheck,
  PickedBackup,
  RestoreResult,
} from './backup.models';

/**
 * Thin wrapper over the `backup_*` / `recovery_*` Tauri commands. Passphrases are
 * passed straight through: nothing is stored here.
 */
@Injectable({ providedIn: 'root' })
export class BackupService {
  private readonly invoke = inject(TAURI_INVOKE);

  status(): Promise<BackupStatus> {
    return this.invoke<BackupStatus>('backup_status');
  }

  checkPassphrase(passphrase: string): Promise<PassphraseCheck> {
    return this.invoke<PassphraseCheck>('backup_check_passphrase', { passphrase });
  }

  /** `null` when the Save dialog was cancelled. */
  create(passphrase: string, confirmation: string): Promise<BackupInfo | null> {
    return this.invoke<BackupInfo | null>('backup_create', { passphrase, confirmation });
  }

  /** Opens the native file dialog; `null` when cancelled. Only the file name comes back. */
  pickFile(): Promise<PickedBackup | null> {
    return this.invoke<PickedBackup | null>('backup_pick_file');
  }

  restore(passphrase: string): Promise<RestoreResult> {
    return this.invoke<RestoreResult>('backup_restore', { passphrase });
  }

  recoveryUnlock(passphrase: string): Promise<void> {
    return this.invoke<void>('recovery_unlock', { passphrase });
  }

  recoveryRestore(passphrase: string): Promise<void> {
    return this.invoke<void>('recovery_restore', { passphrase });
  }
}
