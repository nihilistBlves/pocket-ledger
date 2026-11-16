import { TestBed } from '@angular/core/testing';
import { CategoryNode } from '../ledger/ledger.models';
import { fakeBackend } from '../ledger/testing';
import { Categories } from './categories';

function node(id: number, extra: Partial<CategoryNode> = {}): CategoryNode {
  return {
    id, parentId: null, key: null, name: `Categoría ${id}`, kind: 'expense', isSystem: false, sortOrder: id,
    color: null, childCount: 0, monthTxCount: 2, monthAmountCents: -5000, totalTxCount: 10, ruleCount: 0, budget: null,
    ...extra,
  };
}

function tree(): CategoryNode[] {
  return [
    node(1, { name: 'Alimentación', childCount: 2, monthTxCount: 5, monthAmountCents: -24000, totalTxCount: 40, ruleCount: 5 }),
    node(2, { name: 'Supermercado', parentId: 1, color: 'cat-7', monthAmountCents: -20000, ruleCount: 3 }),
    node(3, { name: 'Cafeterías', parentId: 1, totalTxCount: 164, monthTxCount: 18, monthAmountCents: -4000, ruleCount: 2, budget: 'group' }),
    node(4, { name: 'Ingresos', kind: 'income', monthAmountCents: 248000, monthTxCount: 1 }),
    node(5, { name: 'Ocio', color: 'cat-8', monthTxCount: 0, monthAmountCents: 0 }),
    node(6, { name: 'Transferencias internas', kind: 'transfer', isSystem: true, key: 'transfer', monthAmountCents: -120000, monthTxCount: 4 }),
    node(7, { name: 'Sin categoría', kind: 'system', isSystem: true, key: 'uncategorized' }),
  ];
}

