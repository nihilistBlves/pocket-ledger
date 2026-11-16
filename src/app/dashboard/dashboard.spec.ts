import { TestBed } from '@angular/core/testing';
import { Router } from '@angular/router';
import { Dashboard as DashboardData, TxRow } from '../ledger/ledger.models';
import { fakeBackend } from '../ledger/testing';
import { Dashboard, percentDelta, pointsDelta, savingsRate } from './dashboard';
import { buildChart, formatAxis, niceStep } from './evolution-chart';

function row(id: number, extra: Partial<TxRow> = {}): TxRow {
  return {
    id, accountId: 1, accountName: 'Cuenta de prueba', status: 'booked', date: '2026-11-14',
    bookingDate: '2026-11-14', valueDate: null, amountCents: -6318, currency: 'EUR',
    description: `Comercio ${id}`, counterparty: null, remittance: null,
    categoryId: 10, categoryName: 'Supermercado', categoryKey: null, parentCategoryId: 1,
    parentCategoryName: 'Alimentación', categorySource: 'rule', isTransfer: false,
    transferPeerId: null, transferPeerAccountName: null, notes: null, tags: [], ...extra,
  };
}

function evolution(): DashboardData['evolution'] {
  const months = ['2025-12', '2026-01', '2026-02', '2026-03', '2026-04', '2026-05', '2026-06', '2026-07', '2026-08', '2026-09', '2026-10', '2026-11'];
  return months.map((month, i) => ({
    month,
    incomeCents: 200000 + i * 1000,
    expenseCents: 150000 + i * 500,
    balanceCents: i === 3 ? -20000 : 50000 + i * 500,
  }));
}

function dashboard(extra: Partial<DashboardData> = {}): DashboardData {
  const names = ['Vivienda', 'Alimentación', 'Transporte', 'Suministros', 'Compras', 'Restaurantes', 'Ocio', 'Salud'];
  return {
    month: '2026-11',
    isCurrentMonth: true,
    throughDay: 15,
    totals: { incomeCents: 285500, expenseCents: 160885, balanceCents: 124615 },
    previous: { incomeCents: 245000, expenseCents: 174100, balanceCents: 70900 },
    previousLabelRange: { from: '2026-10-01', to: '2026-10-15' },
    evolution: evolution(),
    categories: names.map((name, i) => ({
      categoryId: i + 1, name, key: null, expenseCents: 89500 - i * 10000, txCount: 2,
    })),
    accounts: [
      { id: 1, name: 'Cuenta de prueba', kind: 'checking', displaySuffix: '1111', balanceCents: 318452, currency: 'EUR', balanceDate: '2026-11-15', lastSyncedAt: null },
      { id: 2, name: 'Tarjeta de prueba', kind: 'card', displaySuffix: null, balanceCents: -10573, currency: 'EUR', balanceDate: '2026-11-15', lastSyncedAt: null },
    ],
    recent: [
      row(1, { status: 'pending', amountCents: -5230, description: 'Gasolinera' }),
      row(2),
      row(3, { amountCents: 2500 }),
      row(4, { isTransfer: true, categoryName: null }),
    ],
    monthTxCount: 25,
    ...extra,
  };
}

