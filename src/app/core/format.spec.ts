import {
  formatBp,
  formatEurosInput,
  parseEuros,
  formatDate,
  formatDayMonth,
  formatMonthLong,
  formatMonthShort,
  formatMonthYearShort,
  formatPercent,
  formatPeriodLabel,
  formatPoints,
  formatSignedCents,
  monthOf,
  monthRange,
  shiftMonth,
} from './format';

describe('format helpers', () => {
  it('signs amounts explicitly', () => {
    expect(formatSignedCents(285500)).toBe('+2.855,00 €');
    expect(formatSignedCents(-5230)).toBe('−52,30 €');
    expect(formatSignedCents(0)).toBe('0,00 €');
  });

  it('formats percentages and points with es-ES conventions', () => {
    expect(formatPercent(23.4)).toBe('23,4 %');
    expect(formatPercent(-7.59)).toBe('7,6 %');
    expect(formatPoints(14.7)).toBe('14,7 p. p.');
  });

  it('names months', () => {
    expect(formatMonthLong('2026-11')).toBe('noviembre 2026');
    expect(formatMonthShort('2026-11')).toBe('nov');
    expect(formatMonthYearShort('2025-12')).toBe('dic 2025');
    expect(formatDayMonth('2026-11-05')).toBe('05/11');
  });

  it('reads date-only strings as calendar dates', () => {
    expect(formatDate('2026-11-15')).toBe('15/11/2026');
    expect(formatDate('2026-01-01')).toBe('01/01/2026');
  });

  it('shifts months across year boundaries and computes ranges', () => {
    expect(shiftMonth('2026-01', -1)).toBe('2025-12');
    expect(shiftMonth('2026-12', 1)).toBe('2027-01');
    expect(monthRange('2026-02')).toEqual({ from: '2026-02-01', to: '2026-02-28' });
    expect(monthRange('2028-02').to).toBe('2028-02-29');
    expect(monthOf(new Date(2026, 10, 15))).toBe('2026-11');
  });

  it('labels comparison periods', () => {
    expect(formatPeriodLabel({ from: '2026-10-01', to: '2026-10-15' })).toBe('1–15 oct');
    expect(formatPeriodLabel({ from: '2026-10-01', to: '2026-10-31' })).toBe('oct 2026');
  });
});

describe('amount input helpers', () => {
  it('parses es-ES amounts into cents', () => {
    expect(parseEuros('150')).toBe(15000);
    expect(parseEuros('150,5')).toBe(15050);
    expect(parseEuros('150,00 €')).toBe(15000);
    expect(parseEuros('1.234,56')).toBe(123456);
    expect(parseEuros('1234.56')).toBe(123456);
    expect(parseEuros('1.234')).toBe(123400);
    expect(parseEuros(',5')).toBe(50);
    expect(parseEuros('0')).toBe(0);
  });

  it('rejects malformed amounts', () => {
    for (const bad of ['', 'abc', '1,234,5', '12,345', '-5', '1.2.3,4', '12.345,6.7', ',']) {
      expect(parseEuros(bad), bad).toBeNull();
    }
  });

  it('formats cents for an input and basis points as percent', () => {
    expect(formatEurosInput(15000)).toBe('150,00');
    expect(formatEurosInput(123456)).toBe('1234,56');
    expect(formatBp(7810)).toBe('78,1 %');
    expect(formatBp(11950)).toBe('119,5 %');
  });
});
