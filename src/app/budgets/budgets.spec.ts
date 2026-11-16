import { TestBed } from '@angular/core/testing';
import { BudgetMonth, BudgetRow, CategoryOption } from '../ledger/ledger.models';
import { fakeBackend } from '../ledger/testing';
import { Budgets } from './budgets';

function brow(id: number, extra: Partial<BudgetRow>): BudgetRow {
  return {
    categoryId: id, name: `Cat ${id}`, parentId: null, parentName: null, isGroup: false, childNames: [], color: null,
    amountCents: 10000, spentCents: 0, remainingCents: 10000, percentBp: 0, status: 'ok', avg3Cents: 0,
    startMonth: '2026-01', previousAmountCents: null, ...extra,
  };
}

function month(extra: Partial<BudgetMonth> = {}): BudgetMonth {
  return {
    month: '2026-11', isCurrentMonth: true, dayOfMonth: 16, daysInMonth: 30,
    totals: { budgetCents: 70000, spentCents: 61995, remainingCents: 8005, overCount: 1, overCents: 2340, count: 3 },
    budgets: [
      brow(2, { name: 'Salidas', parentId: 1, parentName: 'Ocio', color: 'cat-8', amountCents: 12000, spentCents: 14340, remainingCents: -2340, percentBp: 11950, status: 'over', avg3Cents: 13120, startMonth: '2026-11', previousAmountCents: 10000 }),
      brow(3, { name: 'Suministros', isGroup: true, childNames: ['Luz', 'Agua'], color: 'cat-6', amountCents: 18000, spentCents: 16410, remainingCents: 1590, percentBp: 9117, status: 'warn' }),
      brow(4, { name: 'Alimentación', isGroup: true, childNames: ['Supermercado'], color: 'cat-1', amountCents: 40000, spentCents: 31245, remainingCents: 8755, percentBp: 7811 }),
    ],
    unbudgeted: [
      { categoryId: 5, name: 'Seguros', parentName: 'Finanzas', color: null, spentCents: 4210, avg3Cents: 3860 },
    ],
    uncategorizedCents: 3760,
    ...extra,
  };
}

const CATEGORIES: CategoryOption[] = [
  { id: 1, parentId: null, key: null, name: 'Ocio', kind: 'expense', color: null },
  { id: 2, parentId: 1, key: null, name: 'Salidas', kind: 'expense', color: 'cat-8' },
  { id: 5, parentId: 1, key: null, name: 'Seguros', kind: 'expense', color: null },
  { id: 6, parentId: 1, key: null, name: 'Cine', kind: 'expense', color: null },
  { id: 7, parentId: null, key: null, name: 'Nómina', kind: 'income', color: null },
  { id: 8, parentId: null, key: 'transfer', name: 'Transferencias internas', kind: 'transfer', color: null },
];

