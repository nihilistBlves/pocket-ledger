import { TestBed } from '@angular/core/testing';
import { TAURI_INVOKE } from '../core/tauri';
import { Backups, CHECK_DEBOUNCE_MS } from './backups';

const STRONG = 'violin-marmalade-orbit-glacier-seven';

interface Call {
  command: string;
  args?: Record<string, unknown>;
}

describe('Backups settings', () => {
  let calls: Call[];
  let status: { last_backup_at: string | null; last_file_name: string | null; reminder: boolean };
  let createResult: { created_at: string; file_name: string; size: number } | null;
  let failWith: { code: string; message: string } | null;
  let pickResult: { file_name: string } | null;

  async function render() {
    const fixture = TestBed.createComponent(Backups);
    await fixture.whenStable();
    fixture.detectChanges();
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }

  const settle = async (fixture: { whenStable(): Promise<unknown>; detectChanges(): void }) => {
    await fixture.whenStable();
    fixture.detectChanges();
  };

  const input = (el: HTMLElement, id: string) => el.querySelector(`#${id}`) as HTMLInputElement;

  const type = (el: HTMLElement, id: string, value: string) => {
    const field = input(el, id);
    field.value = value;
    field.dispatchEvent(new Event('input'));
  };

  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

  const button = (el: HTMLElement, text: string) =>
    Array.from(el.querySelectorAll('button')).find((b) => b.textContent?.trim().includes(text)) as
      | HTMLButtonElement
      | undefined;

  /** Types the passphrase, waits for the debounced strength check. */
  async function fillCreate(
    fixture: { whenStable(): Promise<unknown>; detectChanges(): void },
    el: HTMLElement,
    pass: string,
    confirmation: string,
  ) {
    type(el, 'bk-pass', pass);
    type(el, 'bk-conf', confirmation);
    await wait(CHECK_DEBOUNCE_MS + 60);
    await settle(fixture);
  }

  beforeEach(() => {
    calls = [];
    failWith = null;
    pickResult = { file_name: 'pocket-ledger-2026-11-15-1030.plbackup' };
    status = { last_backup_at: null, last_file_name: null, reminder: true };
    createResult = { created_at: '2026-11-16T09:30:00.000Z', file_name: 'copia.plbackup', size: 1234 };
    TestBed.configureTestingModule({
      providers: [
        {
          provide: TAURI_INVOKE,
          useValue: async (command: string, args?: Record<string, unknown>) => {
            calls.push({ command, args });
            switch (command) {
              case 'backup_status':
                return structuredClone(status);
              case 'backup_check_passphrase': {
                const p = String(args?.['passphrase']);
                return p === STRONG
                  ? { ok: true, score: 4, feedback: null }
                  : { ok: false, score: 1, feedback: 'Usa al menos 12 caracteres.' };
              }
              case 'backup_create':
                if (failWith) throw failWith;
                if (createResult) {
                  status = {
                    last_backup_at: createResult.created_at,
                    last_file_name: createResult.file_name,
                    reminder: false,
                  };
                }
                return createResult;
              case 'backup_pick_file':
                return pickResult;
              case 'backup_restore':
                if (failWith) throw failWith;
                return {
                  backup_created_at: '2026-11-15T10:30:00.000Z',
                  pre_restore_file_name: 'pocket-ledger.db.pre-restore-20261116T093000Z',
                };
              default:
                throw new Error(`unexpected ${command}`);
            }
          },
        },
      ],
    });
  });

  it('shows no backup yet with the reminder and the mandatory warning', async () => {
    const { el } = await render();
    expect(el.querySelector('h1')?.textContent).toBe('Ajustes');
    expect(el.textContent).toContain('Copias de seguridad');
    expect(el.querySelector('[data-test="last-backup"]')?.textContent).toContain(
      'Aún no has hecho ninguna copia',
    );
    expect(el.querySelector('.reminder')).not.toBeNull();
    expect(el.querySelector('[data-test="warning"]')?.textContent).toContain(
      'Si olvidas la frase de paso, la copia no se podrá recuperar. Nadie puede restablecerla.',
    );
    expect(input(el, 'bk-pass').type).toBe('password');
    expect(input(el, 'bk-pass').getAttribute('autocomplete')).toBe('new-password');
    expect(input(el, 'bk-conf').type).toBe('password');
    expect(button(el, 'Crear copia')?.disabled).toBe(true);
  });

  it('shows the last backup date in es-ES and no reminder when it is recent', async () => {
    status = { last_backup_at: '2026-11-15T10:30:00.000Z', last_file_name: 'a.plbackup', reminder: false };
    const { el } = await render();
    const text = el.querySelector('[data-test="last-backup"]')?.textContent ?? '';
    expect(text).toContain('Última copia: 15/11/2026');
    expect(text).toContain('a.plbackup');
    expect(el.querySelector('.reminder')).toBeNull();
  });

  it('shows weak feedback and keeps the button disabled', async () => {
    const { fixture, el } = await render();
    await fillCreate(fixture, el, 'corta', 'corta');
    expect(el.querySelector('[data-test="feedback"]')?.textContent).toContain('al menos 12');
    expect(el.querySelector('[role="meter"]')?.getAttribute('aria-valuenow')).toBe('1');
    expect(button(el, 'Crear copia')?.disabled).toBe(true);
  });

  it('disables the button when the confirmation does not match', async () => {
    const { fixture, el } = await render();
    await fillCreate(fixture, el, STRONG, `${STRONG}x`);
    expect(el.querySelector('[data-test="mismatch"]')).not.toBeNull();
    expect(button(el, 'Crear copia')?.disabled).toBe(true);
    // Nothing was sent to create.
    expect(calls.some((c) => c.command === 'backup_create')).toBe(false);
  });

  it('debounces the strength check', async () => {
    const { el } = await render();
    for (const v of ['a', 'ab', 'abc']) type(el, 'bk-pass', v);
    await wait(CHECK_DEBOUNCE_MS + 60);
    expect(calls.filter((c) => c.command === 'backup_check_passphrase')).toHaveLength(1);
  });

  it('creates a backup, shows the file name and clears the fields', async () => {
    const { fixture, el } = await render();
    await fillCreate(fixture, el, STRONG, STRONG);
    const create = button(el, 'Crear copia')!;
    expect(create.disabled).toBe(false);
    create.click();
    await settle(fixture);
    await settle(fixture);

    const call = calls.find((c) => c.command === 'backup_create');
    expect(call?.args).toEqual({ passphrase: STRONG, confirmation: STRONG });
    expect(el.textContent).toContain('Copia creada y verificada: copia.plbackup');
    expect(input(el, 'bk-pass').value).toBe('');
    expect(input(el, 'bk-conf').value).toBe('');
    expect(el.querySelector('[data-test="last-backup"]')?.textContent).toContain(
      'Última copia: 16/11/2026',
    );
    expect(el.querySelector('.reminder')).toBeNull();
    expect(button(el, 'Crear copia')?.disabled).toBe(true);
  });

  it('shows the error and clears the fields when creating fails', async () => {
    failWith = { code: 'busy', message: 'Ya hay otra operación bancaria en curso. Espera a que termine.' };
    const { fixture, el } = await render();
    await fillCreate(fixture, el, STRONG, STRONG);
    button(el, 'Crear copia')!.click();
    await settle(fixture);
    await settle(fixture);
    expect(el.querySelector('[role="alert"]')?.textContent).toContain('otra operación bancaria');
    expect(input(el, 'bk-pass').value).toBe('');
  });

  it('keeps the form when the Save dialog is cancelled', async () => {
    createResult = null;
    const { fixture, el } = await render();
    await fillCreate(fixture, el, STRONG, STRONG);
    button(el, 'Crear copia')!.click();
    await settle(fixture);
    expect(input(el, 'bk-pass').value).toBe(STRONG);
    expect(el.textContent).not.toContain('Copia creada');
  });

  it('restoring needs the file, the passphrase and an explicit confirmation', async () => {
    const { fixture, el } = await render();
    button(el, 'Restaurar una copia…')!.click();
    await settle(fixture);
    expect(el.querySelector('[data-test="picked"]')?.textContent).toContain(
      'pocket-ledger-2026-11-15-1030.plbackup',
    );
    expect(button(el, 'Restaurar')?.disabled).toBe(true);

    type(el, 'bk-rpass', 'frase de la copia');
    await settle(fixture);
    const restore = button(el, 'Restaurar')!;
    expect(restore.disabled).toBe(false);
    restore.click();
    await settle(fixture);

    // The confirmation explains the consequences; nothing was restored yet.
    expect(calls.some((c) => c.command === 'backup_restore')).toBe(false);
    const confirm = el.querySelector('[role="alertdialog"]')!;
    expect(confirm.textContent).toContain('Se sustituirán todos los datos actuales');
    expect(confirm.textContent).toContain('Se conserva una copia de la base de datos actual');
    expect(confirm.textContent).toContain('volver a conectar el banco si restauras en otro equipo');

    // Cancel goes back without restoring.
    button(el, 'Cancelar')!.click();
    await settle(fixture);
    expect(el.querySelector('[role="alertdialog"]')).toBeNull();
    expect(calls.some((c) => c.command === 'backup_restore')).toBe(false);

    button(el, 'Restaurar')!.click();
    await settle(fixture);
    button(el, 'Sí, reemplazar')!.click();
    await settle(fixture);
    await settle(fixture);

    const call = calls.find((c) => c.command === 'backup_restore');
    expect(call?.args).toEqual({ passphrase: 'frase de la copia' });
    expect(el.textContent).toContain('Copia restaurada (creada el 15/11/2026).');
    expect(el.textContent).toContain('pocket-ledger.db.pre-restore-20261116T093000Z');
    expect(el.querySelector('#bk-rpass')).toBeNull();
  });

  it('shows the restore error returned by the backend', async () => {
    failWith = { code: 'wrong_passphrase', message: 'La frase de paso no es correcta o el archivo está dañado.' };
    const { fixture, el } = await render();
    button(el, 'Restaurar una copia…')!.click();
    await settle(fixture);
    type(el, 'bk-rpass', 'otra frase');
    await settle(fixture);
    button(el, 'Restaurar')!.click();
    await settle(fixture);
    button(el, 'Sí, reemplazar')!.click();
    await settle(fixture);
    await settle(fixture);
    expect(el.querySelector('[role="alert"]')?.textContent).toContain('no es correcta');
    expect(input(el, 'bk-rpass').value).toBe('');
  });
});