describe('Categories screen', () => {
  let nodes: CategoryNode[];
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
    const fixture = TestBed.createComponent(Categories);
    await settle(fixture);
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }
  const text = (el: Element) => el.textContent!.replace(/\s+/g, ' ');
  const trs = (el: HTMLElement) => Array.from(el.querySelectorAll<HTMLElement>('.tree tbody tr'));
  const rowOf = (el: HTMLElement, name: string) => trs(el).find((r) => text(r).includes(name))!;
  const byLabel = (el: ParentNode, label: string) => el.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;
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

  beforeEach(() => {
    nodes = tree();
    backend = fakeBackend({
      ledger_category_tree: () => nodes,
      ledger_update_category: (a) => nodes.find((n) => n.id === a['id']),
      ledger_delete_category: () => ({ movedTransactions: 164, repointedRules: 2, deletedRules: 0, deletedBudgets: 0 }),
      ledger_create_category: (a) => node(99, { name: a['name'] as string, parentId: a['parentId'] as number | null }),
    });
  });

  it('renders the groups collapsed with kind, counts, amounts and system locks', async () => {
    const { el } = await render();
    expect(backend.of('ledger_category_tree')[0].args).toEqual({ month: '2026-11' });
    expect(trs(el).map((r) => r.querySelector('.cat')!.textContent!.trim())).toEqual([
      'Alimentación', 'Ingresos', 'Ocio', 'Transferencias internas', 'Sin categoría',
    ]);
    expect(text(rowOf(el, 'Alimentación'))).toContain('2 subcategorías');
    expect(text(rowOf(el, 'Alimentación'))).toContain('−240,00 €');
    expect(text(rowOf(el, 'Ingresos'))).toContain('+2.480,00 €');
    expect(text(rowOf(el, 'Ingresos'))).toContain('Ingreso');
    expect(text(rowOf(el, 'Ocio'))).toContain('—');
    const sys = rowOf(el, 'Transferencias internas');
    expect(text(sys)).toContain('Sistema');
    expect(text(sys)).toContain('⇄ 1.200,00 €');
    expect(sys.querySelector('[aria-label="Categoría del sistema"]')).not.toBeNull();
    expect(sys.querySelector('[aria-label^="Añadir subcategoría"]')).toBeNull();
    expect(text(el.querySelector('.card-head')!)).toContain('3 grupos · 2 subcategorías');
  });

  it('uses stored colours with a fallback to the group position', async () => {
    const { fixture, el } = await render();
    byLabel(el, 'Desplegar Alimentación').click();
    await settle(fixture);
    const dot = (name: string) => rowOf(el, name).querySelector('.cat i')!.className;
    expect(dot('Alimentación')).toBe('c1');
    expect(dot('Supermercado')).toBe('c7');
    expect(dot('Cafeterías')).toBe('c1');
    expect(dot('Ocio')).toBe('c8');
    expect(dot('Ingresos')).toBe('c2');
  });

  it('expands and collapses with the keyboard', async () => {
    const { fixture, el } = await render();
    const row = rowOf(el, 'Alimentación');
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight' }));
    await settle(fixture);
    expect(trs(el)).toHaveLength(7);
    expect(byLabel(el, 'Plegar Alimentación').getAttribute('aria-expanded')).toBe('true');
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft' }));
    await settle(fixture);
    expect(trs(el)).toHaveLength(5);
  });

  it('edits name, group and colour of a subcategory and saves them', async () => {
    const { fixture, el } = await render();
    byLabel(el, 'Desplegar Alimentación').click();
    await settle(fixture);
    rowOf(el, 'Cafeterías').click();
    await settle(fixture);
    const panel = el.querySelector('.panel') as HTMLElement;
    expect(text(panel)).toContain('Subcategoría de Alimentación');
    expect(text(panel)).toContain('164 en total · 18 este mes');
    expect(text(panel)).toContain('Incluido en el de Alimentación');
    expect(text(panel)).toContain('Del grupo');
    expect(panel.querySelector('a[href="/reglas"]')!.textContent).toBe('2 reglas');
    // Same kind only: the income group is not offered.
    const groups = Array.from(panel.querySelectorAll('#c-parent option')).map((o) => o.textContent);
    expect(groups).toEqual(['Alimentación', 'Ocio']);
    expect(button(panel, 'Guardar cambios').disabled).toBe(true);

    setInput(panel, '#c-name', '  Café ');
    setSelect(panel, '#c-parent', '5');
    byLabel(panel, 'Terracota').click();
    await settle(fixture);
    expect(byLabel(panel, 'Terracota').getAttribute('aria-pressed')).toBe('true');
    button(panel, 'Guardar cambios').click();
    await settle(fixture);
    expect(backend.of('ledger_update_category').at(-1)!.args).toEqual({ id: 3, name: 'Café', parentId: 5, color: 'cat-2' });
    expect(backend.of('ledger_category_tree')).toHaveLength(2);
    expect(text(el.querySelector('.status-line')!)).toContain('Cambios guardados.');
  });

  it('offers "Por defecto" for groups and lets the group be recoloured', async () => {
    const { fixture, el } = await render();
    rowOf(el, 'Ocio').click();
    await settle(fixture);
    const panel = el.querySelector('.panel') as HTMLElement;
    expect(text(panel)).toContain('Por defecto');
    expect(text(panel)).not.toContain('Del grupo');
    expect(panel.querySelector('#c-parent')).toBeNull();
    byLabel(panel, 'Gris').click();
    await settle(fixture);
    button(panel, 'Guardar cambios').click();
    await settle(fixture);
    expect(backend.of('ledger_update_category').at(-1)!.args).toEqual({ id: 5, name: 'Ocio', parentId: null, color: 'other' });
  });

  it('restricts system categories to their colour', async () => {
    const { fixture, el } = await render();
    rowOf(el, 'Transferencias internas').click();
    await settle(fixture);
    const panel = el.querySelector('.panel') as HTMLElement;
    expect(text(panel)).toContain('Categoría del sistema');
    expect(text(panel)).toContain('No se puede renombrar, mover ni eliminar.');
    expect(panel.querySelector('#c-name')).toBeNull();
    expect(button(panel, 'Eliminar…')).toBeUndefined();
    byLabel(panel, 'Oliva').click();
    await settle(fixture);
    button(panel, 'Guardar cambios').click();
    await settle(fixture);
    expect(backend.of('ledger_update_category').at(-1)!.args).toEqual({ id: 6, name: 'Transferencias internas', parentId: null, color: 'cat-4' });
  });

  it('blocks deleting a group that still has subcategories and explains why', async () => {
    const { fixture, el } = await render();
    rowOf(el, 'Alimentación').click();
    await settle(fixture);
    const del = button(el, 'Eliminar…');
    expect(del.getAttribute('aria-disabled')).toBe('true');
    expect(text(el.querySelector('#del-why')!)).toBe('Alimentación tiene 2 subcategorías. Muévelas o elimínalas primero.');
    del.click();
    await settle(fixture);
    expect(el.querySelector('[role="dialog"]')).toBeNull();
  });

  it('deletes through the dialog, defaulting to the parent group', async () => {
    const { fixture, el } = await render();
    byLabel(el, 'Desplegar Alimentación').click();
    await settle(fixture);
    rowOf(el, 'Cafeterías').click();
    await settle(fixture);
    button(el, 'Eliminar…').click();
    await settle(fixture);
    const dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    expect(text(dlg)).toContain('Eliminar «Cafeterías»');
    expect(text(dlg)).toContain('Tiene 164 movimientos y 2 reglas');
    expect((dlg.querySelector('#dlg-to') as HTMLSelectElement).value).toBe('1');
    expect(text(dlg.querySelector('#dlg-to')!)).toContain('Alimentación (el grupo)');
    expect(text(dlg)).not.toContain('presupuesto de este mes');
    (dlg.querySelectorAll('input[name="dlg-r"]')[1] as HTMLInputElement).click();
    await settle(fixture);
    button(dlg, 'Eliminar y mover 164 movimientos').click();
    await settle(fixture);
    expect(backend.of('ledger_delete_category')[0].args).toEqual({ id: 3, moveTo: 1, deleteRules: true });
    expect(el.querySelector('[role="dialog"]')).toBeNull();
    expect(text(el.querySelector('.status-line')!)).toContain('«Cafeterías» eliminada.');
    expect(el.querySelector('.panel')).toBeNull();
  });

  it('forces rule deletion when the target is a system category', async () => {
    const { fixture, el } = await render();
    byLabel(el, 'Desplegar Alimentación').click();
    await settle(fixture);
    rowOf(el, 'Cafeterías').click();
    await settle(fixture);
    button(el, 'Eliminar…').click();
    await settle(fixture);
    const dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    setSelect(dlg, '#dlg-to', '7');
    await settle(fixture);
    expect((dlg.querySelectorAll('input[name="dlg-r"]')[0] as HTMLInputElement).disabled).toBe(true);
    expect(text(dlg)).toContain('se eliminarán');
    button(dlg, 'Eliminar y mover 164 movimientos').click();
    await settle(fixture);
    expect(backend.of('ledger_delete_category')[0].args).toEqual({ id: 3, moveTo: 7, deleteRules: true });
  });

  it('warns that an own budget is removed and closes the dialog with Escape', async () => {
    nodes[2].budget = 'own';
    const { fixture, el } = await render();
    byLabel(el, 'Desplegar Alimentación').click();
    await settle(fixture);
    rowOf(el, 'Cafeterías').click();
    await settle(fixture);
    button(el, 'Eliminar…').click();
    await settle(fixture);
    expect(text(el.querySelector('[role="dialog"]')!)).toContain('Su presupuesto de este mes también se quitará.');
    el.querySelector('.backdrop')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle(fixture);
    expect(el.querySelector('[role="dialog"]')).toBeNull();
    expect(backend.of('ledger_delete_category')).toHaveLength(0);
  });

  it('creates a group with its kind and a subcategory without one', async () => {
    const { fixture, el } = await render();
    button(el, 'Nuevo grupo').click();
    await settle(fixture);
    let dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    setInput(dlg, '#n-name', 'Mascotas');
    (dlg.querySelectorAll('input[name="nkind"]')[1] as HTMLInputElement).click();
    await settle(fixture);
    button(dlg, 'Crear').click();
    await settle(fixture);
    expect(backend.of('ledger_create_category')[0].args).toEqual({ parentId: null, name: 'Mascotas', kind: 'income' });

    byLabel(el, 'Añadir subcategoría a Ocio').click();
    await settle(fixture);
    dlg = el.querySelector('[role="dialog"]') as HTMLElement;
    expect(text(dlg)).toContain('Nueva subcategoría de Ocio');
    expect(dlg.querySelector('input[name="nkind"]')).toBeNull();
    setInput(dlg, '#n-name', 'Cine');
    button(dlg, 'Crear').click();
    await settle(fixture);
    expect(backend.of('ledger_create_category')[1].args).toEqual({ parentId: 5, name: 'Cine', kind: null });
  });

  it('maps a conflict to a specific message', async () => {
    backend = fakeBackend({
      ledger_category_tree: () => nodes,
      ledger_update_category: () => { throw { code: 'conflict', message: 'generic' }; },
    });
    const { fixture, el } = await render();
    rowOf(el, 'Ocio').click();
    await settle(fixture);
    setInput(el, '#c-name', 'Alimentación');
    await settle(fixture);
    button(el, 'Guardar cambios').click();
    await settle(fixture);
    expect(text(el.querySelector('.status-line')!)).toContain('Ya existe una categoría con ese nombre');
  });
});
