import { TestBed } from '@angular/core/testing';
import { AccountView } from '../bank/bank.models';
import { fakeBackend } from '../ledger/testing';
import { ImportScreen } from './import';
import {
  ImportBatch,
  ImportPreview,
  ImportProfile,
  PickedImport,
  ProfileView,
  RawTable,
} from './import.models';

const OPENBANK: ImportProfile = {
  format: { type: 'Excel', sheet: null },
  skip_rows: null,
  has_header: true,
  columns: {
    booking_date: { Name: 'Fecha de operación' },
    value_date: { Name: 'Fecha valor' },
    description: { Name: 'Concepto' },
    counterparty: null,
    remittance: null,
    entry_reference: null,
    amount: { Single: { column: { Name: 'Importe' } } },
  },
  date_formats: ['%d/%m/%Y'],
  decimal: { decimal: ',', thousands: '.' },
  currency: 'EUR',
};

const GENERIC: ImportProfile = {
  format: { type: 'Csv', delimiter: ';', encoding: 'Windows1252', quote: '"' },
  skip_rows: null,
  has_header: true,
  columns: {
    booking_date: { Name: 'Fecha' },
    value_date: { Name: 'Fecha valor' },
    description: { Name: 'Concepto' },
    counterparty: null,
    remittance: null,
    entry_reference: null,
    amount: { Single: { column: { Name: 'Importe' } } },
  },
  date_formats: ['%d/%m/%Y'],
  decimal: { decimal: ',', thousands: '.' },
  currency: 'EUR',
};

const PROFILES: ProfileView[] = [
  { name: 'CSV genérico (es-ES)', builtin: true, profile: GENERIC },
  { name: 'Openbank (Excel)', builtin: true, profile: OPENBANK },
];

const detectedFile: PickedImport = {
  fileName: 'extracto-demo.xls',
  sizeBytes: 4096,
  detectedProfile: 'Openbank (Excel)',
  suggestedFormat: { type: 'Excel', sheet: null },
  profiles: PROFILES,
};

const unknownFile: PickedImport = {
  fileName: 'movimientos-demo.csv',
  sizeBytes: 900,
  detectedProfile: null,
  suggestedFormat: { type: 'Csv', delimiter: ';', encoding: 'Utf8', quote: '"' },
  profiles: PROFILES,
};

const RAW: RawTable = {
  rows: [
    ['Fecha', 'Descripción', 'Cargo', 'Abono'],
    ['03/10/2026', 'MERCADO DEMO', '42,18', ''],
  ],
  rowNumbers: [1, 2],
  totalRows: 2,
  columnCount: 4,
};

const ACCOUNTS: AccountView[] = [
  { id: 1, name: 'Cuenta Demo', kind: 'checking', display_suffix: '0001', balance_cents: null, currency: 'EUR', last_synced_at: null },
  { id: 2, name: 'Ahorro Demo', kind: 'savings', display_suffix: null, balance_cents: null, currency: 'EUR', last_synced_at: null },
];

function preview(extra: Partial<ImportPreview> = {}): ImportPreview {
  return {
    totalRows: 4,
    validRows: 3,
    parseErrors: [{ rowNumber: 9, reason: { kind: 'invalid_amount', column: 'Importe', problem: 'malformed' } }],
    parseErrorCount: 1,
    rows: [
      { rowNumber: 3, date: '2026-10-31', description: 'NOMINA DEMO', amountCents: 215000, currency: 'EUR', status: 'new' },
      { rowNumber: 4, date: '2026-10-30', description: 'MERCADO DEMO', amountCents: -6432, currency: 'EUR', status: 'new' },
      { rowNumber: 5, date: '2026-10-29', description: 'LUZ DEMO', amountCents: -4890, currency: 'EUR', status: 'duplicate' },
    ],
    dateFrom: '2026-10-29',
    dateTo: '2026-10-31',
    hasIdentity: true,
    matchedAccountId: 1,
    identity: null,
    counts: null,
    ...extra,
  };
}

const COUNTS = { wouldInsert: 2, duplicates: 1, updated: 0, transfersMatched: 0, categorized: 1, rejected: 0 };

function batch(extra: Partial<ImportBatch> = {}): ImportBatch {
  return {
    id: 7,
    accountId: 1,
    accountName: 'Cuenta Demo',
    profile: 'Openbank (Excel)',
    fileName: 'extracto-octubre.xls',
    importedAt: '2026-11-16T10:12:00',
    rowsTotal: 42,
    inserted: 38,
    updated: 0,
    duplicates: 3,
    errors: 1,
    undoneAt: null,
    canUndo: true,
    ...extra,
  };
}

