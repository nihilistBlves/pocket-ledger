/** Shapes returned by the Rust backup commands. None of them carries a secret or a path. */

export type RecoveryReason = 'key_missing' | 'key_malformed' | 'key_wrong';

export interface AppMode {
  mode: 'ready' | 'recovery';
  reason: RecoveryReason | null;
}

export interface BackupStatus {
  last_backup_at: string | null;
  last_file_name: string | null;
  reminder: boolean;
}

export interface PassphraseCheck {
  ok: boolean;
  /** zxcvbn score, 0..=4. */
  score: number;
  feedback: string | null;
}

export interface BackupInfo {
  created_at: string;
  file_name: string;
  size: number;
}

export interface PickedBackup {
  file_name: string;
}

export interface RestoreResult {
  backup_created_at: string;
  pre_restore_file_name: string | null;
}