describe('Budgets screen', () => {
  let current: BudgetMonth;
  let backend: ReturnType<typeof fakeBackend>;

  async function settle(fixture: { detectChanges(): void; whenStable(): Promise<unknown> }) {
    for (let i = 0; i < 3; i++) {
      fixture.detectChanges();
      await fixture.whenStable();
      await new Promise((r) => setTimeout(r));
    }
  }
  async function render() {
    TestBed.configureTestingModule({ providers: backend.providers });
    const fixture = TestBed.createComponent(Budgets);
    await settle(fixture);
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }
  const text = (el: Element) => el.textContent!.replace(/\s+/g, ' ');
  const kpi = (el: HTMLElement, i: number) => text(el.querySelectorAll('.kpi')[i]);
  const button = (el: ParentNode, label: string) =>
    Array.from(el.querySelectorAll('button')).find((b) => b.textContent?.trim() === label) as HTMLButtonElement;
  const prog = (el: HTMLElement, i: number) => el.querySelectorAll<HTMLElement>('.bud .prog')[i];
  function setInput(el: ParentNode, sel: string, value: string) {
    const i = el.querySelector(sel) as HTMLInputElement;
    i.value = value;
    i.dispatchEvent(new Event('input'));
  }
  function setSelect(el: ParentNode, sel: string, value: string) {
    const s = el.querySelector(sel) as HTMLSelectElement;
    s.value = value;
    s.dispatchEvent(new Event('change'));
  }

  beforeEach(() => {
    current = month();
    backend = fakeBackend({
      ledger_budgets: () => current,
      ledger_categories: () => CATEGORIES,
      ledger_set_budget: () => undefined,
      ledger_remove_budget: () => undefined,
    });
  });

  it('asks for the month with today and shows the KPI math', async () => {
    const { el } = await render();
    expect(backend.of('ledger_budgets')[0].args).toEqual({ month: '2026-11', today: '2026-11-15' });
    expect(kpi(el, 0)).toContain('700,00 €');
    expect(kpi(el, 0)).toContain('3 presupuestos mensuales');
    expect(kpi(el, 1)).toContain('619,95 €');
    expect(kpi(el, 1)).toContain('88,6 % del total · día 16 de 30');
    expect(kpi(el, 2)).toContain('80,05 €');
    expect(kpi(el, 2)).toContain('≈ 5,72 € al día hasta fin de mes');
    expect(kpi(el, 3)).toContain('1 de 3');
    expect(kpi(el, 3)).toContain('23,40 € por encima');
    const foot = text(el.querySelector('tfoot')!);
    expect(foot).toContain('700,00 €');
    expect(foot).toContain('619,95 €');
    expect(foot).toContain('88,6 %');
  });

  it('renders the ok, warn and over states with pace marker and a text label', async () => {
    const { el } = await render();
    const rows = Array.from(el.querySelectorAll('.bud tbody tr'));
    expect(rows).toHaveLength(3);
    expect(prog(el, 0).classList.contains('is-over')).toBe(true);
    expect(prog(el, 1).classList.contains('is-warn')).toBe(true);
    expect(prog(el, 2).classList.contains('is-over') || prog(el, 2).classList.contains('is-warn')).toBe(false);
    expect(text(rows[0])).toContain('−23,40 €');
    expect(text(rows[0])).toContain('Excedido');
    expect(text(rows[0])).toContain('119,5 %');
    expect(text(rows[0])).toContain('Ocio');
    expect(text(rows[1])).not.toContain('Excedido');
    expect(text(rows[1])).toContain('Grupo · Luz, Agua');
    expect(text(rows[1])).toContain('91,2 %');
    expect(prog(el, 0).getAttribute('aria-label')).toBe('Salidas: 119,5 % gastado');
    expect((prog(el, 0).querySelector('.fill') as HTMLElement).style.width).toBe('100%');
    expect((prog(el, 2).querySelector('.fill') as HTMLElement).style.width).toBe('78.11%');
    expect(prog(el, 0).querySelectorAll('.pace')).toHaveLength(1);
    expect((prog(el, 0).querySelector('.pace') as HTMLElement).style.left).toMatch(/^53.33/);
    expect(rows[0].querySelector('.cat i')!.className).toBe('c8');
    expect(text(el.querySelector('.legend-pace')!)).toContain('53,3 % del mes');
  });

  it('hides the pace marker and per-day figure for past months', async () => {
    const { fixture, el } = await render();
    current = month({ month: '2026-10', isCurrentMonth: false, dayOfMonth: null, daysInMonth: 31 });
    (el.querySelector('[aria-label="Mes anterior"]') as HTMLButtonElement).click();
    await settle(fixture);
    expect(backend.of('ledger_budgets').at(-1)!.args).toEqual({ month: '2026-10', today: '2026-11-15' });
    expect(el.querySelectorAll('.pace')).toHaveLength(0);
    expect(kpi(el, 1)).toContain('88,6 % del total');
    expect(kpi(el, 1)).not.toContain('día');
    expect(kpi(el, 2)).not.toContain('al día');
  });

  it('lists spending without a budget and the uncategorized note', async () => {
    const { el } = await render();
    const nob = text(el.querySelector('.nob')!);
    expect(nob).toContain('Este mes · 1 categoría');
    expect(nob).toContain('Seguros');
    expect(nob).toContain('42,10 €');
    expect(nob).toContain('38,60 €');
    expect(nob).toContain('No incluye «Sin categoría» (37,60 €)');
  });

  it('edits a budget: es-ES amount to cents and the apply-from month', async () => {
    const { fixture, el } = await render();
    (el.querySelector('[aria-label="Editar presupuesto de Salidas"]') as HTMLButtonElement).click();
    await settle(fixture);
    const dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    expect(text(dlg)).toContain('Presupuesto de Salidas');
    expect(text(dlg)).toContain('Subcategoría de Ocio');
    expect((dlg.querySelector('#b-amt') as HTMLInputElement).value).toBe('120,00');
    expect(text(dlg)).toContain('Gasto medio de los últimos 3 meses: 131,20 €.');
    expect(text(dlg)).toContain('noviembre 2026 (este mes)');
    expect(text(dlg)).toContain('Los meses anteriores conservan su importe (100,00 €).');

    setInput(dlg, '#b-amt', '1.234,5');
    button(dlg, 'Guardar').click();
    await settle(fixture);
    expect(backend.of('ledger_set_budget')[0].args).toEqual({ categoryId: 2, month: '2026-11', amountCents: 123450 });
    expect(el.querySelector('[role="dialog"]')).toBeNull();
    expect(backend.of('ledger_budgets')).toHaveLength(2);

    (el.querySelector('[aria-label="Editar presupuesto de Salidas"]') as HTMLButtonElement).click();
    await settle(fixture);
    setInput(el, '#b-amt', '150');
    setSelect(el, '#b-from', 'next');
    await settle(fixture);
    expect(text(el.querySelector('[role="dialog"]')!)).toContain('Los meses anteriores conservan su importe (120,00 €).');
    button(el, 'Guardar').click();
    await settle(fixture);
    expect(backend.of('ledger_set_budget')[1].args).toEqual({ categoryId: 2, month: '2026-12', amountCents: 15000 });
  });

  it('removes a budget from the chosen month', async () => {
    const { fixture, el } = await render();
    (el.querySelector('[aria-label="Editar presupuesto de Suministros"]') as HTMLButtonElement).click();
    await settle(fixture);
    expect(text(el.querySelector('[role="dialog"]')!)).toContain('Grupo de categorías');
    button(el, 'Quitar').click();
    await settle(fixture);
    expect(backend.of('ledger_remove_budget')[0].args).toEqual({ categoryId: 3, month: '2026-11' });
    expect(text(el.querySelector('.status-line')!)).toContain('Presupuesto quitado.');
  });

  it('rejects invalid amounts without calling the backend', async () => {
    const { fixture, el } = await render();
    (el.querySelector('[aria-label="Editar presupuesto de Salidas"]') as HTMLButtonElement).click();
    await settle(fixture);
    for (const bad of ['', 'abc', '0', '1.000.001']) {
      setInput(el, '#b-amt', bad);
      button(el, 'Guardar').click();
      await settle(fixture);
      expect(text(el.querySelector('[role="dialog"]')!)).toMatch(/Escribe un importe|demasiado grande/);
    }
    expect(backend.of('ledger_set_budget')).toHaveLength(0);
  });

  it('adds a budget for an expense category chosen in the dialog', async () => {
    const { fixture, el } = await render();
    button(el, 'Añadir presupuesto').click();
    await settle(fixture);
    const dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    const options = Array.from(dlg.querySelectorAll('#b-cat option')).map((o) => o.textContent!.trim());
    expect(options).toEqual(['Elige una categoría', 'Ocio', 'Seguros', 'Cine']);
    button(dlg, 'Guardar').click();
    await settle(fixture);
    expect(text(dlg)).toContain('Elige una categoría.');
    setSelect(dlg, '#b-cat', '6');
    setInput(dlg, '#b-amt', '25,50');
    await settle(fixture);
    button(dlg, 'Guardar').click();
    await settle(fixture);
    expect(backend.of('ledger_set_budget')[0].args).toEqual({ categoryId: 6, month: '2026-11', amountCents: 2550 });
  });

  it('prefills the average when adding from the unbudgeted list', async () => {
    const { fixture, el } = await render();
    (el.querySelector('[aria-label="Añadir presupuesto a Seguros"]') as HTMLButtonElement).click();
    await settle(fixture);
    const dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    expect((dlg.querySelector('#b-amt') as HTMLInputElement).value).toBe('38,60');
    expect(dlg.querySelector('#b-cat')).toBeNull();
    button(dlg, 'Guardar').click();
    await settle(fixture);
    expect(backend.of('ledger_set_budget')[0].args).toEqual({ categoryId: 5, month: '2026-11', amountCents: 3860 });
  });

  it('maps a conflict to the group/subcategory message', async () => {
    backend = fakeBackend({
      ledger_budgets: () => current,
      ledger_categories: () => CATEGORIES,
      ledger_set_budget: () => { throw { code: 'conflict', message: 'generic' }; },
    });
    const { fixture, el } = await render();
    (el.querySelector('[aria-label="Editar presupuesto de Salidas"]') as HTMLButtonElement).click();
    await settle(fixture);
    button(el, 'Guardar').click();
    await settle(fixture);
    expect(text(el.querySelector('[role="dialog"]')!)).toContain('Ya hay un presupuesto en su grupo o en sus subcategorías');
  });
});
