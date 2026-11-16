import { TestBed } from '@angular/core/testing';
import { CategoryOption, RuleRow, RuleTest } from '../ledger/ledger.models';
import { fakeBackend } from '../ledger/testing';
import { Rules, isUnused, levelOf, monthsBefore } from './rules';

function rule(id: number, extra: Partial<RuleRow> = {}): RuleRow {
  return {
    id, order: id, pattern: `COMERCIO ${id}`, field: 'any', matchType: 'contains', accountId: null,
    direction: 'any', categoryId: 11, priority: 0, origin: 'manual', accountName: null,
    categoryName: 'Supermercado', parentCategoryName: 'Alimentación', hits: 3, lastHitAt: '2026-11-10',
    createdAt: '2026-10-01T10:00:00', ...extra,
  };
}

const CATEGORIES: CategoryOption[] = [
  { id: 10, parentId: null, key: null, name: 'Alimentación', kind: 'expense', color: null },
  { id: 11, parentId: 10, key: null, name: 'Supermercado', kind: 'expense', color: null },
  { id: 12, parentId: 10, key: null, name: 'Cafeterías', kind: 'expense', color: 'cat-7' },
  { id: 30, parentId: null, key: null, name: 'Nómina', kind: 'income', color: null },
  { id: 40, parentId: null, key: 'transfer', name: 'Transferencias internas', kind: 'transfer', color: null },
  { id: 41, parentId: null, key: 'uncategorized', name: 'Sin categoría', kind: 'system', color: null },
];

function fixtureRules(): RuleRow[] {
  return [
    rule(1, { pattern: 'NOMINA ACME', field: 'description', direction: 'in', accountId: 1, accountName: 'Cuenta de prueba', categoryId: 30, categoryName: 'Nómina', priority: 10, hits: 11, lastHitAt: '2026-10-30' }),
    rule(2, { pattern: 'MERCADONA', direction: 'out', hits: 64, lastHitAt: '2026-11-15' }),
    rule(3, { pattern: 'CARREFOUR', priority: -10, hits: 0, lastHitAt: null }),
    rule(4, { pattern: 'CAFE PLAZA MAYOR', direction: 'out', origin: 'learned', categoryId: 12, categoryName: 'Cafeterías', hits: 7, lastHitAt: '2026-03-01', createdAt: '2026-11-03T09:00:00' }),
    rule(5, { pattern: 'GIMNASIO', priority: 5, categoryId: 11, hits: 1, lastHitAt: '2026-11-01' }),
  ];
}

function testResult(): RuleTest {
  const base = { counterparty: null, winningRuleId: null, winningRulePattern: null, winningRuleOrder: null };
  return {
    from: '2026-08-18', to: '2026-11-15', matched: 6, change: 2, unchanged: 1, manual: 1, otherRule: 1,
    rows: [
      { ...base, txId: 1, date: '2026-11-12', description: 'PARKING PLAZA MAYOR', amountCents: -450, categoryId: 11, categoryName: 'Supermercado', outcome: 'change' },
      { ...base, txId: 2, date: '2026-11-09', description: 'CAFETERIA PLAZA 2', amountCents: -320, categoryId: 41, categoryName: 'Sin categoría', outcome: 'change' },
      { ...base, txId: 3, date: '2026-10-21', description: 'CAFE A MANO', amountCents: -1480, categoryId: 11, categoryName: 'Supermercado', outcome: 'manual' },
      { ...base, txId: 4, date: '2026-11-10', description: 'AMAZON PRIME', amountCents: -499, categoryId: 11, categoryName: 'Supermercado', outcome: 'other_rule', winningRuleId: 2, winningRulePattern: 'MERCADONA', winningRuleOrder: 2 },
      { ...base, txId: 5, date: '2026-11-13', description: 'CAFE PLAZA MAYOR', amountCents: -260, categoryId: 12, categoryName: 'Cafeterías', outcome: 'unchanged' },
    ],
  };
}

