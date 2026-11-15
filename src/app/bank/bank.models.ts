/** Shapes returned by the Rust bank commands. None of them carries a secret. */

export type ConnectionView =
  | { kind: 'not_connected' }
  | {
      kind: 'connected';
      aspsp_name: string;
      valid_until: string;
      days_remaining: number;
      consent_days_total: number;
      granted_on: string;
    }
  | {
      kind: 'expired';
      aspsp_name: string;
      valid_until: string;
      consent_days_total: number;
      granted_on: string;
    };

export type AccountKind = 'checking' | 'savings' | 'card' | 'cash' | 'other';

export interface AccountView {
  id: number;
  name: string;
  kind: AccountKind;
  display_suffix: string | null;
  balance_cents: number | null;
  currency: string;
  last_synced_at: string | null;
}

export interface RunView {
  started_at: string;
  trigger: 'manual' | 'auto';
  status: 'running' | 'ok' | 'error';
  inserted: number;
  updated: number;
  error_code: string | null;
  duration_ms: number | null;
}

export interface BankOverview {
  configured: boolean;
  aspsp_name: string | null;
  connection: ConnectionView;
  accounts: AccountView[];
  runs: RunView[];
  next_auto_sync_at: string | null;
}

export interface CommandError {
  code: string;
  message: string;
}

export interface SyncResult {
  accounts: number;
  inserted: number;
  updated: number;
  pending: number;
  skipped: number;
  duration_ms: number;
}

export interface ConnectResult {
  accounts_linked: number;
  valid_until: string;
}

export type BankBusy = 'connect' | 'sync' | 'disconnect';
