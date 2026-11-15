import { STRINGS } from '../core/strings';
import { RunView } from './bank.models';

const E = STRINGS.accounts.errors;

/** Spanish explanation of the short error code stored with a failed sync run. */
export function errorDetail(code: string | null): string {
  if (!code) return E.generic('desconocido');
  switch (code) {
    case 'ASPSP_RATE_LIMIT_EXCEEDED':
      return E.rateLimit;
    case 'EXPIRED_SESSION':
    case 'REVOKED_SESSION':
    case 'CLOSED_SESSION':
    case 'UNAUTHORIZED':
      return E.reauth;
    case 'ASPSP_TIMEOUT':
      return E.bankTimeout;
    case 'ASPSP_ERROR':
      return E.bankError;
    case 'NETWORK_TIMEOUT':
    case 'NETWORK_CONNECT':
    case 'NETWORK_OTHER':
      return E.network;
    case 'KEY_ERROR':
    case 'INVALID_CONFIG':
      return E.key;
    case 'DB_ERROR':
      return E.database;
    case 'ABORTED':
      return E.aborted;
    default:
      return E.generic(code);
  }
}

/** "N nuevos, M actualizados", "Sin cambios", or a dash for a failed run with no rows. */
export function movementsText(run: RunView): string {
  const t = STRINGS.accounts;
  if (run.status === 'running') return t.running;
  const parts: string[] = [];
  if (run.inserted > 0) parts.push(t.newCount(run.inserted));
  if (run.updated > 0) parts.push(t.updatedCount(run.updated));
  if (parts.length > 0) return parts.join(', ');
  return run.status === 'error' ? '—' : t.noChanges;
}