describe('rule helpers', () => {
  it('maps priorities to levels', () => {
    expect(levelOf(10)).toBe('high');
    expect(levelOf(3)).toBe('high');
    expect(levelOf(0)).toBe('normal');
    expect(levelOf(-10)).toBe('low');
  });

  it('computes six-month unused rules from today', () => {
    expect(monthsBefore('2026-11-15', 6)).toBe('2026-05-15');
    expect(monthsBefore('2026-08-31', 6)).toBe('2026-02-28');
    expect(isUnused({ lastHitAt: null }, '2026-11-15')).toBe(true);
    expect(isUnused({ lastHitAt: '2026-05-14' }, '2026-11-15')).toBe(true);
    expect(isUnused({ lastHitAt: '2026-05-15T08:00:00' }, '2026-11-15')).toBe(false);
  });
});

describe('Rules screen', () => {
  let rules: RuleRow[];
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
    const fixture = TestBed.createComponent(Rules);
    await settle(fixture);
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }
  const text = (el: Element) => el.textContent!.replace(/\s+/g, ' ');
  const trs = (el: HTMLElement) => Array.from(el.querySelectorAll<HTMLElement>('.tbl tbody tr'));
  const button = (el: ParentNode, label: string) =>
    Array.from(el.querySelectorAll('button')).find((b) => b.textContent?.trim() === label) as HTMLButtonElement;
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
  const lastSave = () => backend.of('ledger_save_rule').at(-1)!.args!;

  beforeEach(() => {
    rules = fixtureRules();
    backend = fakeBackend({
      ledger_rules: () => rules,
      ledger_categories: () => CATEGORIES,
      bank_overview: () => ({
        configured: true, aspsp_name: null, connection: { kind: 'not_connected' }, runs: [], next_auto_sync_at: null,
        accounts: [{ id: 1, name: 'Cuenta de prueba' }, { id: 2, name: 'Tarjeta de prueba' }],
      }),
      ledger_save_rule: (a) => {
        const input = a['rule'] as Partial<RuleRow>;
        const id = (a['id'] as number | null) ?? 99;
        const saved = rule(id, { ...input, origin: 'manual', categoryName: 'Cafeterías' });
        rules = id === 99 ? [...rules, saved] : rules.map((r) => (r.id === id ? saved : r));
        return { rule: saved, recategorized: 2 };
      },
      ledger_test_rule: () => testResult(),
      ledger_delete_rule: () => undefined,
      ledger_apply_rules_uncategorized: () => ({ recategorized: 3 }),
    });
  });

  it('lists the rules in evaluation order with condition, scope, origin and hits', async () => {
    const { el } = await render();
    const rows = trs(el);
    expect(rows).toHaveLength(5);
    expect(rows.map((r) => r.querySelector('.order')!.textContent)).toEqual(['1', '2', '3', '4', '5']);
    expect(text(rows[0])).toContain('Contiene NOMINA ACME');
    expect(text(rows[0])).toContain('En descripción · abonos · Cuenta Cuenta de prueba');
    expect(text(rows[0])).toContain('Prioridad alta');
    expect(text(rows[0])).toContain('1130/10/2026');
    expect(text(rows[1])).toContain('Solo cargos');
    expect(text(rows[2])).toContain('Prioridad baja');
    expect(text(rows[2])).toContain('0Nunca');
    expect(text(rows[3])).toContain('Aprendida');
    expect(text(rows[1])).toContain('Manual');
    expect(rows[3].querySelector('.cat i')!.className).toBe('c7');
    expect(text(el)).toContain('Mostrando 5 de 5 reglas');
  });

  it('filters by text, origin (with counts), category and unused', async () => {
    const { fixture, el } = await render();
    expect(text(el.querySelector('.seg')!)).toContain('Todas · 5');
    expect(text(el.querySelector('.seg')!)).toContain('Manuales · 4');
    expect(text(el.querySelector('.seg')!)).toContain('Aprendidas · 1');

    setInput(el, '#r-q', 'mercadona');
    await settle(fixture);
    expect(trs(el)).toHaveLength(1);
    setInput(el, '#r-q', '');
    (el.querySelectorAll('input[name="org"]')[2] as HTMLInputElement).click();
    await settle(fixture);
    expect(trs(el).map((r) => text(r))[0]).toContain('CAFE PLAZA MAYOR');
    expect(trs(el)).toHaveLength(1);
    (el.querySelectorAll('input[name="org"]')[0] as HTMLInputElement).click();
    setSelect(el, '#r-cat', '30');
    await settle(fixture);
    expect(trs(el)).toHaveLength(1);
    setSelect(el, '#r-cat', '');
    (el.querySelector('#r-unused') as HTMLInputElement).click();
    await settle(fixture);
    expect(trs(el).map((r) => r.querySelector('.order')!.textContent)).toEqual(['3', '4']);
    expect(text(el)).toContain('Mostrando 2 de 5 reglas');
  });

  it('turns a learned rule manual on save and maps the priority', async () => {
    const { fixture, el } = await render();
    trs(el)[3].click();
    await settle(fixture);
    const panel = el.querySelector('.panel') as HTMLElement;
    expect(text(panel)).toContain('Regla 4 · editando');
    expect(text(panel)).toContain('Aprendida el 03/11/2026 cuando corregiste «CAFE PLAZA MAYOR»');
    setSelect(panel, '#e-prio', 'high');
    await settle(fixture);
    button(panel, 'Guardar regla').click();
    await settle(fixture);
    expect(lastSave()).toEqual({
      id: 4,
      rule: { pattern: 'CAFE PLAZA MAYOR', field: 'any', matchType: 'contains', accountId: null, direction: 'out', categoryId: 12, priority: 10 },
      applyToExisting: true,
      today: '2026-11-15',
    });
    expect(text(el.querySelector('.status-line')!)).toContain('Regla guardada; se recategorizaron 2 movimientos.');
    expect(text(trs(el)[3])).toContain('Manual');
    expect(trs(el)[3].textContent).not.toContain('Aprendida');
    expect(el.querySelector('.panel .banner')).toBeNull();
  });

  it('keeps an unusual stored priority unless another level is picked', async () => {
    const { fixture, el } = await render();
    trs(el)[4].click();
    await settle(fixture);
    const panel = el.querySelector('.panel') as HTMLElement;
    expect((panel.querySelector('#e-prio') as HTMLSelectElement).value).toBe('high');
    button(panel, 'Guardar regla').click();
    await settle(fixture);
    expect((lastSave()['rule'] as RuleRow).priority).toBe(5);
    setSelect(el, '#e-prio', 'low');
    await settle(fixture);
    button(el, 'Guardar regla').click();
    await settle(fixture);
    expect((lastSave()['rule'] as RuleRow).priority).toBe(-10);
  });

  it('tests the rule over 90 days and renders each outcome', async () => {
    const { fixture, el } = await render();
    trs(el)[3].click();
    await settle(fixture);
    const panel = el.querySelector('.panel') as HTMLElement;
    expect(text(panel)).toContain('Al guardar, aplicar a los movimientos recientes que cambian');
    button(panel, 'Probar').click();
    await settle(fixture);
    expect(backend.of('ledger_test_rule')[0].args).toMatchObject({ id: 4, today: '2026-11-15', rule: { pattern: 'CAFE PLAZA MAYOR', categoryId: 12 } });
    const sum = text(panel.querySelector('.test-sum')!);
    expect(sum).toContain('6 coinciden');
    expect(sum).toContain('2 cambiarían');
    expect(sum).toContain('1 manual, no se toca');
    expect(sum).toContain('1 la gana otra regla');
    const rows = Array.from(panel.querySelectorAll('.mini tbody tr')).map((r) => text(r));
    expect(rows[0]).toContain('PARKING PLAZA MAYOR12/11/2026 · −4,50 €');
    expect(rows[0]).toContain('→ Cafeterías');
    expect(rows[0]).toContain('revisar');
    expect(rows[1]).toContain('→ Cafeterías');
    expect(rows[1]).not.toContain('revisar');
    expect(rows[2]).toContain('puesta a mano · se mantiene');
    expect(rows[3]).toContain('gana la regla 2 «MERCADONA»');
    expect(rows[4]).toContain('sin cambio');
    expect(text(panel)).toContain('Y 1 movimiento más.');
    expect(text(panel)).toContain('Al guardar, aplicar a los 2 que cambian');

    // Opting out of the apply-on-save sends false.
    (panel.querySelector('#e-apply') as HTMLInputElement).click();
    await settle(fixture);
    button(panel, 'Guardar regla').click();
    await settle(fixture);
    expect(lastSave()['applyToExisting']).toBe(false);
  });

  it('discards a stale test when the draft changes', async () => {
    const { fixture, el } = await render();
    trs(el)[1].click();
    await settle(fixture);
    button(el, 'Probar').click();
    await settle(fixture);
    expect(el.querySelector('.test-sum')).not.toBeNull();
    setInput(el, '#e-pat', 'LIDL');
    await settle(fixture);
    expect(el.querySelector('.test-sum')).toBeNull();
  });

  it('creates a rule from an empty editor', async () => {
    const { fixture, el } = await render();
    button(el, 'Nueva regla').click();
    await settle(fixture);
    const panel = el.querySelector('.panel') as HTMLElement;
    expect(button(panel, 'Guardar regla').disabled).toBe(true);
    expect(button(panel, 'Eliminar')).toBeUndefined();
    // Transfer and system categories are not offered.
    const options = Array.from(panel.querySelectorAll('#e-cat option')).map((o) => o.textContent!.trim());
    expect(options).toEqual(['Elige una categoría', 'Alimentación', 'Supermercado', 'Cafeterías', 'Nómina']);
    setInput(panel, '#e-pat', 'Lidl');
    setSelect(panel, '#e-cat', '11');
    setSelect(panel, '#e-acct', '2');
    setSelect(panel, '#e-field', 'counterparty');
    (panel.querySelectorAll('input[name="mt"]')[1] as HTMLInputElement).click();
    await settle(fixture);
    button(panel, 'Guardar regla').click();
    await settle(fixture);
    expect(lastSave()).toMatchObject({
      id: null,
      rule: { pattern: 'Lidl', field: 'counterparty', matchType: 'equals', accountId: 2, direction: 'any', categoryId: 11, priority: 0 },
      applyToExisting: true,
    });
  });

  it('confirms before deleting a rule', async () => {
    const { fixture, el } = await render();
    (el.querySelector('[aria-label="Eliminar regla 2"]') as HTMLButtonElement).click();
    await settle(fixture);
    const dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    expect(text(dlg)).toContain('¿Eliminar la regla «MERCADONA → Supermercado»?');
    expect(text(dlg)).toContain('Ha categorizado 64 movimientos.');
    expect(backend.of('ledger_delete_rule')).toHaveLength(0);
    button(dlg, 'Eliminar regla').click();
    await settle(fixture);
    expect(backend.of('ledger_delete_rule')[0].args).toEqual({ id: 2 });
    expect(el.querySelector('[role="dialog"]')).toBeNull();
    expect(text(el.querySelector('.status-line')!)).toContain('Regla eliminada.');
  });

  it('applies the rules to uncategorized movements and reports the result', async () => {
    const { fixture, el } = await render();
    button(el, 'Aplicar a sin categoría').click();
    await settle(fixture);
    expect(backend.of('ledger_apply_rules_uncategorized')).toHaveLength(1);
    expect(text(el.querySelector('.status-line')!)).toContain('Se categorizaron 3 movimientos.');
  });

  it('shows a specific message when saving conflicts', async () => {
    backend = fakeBackend({
      ledger_rules: () => rules,
      ledger_categories: () => CATEGORIES,
      ledger_save_rule: () => { throw { code: 'conflict', message: 'generic' }; },
    });
    const { fixture, el } = await render();
    trs(el)[1].click();
    await settle(fixture);
    button(el, 'Guardar regla').click();
    await settle(fixture);
    expect(text(el.querySelector('.status-line')!)).toContain('Ya existe una regla igual');
  });
});
