import { TestBed } from '@angular/core/testing';
import { TAURI_INVOKE } from '../core/tauri';
import { BankKey } from './bank-key';
import { BankKeyStatus } from './bank-key.service';

const NONE: BankKeyStatus = { configured: false, source: 'none', file_name: null, exists: false };

describe('BankKey settings', () => {
  let calls: string[];
  let status: BankKeyStatus;
  let pickResult: BankKeyStatus;
  let failWith: { code: string; message: string } | null;

  async function render() {
    const fixture = TestBed.createComponent(BankKey);
    await fixture.whenStable();
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }

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
    status = NONE;
    pickResult = { configured: true, source: 'settings', file_name: 'clave.pem', exists: true };
    failWith = null;
    TestBed.configureTestingModule({
      providers: [
        {
          provide: TAURI_INVOKE,
          useValue: async (command: string) => {
            calls.push(command);
            if (failWith) throw failWith;
            switch (command) {
              case 'bank_key_status':
                return status;
              case 'bank_key_pick':
                status = pickResult;
                return status;
              case 'bank_key_clear':
                status = NONE;
                return status;
              default:
                throw new Error(`unexpected ${command}`);
            }
          },
        },
      ],
    });
  });

  it('shows "No configurada" and no Quitar button when there is no key', async () => {
    const { el } = await render();
    expect(el.textContent).toContain('Conexión bancaria');
    expect(el.querySelector('[data-test="key-status"]')?.textContent).toContain('No configurada');
    expect(button(el, 'Elegir archivo .pem…')).toBeDefined();
    expect(button(el, 'Quitar')).toBeUndefined();
  });

  it('picks a file, shows only its name and offers Quitar', async () => {
    const { fixture, el } = await render();
    button(el, 'Elegir archivo .pem…')!.click();
    await settle(fixture);
    expect(calls).toContain('bank_key_pick');
    expect(el.querySelector('[data-test="key-status"]')?.textContent).toContain(
      'Configurada: clave.pem',
    );
    expect(el.querySelector('[data-test="missing"]')).toBeNull();

    button(el, 'Quitar')!.click();
    await settle(fixture);
    expect(calls).toContain('bank_key_clear');
    expect(el.querySelector('[data-test="key-status"]')?.textContent).toContain('No configurada');
  });

  it('warns when the configured file no longer exists', async () => {
    status = { configured: true, source: 'settings', file_name: 'clave.pem', exists: false };
    const { el } = await render();
    expect(el.querySelector('[data-test="missing"]')?.textContent).toContain('No se encuentra');
  });

  it('explains the priority of the environment variable and hides Quitar', async () => {
    status = { configured: true, source: 'env', file_name: 'clave.pem', exists: true };
    const { el } = await render();
    expect(el.querySelector('[data-test="key-status"]')?.textContent).toContain(
      'variable de entorno',
    );
    expect(el.querySelector('[data-test="env-note"]')?.textContent).toContain('tiene prioridad');
    expect(button(el, 'Quitar')).toBeUndefined();
  });

  it('shows the error returned by the backend', async () => {
    failWith = { code: 'invalid_input', message: 'Los datos enviados no son válidos.' };
    const { fixture, el } = await render();
    button(el, 'Elegir archivo .pem…')!.click();
    await settle(fixture);
    expect(el.querySelector('[role="alert"]')?.textContent).toContain('no son válidos');
  });
});
