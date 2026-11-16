import { TestBed } from '@angular/core/testing';
import { ActivatedRoute, convertToParamMap } from '@angular/router';
import { of } from 'rxjs';
import { CategoryOption, TxPage, TxRow } from '../ledger/ledger.models';
import { fakeBackend } from '../ledger/testing';
import { Transactions, buildFilter } from './transactions';

function row(id: number, extra: Partial<TxRow> = {}): TxRow {
  return {
    id, accountId: 1, accountName: 'Cuenta de prueba', status: 'booked', date: '2026-11-14',
    bookingDate: '2026-11-14', valueDate: '2026-11-15', amountCents: -6318, currency: 'EUR',
    description: `Comercio ${id}`, counterparty: `Comercio ${id} S.A.`, remittance: null,
    categoryId: 11, categoryName: 'Supermercado', categoryKey: null, parentCategoryId: 10,
    parentCategoryName: 'Alimentación', categorySource: 'rule', isTransfer: false,
    transferPeerId: null, transferPeerAccountName: null, notes: null, tags: [], ...extra,
  };
}

const CATEGORIES: CategoryOption[] = [
  { id: 10, parentId: null, key: null, name: 'Alimentación', kind: 'expense' },
  { id: 11, parentId: 10, key: null, name: 'Supermercado', kind: 'expense' },
  { id: 12, parentId: 10, key: null, name: 'Restaurantes', kind: 'expense' },
  { id: 20, parentId: null, key: null, name: 'Transferencias internas', kind: 'transfer' },
];

function page(rows: TxRow[], extra: Partial<TxPage> = {}): TxPage {
  return {
    rows,
    total: rows.length,
    summary: { incomeCents: 2500, expenseCents: 11548, netCents: -9048, transferCount: 0, pendingCount: 0 },
    ...extra,
  };
}

