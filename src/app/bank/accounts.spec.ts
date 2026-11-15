import { TestBed } from '@angular/core/testing';
import { TAURI_INVOKE, TAURI_LISTEN } from '../core/tauri';
import { formatCents, formatDate, formatDuration } from '../core/format';
import { Accounts } from './accounts';
import { BankOverview, CommandError } from './bank.models';
import { BankService, SYNCED_EVENT } from './bank.service';
import { errorDetail, movementsText } from './run-detail';

const ACCOUNTS = [
  { id: 1, name: 'Cuenta Nómina', kind: 'checking', display_suffix: '4821', balance_cents: 318452, currency: 'EUR', last_synced_at: '2026-11-15T10:48:00.000Z' },
  { id: 2, name: 'Tarjeta de crédito', kind: 'card', display_suffix: '0156', balance_cents: -10573, currency: 'EUR', last_synced_at: null },
] as const;

const RUNS = [
  { started_at: '2026-11-15T10:48:00.000Z', trigger: 'manual', status: 'ok', inserted: 4, updated: 1, error_code: null, duration_ms: 3800 },
  { started_at: '2026-11-14T19:00:00.000Z', trigger: 'auto', status: 'error', inserted: 0, updated: 0, error_code: 'ASPSP_RATE_LIMIT_EXCEEDED', duration_ms: 1200 },
  { started_at: '2026-11-13T19:00:00.000Z', trigger: 'auto', status: 'ok', inserted: 0, updated: 0, error_code: null, duration_ms: 2700 },
] as const;

function overview(connection: BankOverview['connection'], extra: Partial<BankOverview> = {}): BankOverview {
  return {
    configured: true,
    aspsp_name: 'Openbank',
    connection,
    accounts: structuredClone(ACCOUNTS) as unknown as BankOverview['accounts'],
    runs: structuredClone(RUNS) as unknown as BankOverview['runs'],
    next_auto_sync_at: null,
    ...extra,
  };
}

const connected = (days: number, total = 180): BankOverview['connection'] => ({
  kind: 'connected',
  aspsp_name: 'Openbank',
  valid_until: '2027-04-06T10:00:00.000Z',
  days_remaining: days,
  consent_days_total: total,
  granted_on: '2026-10-08T10:00:00.000Z',
});

