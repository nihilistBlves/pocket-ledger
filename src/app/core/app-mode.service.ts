import { Injectable, inject, signal } from '@angular/core';
import { AppMode, RecoveryReason } from '../backup/backup.models';
import { TAURI_INVOKE } from './tauri';

/**
 * Whether the app runs normally or in recovery mode (database key missing or wrong).
 * Outside the desktop shell, or if the command fails, the app is treated as ready.
 */
@Injectable({ providedIn: 'root' })
export class AppModeService {
  private readonly invoke = inject(TAURI_INVOKE);

  readonly state = signal<'loading' | 'ready' | 'recovery'>('loading');
  readonly reason = signal<RecoveryReason | null>(null);

  async load(): Promise<void> {
    try {
      const m = await this.invoke<AppMode>('app_mode');
      this.reason.set(m.mode === 'recovery' ? m.reason : null);
      this.state.set(m.mode === 'recovery' ? 'recovery' : 'ready');
    } catch {
      this.reason.set(null);
      this.state.set('ready');
    }
  }
}