describe('Transactions screen', () => {
  let rows: TxRow[];
  let pageExtra: Partial<TxPage>;
  let backend: ReturnType<typeof fakeBackend>;

  async function settle(fixture: { detectChanges(): void; whenStable(): Promise<unknown> }) {
    for (let i = 0; i < 3; i++) {
      fixture.detectChanges();
      await fixture.whenStable();
    }
  }
  async function render(queryParams?: Record<string, string>) {
    TestBed.configureTestingModule({
      providers: [
        ...backend.providers,
        ...(queryParams
          ? [{ provide: ActivatedRoute, useValue: { queryParamMap: of(convertToParamMap(queryParams)) } }]
          : []),
      ],
    });
    const fixture = TestBed.createComponent(Transactions);
    await settle(fixture);
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }
  const lastFilter = () => backend.of('ledger_transactions').at(-1)!.args!['filter'] as Record<string, unknown>;
  const text = (el: HTMLElement) => el.textContent!.replace(/\s+/g, ' ');
  const trs = (el: HTMLElement) => Array.from(el.querySelectorAll<HTMLElement>('.tbl tbody tr'));
  function setSelect(el: HTMLElement, sel: string, value: string) {
    const s = el.querySelector(sel) as HTMLSelectElement;
    s.value = value;
    s.dispatchEvent(new Event('change'));
  }
  function setInput(el: HTMLElement, sel: string, value: string, event = 'input') {
    const i = el.querySelector(sel) as HTMLInputElement | HTMLTextAreaElement;
    i.value = value;
    i.dispatchEvent(new Event(event));
  }
  const button = (el: HTMLElement, label: string) =>
    Array.from(el.querySelectorAll('button')).find((b) => b.textContent?.trim() === label) as HTMLButtonElement;

  beforeEach(() => {
    rows = [row(1, { status: 'pending', description: 'Gasolinera' }), row(2), row(3, { amountCents: 2500 })];
    pageExtra = {};
    backend = fakeBackend({
      ledger_transactions: () => page(rows, pageExtra),
      ledger_categories: () => CATEGORIES,
      ledger_tags: () => ['Coche', 'Hogar'],
      bank_overview: () => ({
        configured: true, aspsp_name: null, connection: { kind: 'not_connected' }, runs: [], next_auto_sync_at: null,
        accounts: [{ id: 1, name: 'Cuenta de prueba' }, { id: 2, name: 'Tarjeta de prueba' }],
      }),
      ledger_set_category: (a) => ({ row: row(a['txId'] as number, { categoryId: a['categoryId'] as number }), recategorized: 2, ruleId: a['createRule'] ? 7 : null }),
      ledger_set_notes: (a) => row(a['txId'] as number, { notes: a['notes'] as string | null }),
      ledger_set_tags: (a) => row(a['txId'] as number, { tags: a['tags'] as string[] }),
      ledger_unlink_transfer: () => [],
    });
  });

  it('loads the current month and renders rows, signed amounts and badges', async () => {
    const { el } = await render();
    expect(lastFilter()).toEqual({ from: '2026-11-01', to: '2026-11-30', offset: 0, limit: 100 });
    expect(trs(el)).toHaveLength(3);
    expect(el.querySelector('.month-label')!.textContent).toBe('noviembre 2026');
    const t = text(el);
    expect(t).toContain('14/11/2026');
    expect(t).toContain('−63,18 €');
    expect(t).toContain('+25,00 €');
    expect(trs(el)[0].textContent).toContain('Pendiente');
    expect(t).toContain('Mostrando 3 de 3 movimientos');
    expect(t).not.toContain('Exportar');
    expect(text(el.querySelector('.totals')!)).toContain('+25,00 €');
    expect(text(el.querySelector('.totals')!)).toContain('−115,48 €');
    expect(text(el.querySelector('.totals')!)).toContain('−90,48 €');
  });

  it('writes singular and plural summary notes', async () => {
    pageExtra = { total: 1, summary: { incomeCents: 0, expenseCents: 100, netCents: -100, transferCount: 1, pendingCount: 1 } };
    const one = await render();
    expect(text(one.el.querySelector('.t-note') as HTMLElement)).toBe('1 movimiento · 1 transferencia interna excluida · incluye 1 pendiente');
    TestBed.resetTestingModule();
    pageExtra = { total: 25, summary: { incomeCents: 0, expenseCents: 100, netCents: -100, transferCount: 2, pendingCount: 3 } };
    backend = fakeBackend({ ledger_transactions: () => page(rows, pageExtra), ledger_categories: () => CATEGORIES, ledger_tags: () => [] });
    const many = await render();
    expect(text(many.el.querySelector('.t-note') as HTMLElement)).toBe('25 movimientos · 2 transferencias internas excluidas · incluye 3 pendientes');
  });

  it('sends the right filter when controls change', async () => {
    const { fixture, el } = await render();
    setSelect(el, '#f-acct', '2');
    await settle(fixture);
    expect(lastFilter()).toMatchObject({ accountId: 2, from: '2026-11-01' });

    setSelect(el, '#f-cat', '11');
    await settle(fixture);
    expect(lastFilter()).toMatchObject({ accountId: 2, categoryId: 11 });

    setSelect(el, '#f-cat', 'none');
    await settle(fixture);
    expect(lastFilter()['uncategorized']).toBe(true);
    expect(lastFilter()['categoryId']).toBeUndefined();

    (el.querySelector('input[value="out"]') as HTMLInputElement).click();
    await settle(fixture);
    expect(lastFilter()['direction']).toBe('out');

    (el.querySelector('.pending-check input') as HTMLInputElement).click();
    await settle(fixture);
    expect(lastFilter()['status']).toBe('pending');

    setInput(el, '#f-from', '2026-10-05', 'change');
    await settle(fixture);
    expect(lastFilter()).toMatchObject({ from: '2026-10-05' });
    expect(lastFilter()['to']).toBeUndefined();
  });

  it('debounces the text search', async () => {
    const { fixture, el } = await render();
    const before = backend.of('ledger_transactions').length;
    setInput(el, '#f-q', 'merca');
    setInput(el, '#f-q', 'mercadona');
    await settle(fixture);
    expect(backend.of('ledger_transactions').length).toBe(before);
    await new Promise((r) => setTimeout(r, 350));
    await settle(fixture);
    expect(backend.of('ledger_transactions').length).toBe(before + 1);
    expect(lastFilter()['text']).toBe('mercadona');
  });

  it('reads q and month from the query string', async () => {
    const { el } = await render({ q: 'mercadona', month: '2026-10' });
    expect(lastFilter()).toMatchObject({ text: 'mercadona', from: '2026-10-01', to: '2026-10-31' });
    expect((el.querySelector('#f-q') as HTMLInputElement).value).toBe('mercadona');
  });

  it('searches the whole history when only q is given', async () => {
    const { el } = await render({ q: 'netflix' });
    expect(lastFilter()['from']).toBeUndefined();
    expect(el.querySelector('.month-label')!.textContent).toBe('Todo el historial');
  });

  it('changes month with the selector', async () => {
    const { fixture, el } = await render();
    (el.querySelector('[aria-label="Mes anterior"]') as HTMLButtonElement).click();
    await settle(fixture);
    expect(lastFilter()).toMatchObject({ from: '2026-10-01', to: '2026-10-31' });
  });

  it('selects with the keyboard and saves a category with a rule', async () => {
    const { fixture, el } = await render();
    const first = trs(el)[0];
    first.focus();
    first.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    expect(document.activeElement).toBe(trs(el)[1]);
    trs(el)[1].dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle(fixture);
    expect(trs(el)[1].getAttribute('aria-current')).toBe('true');
    expect(el.querySelector('#d-title')!.textContent).toBe('Comercio 2');
    expect(button(el, 'Guardar cambios').disabled).toBe(true);

    setSelect(el, '#d-cat', '12');
    (el.querySelector('#d-rule') as HTMLInputElement).click();
    await settle(fixture);
    expect(text(el.querySelector('#d-rule-hint') as HTMLElement)).toBe('Si la descripción contiene «Comercio 2 S.A.» → Restaurantes');
    button(el, 'Guardar cambios').click();
    await settle(fixture);

    expect(backend.of('ledger_set_category')[0].args).toEqual({ txId: 2, categoryId: 12, createRule: true });
    expect(backend.of('ledger_set_notes')).toHaveLength(0);
    expect(backend.of('ledger_set_tags')).toHaveLength(0);
    expect(el.querySelector('[role="status"]')!.textContent).toContain('Regla creada; se recategorizaron 2 movimientos');
  });

  it('shows the unlink action and a disabled category for transfers', async () => {
    rows = [row(5, { isTransfer: true, categoryId: 20, categoryName: 'Transferencias internas', parentCategoryId: null, amountCents: -50000, transferPeerAccountName: 'Tarjeta de prueba' })];
    const { fixture, el } = await render();
    expect(trs(el)[0].textContent).toContain('⇄');
    expect(trs(el)[0].querySelector('.amt-muted')).not.toBeNull();
    trs(el)[0].click();
    await settle(fixture);
    expect((el.querySelector('#d-cat') as HTMLSelectElement).disabled).toBe(true);
    expect((el.querySelector('#d-rule') as HTMLInputElement).disabled).toBe(true);
    button(el, 'Deshacer transferencia interna').click();
    await settle(fixture);
    expect(backend.of('ledger_unlink_transfer')[0].args).toEqual({ txId: 5 });
    expect(backend.of('ledger_set_category')).toHaveLength(0);
  });

  it('saves only the changed notes and tags', async () => {
    rows = [row(1, { tags: ['Coche'], notes: 'vieja' })];
    const { fixture, el } = await render();
    trs(el)[0].click();
    await settle(fixture);
    setInput(el, '#d-note', ' nueva nota ');
    const tag = el.querySelector('#d-taginput') as HTMLInputElement;
    tag.value = 'Hogar';
    tag.dispatchEvent(new Event('input'));
    tag.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle(fixture);
    (el.querySelector('[aria-label="Quitar etiqueta Coche"]') as HTMLButtonElement).click();
    await settle(fixture);
    button(el, 'Guardar cambios').click();
    await settle(fixture);
    expect(backend.of('ledger_set_notes')[0].args).toEqual({ txId: 1, notes: 'nueva nota' });
    expect(backend.of('ledger_set_tags')[0].args).toEqual({ txId: 1, tags: ['Hogar'] });
    expect(backend.of('ledger_set_category')).toHaveLength(0);
    expect(text(el)).toContain('Cambios guardados.');
  });

  it('discards the draft', async () => {
    const { fixture, el } = await render();
    trs(el)[0].click();
    await settle(fixture);
    setInput(el, '#d-note', 'algo');
    await settle(fixture);
    expect(button(el, 'Guardar cambios').disabled).toBe(false);
    button(el, 'Descartar').click();
    await settle(fixture);
    expect((el.querySelector('#d-note') as HTMLTextAreaElement).value).toBe('');
    expect(button(el, 'Guardar cambios').disabled).toBe(true);
  });

  it('loads more rows with the next offset', async () => {
    pageExtra = { total: 5 };
    const { fixture, el } = await render();
    expect(trs(el)).toHaveLength(3);
    rows = [row(4), row(5)];
    button(el, 'Cargar más').click();
    await settle(fixture);
    expect(backend.of('ledger_transactions').at(-1)!.args!['filter']).toMatchObject({ offset: 3, limit: 100 });
    expect(trs(el)).toHaveLength(5);
  });

  it('shows an empty state with a way to clear filters', async () => {
    rows = [];
    const { fixture, el } = await render();
    expect(el.querySelector('.empty')!.textContent).toContain('No hay movimientos');
    expect(button(el, 'Quitar filtros')).toBeUndefined();
    (el.querySelector('.pending-check input') as HTMLInputElement).click();
    await settle(fixture);
    expect(button(el, 'Quitar filtros')).toBeTruthy();
  });
});

describe('buildFilter', () => {
  const base = { month: '2026-11', dateFrom: '', dateTo: '', q: '', accountId: null, category: '', direction: 'all' as const, pendingOnly: false };

  it('lets a date range override the month', () => {
    expect(buildFilter({ ...base, dateFrom: '2026-10-05', dateTo: '2026-10-20' })).toEqual({ from: '2026-10-05', to: '2026-10-20' });
  });

  it('leaves dates open without month or range, and trims text', () => {
    expect(buildFilter({ ...base, month: null, q: '  ikea ' })).toEqual({ text: 'ikea' });
  });
});
