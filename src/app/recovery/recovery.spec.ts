import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { App } from '../app';
import { routes } from '../app.routes';
import { AppInfoService } from '../core/app-info.service';
import { TAURI_INVOKE } from '../core/tauri';
import { Recovery } from './recovery';

describe('Recovery mode', () => {
  let mode: { mode: 'ready' | 'recovery'; reason: string | null };
  let calls: { command: string; args?: Record<string, unknown> }[];
  let failWith: { code: string; message: string } | null;

  const settle = async (fixture: { whenStable(): Promise<unknown>; detectChanges(): void }) => {
    await fixture.whenStable();
    fixture.detectChanges();
  };
  const button = (el: HTMLElement, text: string) =>
    Array.from(el.querySelectorAll('button')).find((b) => b.textContent?.trim().includes(text)) as
      | HTMLButtonElement
      | undefined;

  beforeEach(() => {
    calls = [];
    failWith = null;
    mode = { mode: 'recovery', reason: 'key_missing' };
    TestBed.configureTestingModule({
      imports: [App],
      providers: [
        provideRouter(routes),
        { provide: AppInfoService, useValue: { get: async () => ({ name: 'Pocket Ledger', version: '9.9.9' }) } },
        {
          provide: TAURI_INVOKE,
          useValue: async (command: string, args?: Record<string, unknown>) => {
            calls.push({ command, args });
            switch (command) {
              case 'app_mode':
                return structuredClone(mode);
              case 'backup_pick_file':
                return { file_name: 'copia-ficticia.plbackup' };
              case 'recovery_unlock':
              case 'recovery_restore':
                if (failWith) throw failWith;
                mode = { mode: 'ready', reason: null };
                return undefined;
              default:
                throw new Error(`unexpected ${command}`);
            }
          },
        },
      ],
    });
  });

  it('renders only the recovery screen, with no navigation or router', async () => {
    const fixture = TestBed.createComponent(App);
    await settle(fixture);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('app-recovery')).not.toBeNull();
    expect(el.querySelector('nav')).toBeNull();
    expect(el.querySelector('router-outlet')).toBeNull();
    expect(el.querySelector('h1')?.textContent).toBe('Modo de recuperación');
    expect(el.querySelector('[data-test="reason"]')?.textContent).toContain(
      'no encuentra la clave de tu base de datos',
    );
    expect(el.textContent).toContain('No se ha borrado nada');
    expect(el.textContent).toContain('Desbloquear con una copia');
    expect(el.textContent).toContain('Recomendado');
    expect(el.textContent).toContain('Restaurar una copia');
  });

  it('explains a key that does not open the database', async () => {
    mode = { mode: 'recovery', reason: 'key_wrong' };
    const fixture = TestBed.createComponent(App);
    await settle(fixture);
    await settle(fixture);
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('no abre la base de datos');
  });

  it('renders the normal shell when the app is ready', async () => {
    mode = { mode: 'ready', reason: null };
    const fixture = TestBed.createComponent(App);
    await settle(fixture);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    expect(el.querySelector('app-recovery')).toBeNull();
    expect(Array.from(el.querySelectorAll('nav a')).map((a) => a.textContent?.trim())).toEqual([
      'Resumen',
      'Movimientos',
      'Categorías',
      'Reglas',
      'Presupuestos',
      'Cuentas',
      'Ajustes',
    ]);
  });

  it('unlocks with a backup and switches to the normal shell', async () => {
    const fixture = TestBed.createComponent(App);
    await settle(fixture);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;

    button(el, 'Elegir archivo de copia…')!.click();
    await settle(fixture);
    expect(el.textContent).toContain('copia-ficticia.plbackup');
    const field = el.querySelector('#rec-pass-unlock') as HTMLInputElement;
    expect(field.type).toBe('password');
    expect(button(el, 'Desbloquear')?.disabled).toBe(true);
    field.value = 'frase de la copia';
    field.dispatchEvent(new Event('input'));
    await settle(fixture);
    button(el, 'Desbloquear')!.click();
    await settle(fixture);
    await settle(fixture);
    await settle(fixture);

    expect(calls.find((c) => c.command === 'recovery_unlock')?.args).toEqual({
      passphrase: 'frase de la copia',
    });
    expect(el.querySelector('app-recovery')).toBeNull();
    expect(el.querySelector('nav')).not.toBeNull();
  });

  it('shows the error and stays in recovery when the passphrase is wrong', async () => {
    failWith = { code: 'wrong_passphrase', message: 'La frase de paso no es correcta o el archivo está dañado.' };
    const fixture = TestBed.createComponent(Recovery);
    await settle(fixture);
    const el = fixture.nativeElement as HTMLElement;
    button(el, 'Elegir archivo de copia…')!.click();
    await settle(fixture);
    const field = el.querySelector('#rec-pass-unlock') as HTMLInputElement;
    field.value = 'mala';
    field.dispatchEvent(new Event('input'));
    await settle(fixture);
    button(el, 'Desbloquear')!.click();
    await settle(fixture);
    await settle(fixture);
    expect(el.querySelector('[role="alert"]')?.textContent).toContain('no es correcta');
    expect((el.querySelector('#rec-pass-unlock') as HTMLInputElement).value).toBe('');
  });
});