describe('Importar screen', () => {
  let file: PickedImport | null;
  let history: ImportBatch[];
  let previewFor: (accountId: number | null) => ImportPreview;
  let backend: ReturnType<typeof fakeBackend>;

  function setup() {
    backend = fakeBackend({
      import_history: () => history,
      import_pick_file: () => file,
      import_raw_rows: () => RAW,
      import_accounts: () => ACCOUNTS,
      import_preview: (a) => previewFor((a['accountId'] as number | null) ?? null),
      import_commit: (a) => ({
        batchId: 9, accountId: a['accountId'], inserted: 2, duplicates: 1, updated: 0, invalidRows: 1,
        transfersMatched: 0, categorized: 1, profileSaved: a['saveProfileAs'] !== null,
      }),
      import_cancel: () => null,
      import_undo: () => ({ deleted: 38, transfersUnlinked: 0 }),
      ledger_create_account: () => ACCOUNTS[0],
    });
    TestBed.configureTestingModule({ providers: backend.providers });
  }

  async function render() {
    const fixture = TestBed.createComponent(ImportScreen);
    const settle = async () => {
      for (let i = 0; i < 3; i++) {
        await fixture.whenStable();
        fixture.detectChanges();
      }
    };
    await settle();
    return { fixture, el: fixture.nativeElement as HTMLElement, settle };
  }

  const btn = (el: HTMLElement, text: string) =>
    Array.from(el.querySelectorAll('button, a.btn')).find((b) => b.textContent?.trim().includes(text)) as
      | HTMLButtonElement
      | undefined;
  const test = (el: HTMLElement, id: string) => el.querySelector(`[data-test="${id}"]`) as HTMLElement;
  const choose = (el: HTMLElement, id: string, value: string) => {
    const s = el.querySelector(`#${id}`) as HTMLSelectElement;
    s.value = value;
    s.dispatchEvent(new Event('change'));
  };

  beforeEach(() => {
    file = detectedFile;
    history = [];
    previewFor = (id) => preview(id === null ? {} : { identity: { status: 'match' }, counts: COUNTS });
    setup();
  });

  it('lists the history with Deshacer only when the batch can be undone', async () => {
    history = [
      batch(),
      batch({ id: 6, canUndo: false, inserted: 0, duplicates: 44, errors: 0, fileName: 'sep.xls' }),
      batch({ id: 5, canUndo: false, undoneAt: '2026-10-01T21:17:00', fileName: 'ahorro.csv' }),
    ];
    const { el } = await render();
    const text = el.querySelector('[data-test="history"]')!.textContent!;
    expect(text).toContain('38 nuevos · 3 duplicados');
    expect(text).toContain('1 fila no válida');
    expect(text).toContain('Sin cambios · 44 duplicados');
    expect(text).toContain('Deshecha el 01/10/2026');
    expect(Array.from(el.querySelectorAll('tbody button')).length).toBe(1);
  });

  it('takes a recognised file from the picker to a committed import', async () => {
    const { el, settle } = await render();
    btn(el, 'Elegir archivo')!.click();
    await settle();
    expect(el.textContent).toContain('Formato reconocido: Openbank (Excel).');
    expect(el.textContent).toContain('Fechas dd/mm/aaaa, importe con signo, cabecera detectada automáticamente.');
    expect(el.querySelector('.steps [aria-current="step"]')?.textContent).toBe('Formato');

    test(el, 'continue').click();
    await settle();
    // The account matching the file's IBAN is preselected and checked.
    expect(backend.of('import_preview').map((c) => c.args?.['accountId'])).toEqual([null, 1]);
    expect(test(el, 'match').textContent).toContain('El IBAN del archivo coincide con esta cuenta.');
    expect(test(el, 'match').textContent).toContain('Cuenta Demo (•••• 0001)');

    test(el, 'to-review').click();
    await settle();
    expect(test(el, 'kpi-new').textContent).toBe('2');
    expect(test(el, 'kpi-bad').textContent).toBe('1');
    expect(test(el, 'errors').textContent).toContain('Importe no válido en la columna «Importe»: formato no reconocido.');
    expect(test(el, 'rows').textContent).toContain('+2.150,00 €');
    expect(test(el, 'rows').textContent).toContain('−64,32 €');
    expect(test(el, 'commit').textContent).toContain('Importar 2 movimientos');

    test(el, 'commit').click();
    await settle();
    const commit = backend.of('import_commit')[0].args!;
    expect(commit).toEqual({
      profile: OPENBANK,
      profileName: 'Openbank (Excel)',
      accountId: 1,
      saveProfileAs: null,
    });
    expect(test(el, 'result').textContent).toContain('Cuenta Demo');
    expect(el.textContent).toContain('Importación completada');
    expect(btn(el, 'Ver movimientos')!.getAttribute('href')).toBe('/movimientos');
  });

  it('builds the profile from the generic mapping and saves it under the given name', async () => {
    file = unknownFile;
    previewFor = (id) => preview({ matchedAccountId: null, ...(id === null ? {} : { identity: { status: 'no_identity' }, counts: COUNTS }) });
    const { el, settle } = await render();
    btn(el, 'Elegir archivo')!.click();
    await settle();

    expect(el.textContent).toContain('No reconocemos este formato');
    expect(backend.of('import_raw_rows')[0].args).toEqual({ format: unknownFile.suggestedFormat, maxRows: 40 });
    // The generic prefill names columns the file lacks ("Concepto"): they are left to the user.
    choose(el, 'm-c-description', '1');
    const split = Array.from(el.querySelectorAll('input[name="amt"]'))[1] as HTMLInputElement;
    split.click();
    await settle();
    choose(el, 'm-c-debit', '2');
    choose(el, 'm-c-credit', '3');
    choose(el, 'm-date', '%d-%m-%Y');
    (Array.from(el.querySelectorAll('input[name="dec"]'))[1] as HTMLInputElement).click();
    const save = el.querySelector('#m-save') as HTMLInputElement;
    save.click();
    const name = el.querySelector('#m-name') as HTMLInputElement;
    name.value = 'Tarjeta Demo';
    name.dispatchEvent(new Event('input'));
    await settle();

    test(el, 'continue').click();
    await settle();
    const profile = backend.of('import_preview')[0].args!['profile'] as ImportProfile;
    expect(profile.format).toEqual(unknownFile.suggestedFormat);
    expect(profile.has_header).toBe(true);
    expect(profile.columns.booking_date).toEqual({ Name: 'Fecha' });
    expect(profile.columns.description).toEqual({ Name: 'Descripción' });
    expect(profile.columns.value_date).toBeNull();
    expect(profile.columns.amount).toEqual({ DebitCredit: { debit: { Name: 'Cargo' }, credit: { Name: 'Abono' } } });
    expect(profile.date_formats).toEqual(['%d-%m-%Y']);
    expect(profile.decimal).toEqual({ decimal: '.', thousands: ',' });

    choose(el, 'a-acc', '2');
    await settle();
    test(el, 'to-review').click();
    await settle();
    test(el, 'commit').click();
    await settle();
    const commit = backend.of('import_commit')[0].args!;
    expect(commit['accountId']).toBe(2);
    expect(commit['profileName']).toBe('Tarjeta Demo');
    expect(commit['saveProfileAs']).toBe('Tarjeta Demo');
  });

  it('shows the belongs-to-other banner, blocks Continue and switches account', async () => {
    previewFor = (id) =>
      id === 2
        ? preview({ identity: { status: 'belongs_to_other', existingAccountId: 1 }, counts: null })
        : preview(id === null ? { matchedAccountId: 2 } : { identity: { status: 'match' }, counts: COUNTS });
    const { el, settle } = await render();
    btn(el, 'Elegir archivo')!.click();
    await settle();
    test(el, 'continue').click();
    await settle();

    expect(test(el, 'other').textContent).toContain('Este extracto pertenece a «Cuenta Demo».');
    expect((test(el, 'to-review') as HTMLButtonElement).disabled).toBe(true);

    btn(el, 'Usar Cuenta Demo')!.click();
    await settle();
    expect(el.querySelector('[data-test="other"]')).toBeNull();
    expect((el.querySelector('#a-acc') as HTMLSelectElement).value).toBe('1');
    expect((test(el, 'to-review') as HTMLButtonElement).disabled).toBe(false);
  });

  it('undoes a batch from the history only after confirming', async () => {
    history = [batch()];
    const { el, settle } = await render();
    btn(el, 'Deshacer')!.click();
    await settle();
    expect(el.querySelector('[role="dialog"]')!.textContent).toContain(
      '¿Deshacer la importación de extracto-octubre.xls?',
    );
    expect(el.querySelector('[role="dialog"]')!.textContent).toContain('los 38 movimientos que añadió en Cuenta Demo');
    expect(backend.of('import_undo').length).toBe(0);

    test(el, 'undo-confirm').click();
    await settle();
    expect(backend.of('import_undo')[0].args).toEqual({ batchId: 7 });
    expect(el.querySelector('[role="dialog"]')).toBeNull();
    expect(el.textContent).toContain('Importación deshecha: se eliminaron 38 movimientos.');
  });

  it('cancels the stashed file and returns to the start', async () => {
    const { el, settle } = await render();
    btn(el, 'Elegir archivo')!.click();
    await settle();
    expect(el.querySelector('.steps')).not.toBeNull();
    btn(el, 'Cancelar')!.click();
    await settle();
    expect(backend.of('import_cancel').length).toBe(1);
    expect(el.querySelector('.steps')).toBeNull();
    expect(btn(el, 'Elegir archivo')).toBeDefined();
  });

  it('keeps the start view when the dialog is cancelled', async () => {
    file = null;
    const { el, settle } = await render();
    btn(el, 'Elegir archivo')!.click();
    await settle();
    expect(el.querySelector('.steps')).toBeNull();
  });
});