describe('Accounts screen', () => {
  let current: BankOverview;
  let calls: string[];
  let failWith: CommandError | null;
  let emit: () => void;

  async function render() {
    const fixture = TestBed.createComponent(Accounts);
    await fixture.whenStable();
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }

  const button = (el: HTMLElement, text: string) =>
    Array.from(el.querySelectorAll('button')).find((b) => b.textContent?.trim().includes(text)) as HTMLButtonElement | undefined;

  beforeEach(() => {
    calls = [];
    failWith = null;
    emit = () => undefined;
    TestBed.configureTestingModule({
      providers: [
        {
          provide: TAURI_INVOKE,
          useValue: async (command: string) => {
            calls.push(command);
            if (command !== 'bank_overview' && failWith) throw failWith;
            return command === 'bank_overview' ? structuredClone(current) : undefined;
          },
        },
        {
          provide: TAURI_LISTEN,
          useValue: async (_event: string, handler: () => void) => {
            emit = handler;
            return () => undefined;
          },
        },
      ],
    });
  });

  it('shows a connected bank with dates, progress, accounts and history', async () => {
    current = overview(connected(142));
    const { el } = await render();
    const text = el.textContent ?? '';
    expect(el.querySelector('h1')?.textContent).toBe('Cuentas y conexión');
    expect(text).toContain('Openbank');
    expect(text).toContain('Conectado');
    expect(text).toContain('08/10/2026');
    expect(text).toContain('06/04/2027');
    expect(text).toContain('142 de 180');
    const bar = el.querySelector('[role="progressbar"]');
    expect(bar?.getAttribute('aria-valuenow')).toBe('142');
    expect(bar?.getAttribute('aria-valuemax')).toBe('180');
    expect(text).toContain('•••• 4821');
    expect(text).toContain('3.184,52 €');
    expect(text).toContain('−105,73 €');
    expect(text).toContain('Cuenta corriente');
    expect(text).toContain('Tarjeta');
    expect(text).toContain('Total');
    expect(text).toContain('3.078,79 €');
    expect(text).toContain('4 nuevos, 1 actualizado');
    expect(text).toContain('Sin cambios');
    expect(text).toContain('Manual');
    expect(text).toContain('Automática');
    expect(text).toContain('3,8 s');
    expect(text).toContain('exceso de solicitudes');
    expect(el.querySelector('.banner')).toBeNull();
    expect(button(el, 'Renovar consentimiento')).toBeTruthy();
    expect(button(el, 'Conectar con el banco')).toBeUndefined();
  });

  it('shows the renewal banner under 14 days', async () => {
    current = overview(connected(9));
    const { el } = await render();
    const banner = el.querySelector('.banner[role="status"]');
    expect(banner?.textContent).toContain('caduca en 9 días');
    expect(banner?.textContent).toContain('06/04/2027');
    expect(button(el, 'Renovar ahora')).toBeTruthy();
  });

  it('shows an expired connection', async () => {
    current = overview({
      kind: 'expired',
      aspsp_name: 'Openbank',
      valid_until: '2026-11-01T10:00:00.000Z',
      consent_days_total: 180,
      granted_on: '2026-05-05T10:00:00.000Z',
    });
    const { el } = await render();
    expect(el.textContent).toContain('Caducado');
    expect(el.textContent).toContain('Caducó el');
    expect(el.textContent).toContain('0 de 180');
    expect(el.querySelector('.banner.is-error')?.textContent).toContain('caducó el 01/11/2026');
    expect(button(el, 'Renovar consentimiento')).toBeTruthy();
    expect(button(el, 'Sincronizar ahora')?.disabled).toBe(true);
  });

  it('offers to connect when there is no connection', async () => {
    current = overview({ kind: 'not_connected' }, { accounts: [], runs: [] });
    const { el } = await render();
    expect(el.textContent).toContain('No conectado');
    expect(el.textContent).toContain('se abrirá tu navegador');
    expect(button(el, 'Conectar con el banco')?.disabled).toBe(false);
    expect(button(el, 'Desconectar')).toBeUndefined();
    expect(button(el, 'Sincronizar ahora')?.disabled).toBe(true);
    expect(el.textContent).toContain('Todavía no hay cuentas');
  });

  it('explains the missing configuration and blocks connecting', async () => {
    current = overview({ kind: 'not_connected' }, { configured: false, aspsp_name: null, accounts: [], runs: [] });
    const { el } = await render();
    expect(el.textContent).toContain('enable-banking.json');
    expect(el.textContent).toContain('ENABLE_BANKING_KEY_PATH');
    expect(button(el, 'Conectar con el banco')?.disabled).toBe(true);
  });

  it('shows the command error message in an alert', async () => {
    current = overview({ kind: 'not_connected' });
    failWith = { code: 'port_in_use', message: 'El puerto 8080 está ocupado por otra aplicación.' };
    const { fixture, el } = await render();
    button(el, 'Conectar con el banco')?.click();
    await fixture.whenStable();
    fixture.detectChanges();
    const alert = el.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain('El puerto 8080 está ocupado');
    alert?.querySelector("button")?.click();
    fixture.detectChanges();
    expect(el.querySelector("[role=alert]")).toBeNull();
  });

  it('disables the buttons and explains the wait while connecting', async () => {
    current = overview({ kind: 'not_connected' });
    const bank = TestBed.inject(BankService);
    const { fixture, el } = await render();
    bank.busy.set('connect');
    fixture.detectChanges();
    expect(el.textContent).toContain('Esperando la autorización en el navegador… (máx. 5 min)');
    expect(button(el, 'Conectar con el banco')?.disabled).toBe(true);
    expect(button(el, 'Sincronizar ahora')?.disabled).toBe(true);
  });

  it('asks for confirmation before disconnecting and keeps the data note', async () => {
    current = overview(connected(100));
    const { fixture, el } = await render();
    button(el, 'Desconectar')?.click();
    fixture.detectChanges();
    expect(el.textContent).toContain('¿Desconectar de Openbank?');
    expect(el.textContent).toContain('se conservan los movimientos ya importados');
    expect(calls).not.toContain('bank_disconnect');
    button(el, 'Cancelar')?.click();
    fixture.detectChanges();
    expect(button(el, 'Desconectar')).toBeTruthy();
    button(el, 'Desconectar')?.click();
    fixture.detectChanges();
    button(el, 'Sí, desconectar')?.click();
    await fixture.whenStable();
    expect(calls).toContain('bank_disconnect');
  });

  it('syncs on demand and refreshes when the backend reports a sync', async () => {
    current = overview(connected(100));
    const { fixture, el } = await render();
    button(el, 'Sincronizar ahora')?.click();
    await fixture.whenStable();
    expect(calls).toContain('bank_sync_now');

    const before = calls.filter((c) => c === 'bank_overview').length;
    current = overview(connected(100), { runs: [] });
    emit();
    await fixture.whenStable();
    fixture.detectChanges();
    expect(calls.filter((c) => c === 'bank_overview').length).toBe(before + 1);
    expect(el.textContent).toContain('Aún no se ha sincronizado nada');
  });

  it('listens for the sync event name used by the backend', () => {
    expect(SYNCED_EVENT).toBe('bank://synced');
  });
});

describe('formatting and run details', () => {
  it('formats cents as es-ES currency', () => {
    expect(formatCents(123456)).toBe('1.234,56 €');
    expect(formatCents(5)).toBe('0,05 €');
    expect(formatCents(-10573)).toBe('−105,73 €');
    expect(formatCents(100000000)).toBe('1.000.000,00 €');
  });

  it('formats dates and durations', () => {
    expect(formatDate(new Date(2026, 10, 5, 9, 3))).toBe('05/11/2026');
    expect(formatDate(null)).toBe('—');
    expect(formatDuration(3800)).toBe('3,8 s');
    expect(formatDuration(null)).toBe('—');
  });

  it('describes runs without exposing raw codes for known errors', () => {
    expect(errorDetail('ASPSP_RATE_LIMIT_EXCEEDED')).toContain('exceso de solicitudes');
    expect(errorDetail('EXPIRED_SESSION')).toContain('Renuévalo');
    expect(errorDetail('SOMETHING_NEW')).toContain('SOMETHING_NEW');
    const run = { started_at: '', trigger: 'auto', status: 'ok', inserted: 1, updated: 2, error_code: null, duration_ms: 1 } as const;
    expect(movementsText(run)).toBe('1 nuevo, 2 actualizados');
    expect(movementsText({ ...run, inserted: 0, updated: 0 })).toBe('Sin cambios');
    expect(movementsText({ ...run, inserted: 0, updated: 0, status: 'error' })).toBe('—');
  });
});