describe('Dashboard screen', () => {
  let current: DashboardData;
  let backend: ReturnType<typeof fakeBackend>;

  async function settle(fixture: { detectChanges(): void; whenStable(): Promise<unknown> }) {
    for (let i = 0; i < 3; i++) {
      fixture.detectChanges();
      await fixture.whenStable();
    }
  }
  async function render() {
    TestBed.configureTestingModule({ providers: backend.providers });
    const fixture = TestBed.createComponent(Dashboard);
    await settle(fixture);
    return { fixture, el: fixture.nativeElement as HTMLElement };
  }
  const click = (el: HTMLElement, sel: string) => (el.querySelector(sel) as HTMLElement).click();

  beforeEach(() => {
    current = dashboard();
    backend = fakeBackend({
      ledger_dashboard: () => current,
      bank_overview: () => ({ configured: true, aspsp_name: null, connection: { kind: 'not_connected' }, accounts: [], runs: [], next_auto_sync_at: null }),
    });
  });

  it('renders KPIs with deltas and the comparison label', async () => {
    const { el } = await render();
    expect(backend.of('ledger_dashboard')[0].args).toEqual({ month: '2026-11', today: '2026-11-15' });
    const kpi = (k: string) => el.querySelector(`[data-kpi="${k}"]`)!.textContent!.replace(/\s+/g, ' ');
    expect(kpi('income')).toContain('2.855,00 €');
    expect(kpi('income')).toContain('16,5 %');
    expect(kpi('income')).toContain('vs. 1–15 oct');
    expect(kpi('expense')).toContain('1.608,85 €');
    expect(kpi('expense')).toContain('7,6 %');
    expect(kpi('balance')).toContain('+1.246,15 €');
    expect(kpi('savings')).toContain('43,6 %');
    expect(kpi('savings')).toContain('14,7 p. p.');
    // Expense going down is good, income going up is good.
    expect(el.querySelector('[data-kpi="expense"] .delta')!.classList.contains('good')).toBe(true);
    expect(el.querySelector('[data-kpi="expense"] .delta')!.textContent).toContain('▼');
    expect(el.querySelector('[data-kpi="income"] .delta')!.textContent).toContain('▲');
  });

  it('folds categories beyond the sixth into Otros (n)', async () => {
    const { el } = await render();
    const items = Array.from(el.querySelectorAll('.catlist li')).map((li) => li.textContent!.replace(/\s+/g, ' ').trim());
    expect(items).toHaveLength(7);
    expect(items[0]).toContain('Vivienda');
    expect(items[0]).toContain('895,00 €');
    expect(items[6]).toContain('Otros (2)');
    expect(items[6]).toContain('490,00 €');
    expect(el.textContent).toContain('Total gastos');
  });

  it('shows accounts with their total and recent rows with badges', async () => {
    const { el } = await render();
    const text = el.textContent!;
    expect(text).toContain('3.184,52 €');
    expect(text).toContain('−105,73 €');
    expect(text).toContain('3.078,79 €');
    expect(text).toContain('Ver todos (25)');
    expect(text).toContain('Pendiente');
    expect(text).toContain('+25,00 €');
    expect(el.querySelectorAll('.tbl tbody tr')).toHaveLength(4);
  });

  it('describes accounts with suffix and relative sync, and excludes missing balances', async () => {
    const synced = new Date(2026, 10, 15, 10, 0).toISOString();
    current = dashboard({
      accounts: [
        { id: 1, name: 'Cuenta de prueba', kind: 'checking', displaySuffix: '1234', balanceCents: 9749, currency: 'EUR', balanceDate: null, lastSyncedAt: synced },
        { id: 2, name: 'Cuenta sin saldo', kind: 'other', displaySuffix: null, balanceCents: null, currency: 'EUR', balanceDate: null, lastSyncedAt: null },
      ],
    });
    const { el } = await render();
    const items = Array.from(el.querySelectorAll('.acct-list li')).map((li) => li.textContent!.replace(/\s+/g, ' ').trim());
    expect(items[0]).toContain('•••• 1234 · hace 2 h');
    expect(items[0]).toContain('97,49 €');
    expect(items[0]).not.toContain('sin saldo');
    expect(items[1]).toContain('sin saldo');
    expect(items[1]).toContain('—');
    expect(el.querySelector('.total-line.flat')!.textContent).toContain('97,49 €');
  });

  it('renders the chart with a hidden data table and titles', async () => {
    const { el } = await render();
    const svg = el.querySelector('svg.chart')!;
    expect(svg.getAttribute('role')).toBe('img');
    expect(svg.querySelector('title')!.textContent).toContain('Evolución mensual');
    expect(svg.querySelectorAll('.bal-dot')).toHaveLength(12);
    expect(svg.textContent).toContain('nov*');
    expect(el.querySelectorAll('table.sr-only tbody tr')).toHaveLength(12);
    expect(el.textContent).toContain('* Noviembre, hasta el 15');
  });

  it('navigates months and disables next at the current month', async () => {
    const { fixture, el } = await render();
    expect((el.querySelector('[aria-label="Mes siguiente"]') as HTMLButtonElement).disabled).toBe(true);
    current = dashboard({ month: '2026-10', isCurrentMonth: false, throughDay: null, previousLabelRange: { from: '2026-09-01', to: '2026-09-30' } });
    click(el, '[aria-label="Mes anterior"]');
    await settle(fixture);
    expect(backend.of('ledger_dashboard').at(-1)!.args).toEqual({ month: '2026-10', today: '2026-11-15' });
    expect(el.querySelector('.month-label')!.textContent).toBe('octubre 2026');
    expect(el.textContent).toContain('vs. sep 2026');
    expect((el.querySelector('[aria-label="Mes siguiente"]') as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows an empty state without accounts', async () => {
    current = dashboard({ accounts: [], recent: [], monthTxCount: 0 });
    const { el } = await render();
    expect(el.querySelector('.empty')!.textContent).toContain('Todavía no hay cuentas');
    expect(el.querySelector('.empty a')!.textContent).toContain('Ir a Cuentas');
    expect(el.querySelector('.kpis')).toBeNull();
  });

  it('shows an empty state for a month without movements, keeping the balances', async () => {
    current = dashboard({ monthTxCount: 0, recent: [], categories: [] });
    const { el } = await render();
    expect(el.querySelector('.empty')!.textContent).toContain('No hay movimientos en noviembre 2026');
    expect(el.textContent).toContain('Saldo por cuenta');
    expect(el.querySelector('.kpis')).toBeNull();
  });

  it('reloads when a sync finishes', async () => {
    const { fixture } = await render();
    const before = backend.of('ledger_dashboard').length;
    backend.state.emit();
    await settle(fixture);
    expect(backend.of('ledger_dashboard').length).toBe(before + 1);
  });

  it('searches by navigating to the transactions page', async () => {
    const { fixture, el } = await render();
    const navigate = vi.spyOn(TestBed.inject(Router), 'navigate').mockResolvedValue(true);
    const input = el.querySelector('#dash-q') as HTMLInputElement;
    input.value = ' mercadona ';
    input.dispatchEvent(new Event('input'));
    el.querySelector('form.search')!.dispatchEvent(new Event('submit', { cancelable: true }));
    await settle(fixture);
    expect(navigate).toHaveBeenCalledWith(['/movimientos'], { queryParams: { q: 'mercadona' } });
  });
});

describe('dashboard helpers', () => {
  it('computes deltas and the savings rate', () => {
    expect(percentDelta(110, 100, true)).toMatchObject({ arrow: '▲', cls: 'good', text: '10,0 %' });
    expect(percentDelta(110, 100, false)).toMatchObject({ arrow: '▲', cls: 'bad' });
    expect(percentDelta(-50, -100, true)).toMatchObject({ arrow: '▲', cls: 'good' });
    expect(percentDelta(10, 0, true)).toBeNull();
    expect(percentDelta(100, 100, true)).toMatchObject({ arrow: '', cls: '' });
    expect(pointsDelta(30, 40)).toMatchObject({ arrow: '▼', cls: 'bad', text: '10,0 p. p.' });
    expect(savingsRate({ incomeCents: 0, expenseCents: 5, balanceCents: -5 })).toBeNull();
    expect(savingsRate({ incomeCents: 1000, expenseCents: 750, balanceCents: 250 })).toBe(25);
  });
});

describe('evolution chart geometry', () => {
  it('extends the axis below zero for negative balances', () => {
    const model = buildChart(evolution());
    expect(model.slots).toHaveLength(12);
    expect(model.ticks[0].cents).toBeLessThan(0);
    expect(model.slots[3].balanceY).toBeGreaterThan(model.zeroY);
    expect(model.slots[0].income!.y).toBeLessThan(model.zeroY);
    expect(model.linePoints.split(' ')).toHaveLength(12);
  });

  it('skips zero-height bars and picks round steps', () => {
    const model = buildChart([{ month: '2026-11', incomeCents: 0, expenseCents: 0, balanceCents: 0 }]);
    expect(model.slots[0].income).toBeNull();
    expect(niceStep(1000000, 4)).toBe(250000);
    expect(formatAxis(250000)).toBe('2,5 k €');
    expect(formatAxis(-50000)).toBe('−500 €');
  });
});
