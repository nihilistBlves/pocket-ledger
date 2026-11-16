import {
  MappingForm,
  amountFormatKey,
  batchCounts,
  columnLetter,
  columnOptions,
  convertRefs,
  dateFormatLabel,
  dateFormatOptions,
  datePatternFor,
  decimalSpecFor,
  describeProfile,
  formFromProfile,
  headerRowIndex,
  profileFromForm,
  rowErrorText,
  sameRef,
} from './import.helpers';
import { ImportProfile, RawTable } from './import.models';

function profile(extra: Partial<ImportProfile> = {}): ImportProfile {
  return {
    format: { type: 'Csv', delimiter: ';', encoding: 'Windows1252', quote: '"' },
    skip_rows: null,
    has_header: true,
    columns: {
      booking_date: { Name: 'Fecha' },
      value_date: null,
      description: { Name: 'Concepto' },
      counterparty: null,
      remittance: null,
      entry_reference: null,
      amount: { Single: { column: { Name: 'Importe' } } },
    },
    date_formats: ['%d/%m/%Y'],
    decimal: { decimal: ',', thousands: '.' },
    currency: 'EUR',
    ...extra,
  };
}

const RAW: RawTable = {
  rows: [
    ['Cuenta ficticia', '', ''],
    ['Fecha', 'Concepto', 'Importe'],
    ['03/10/2026', 'MERCADO DEMO', '-42,18'],
  ],
  rowNumbers: [1, 2, 3],
  totalRows: 3,
  columnCount: 3,
};

describe('date formats', () => {
  it('maps display labels to backend patterns and back', () => {
    expect(datePatternFor('dd/mm/aaaa')).toBe('%d/%m/%Y');
    expect(datePatternFor('aaaa-mm-dd')).toBe('%Y-%m-%d');
    expect(datePatternFor('dd/mm/aa')).toBe('%d/%m/%y');
    expect(datePatternFor('otra cosa')).toBeNull();
    expect(dateFormatLabel('%d.%m.%Y')).toBe('dd.mm.aaaa');
    expect(dateFormatLabel('%m/%d/%Y')).toBe('mm/dd/aaaa');
  });

  it('keeps an unknown stored format as an extra option', () => {
    const options = dateFormatOptions('%d.%m.%Y');
    expect(options.at(-1)).toEqual({ label: 'dd.mm.aaaa', pattern: '%d.%m.%Y' });
    expect(dateFormatOptions('%d/%m/%Y').length).toBe(5);
  });
});

describe('amount formats', () => {
  it('maps the three offered styles to a DecimalSpec', () => {
    expect(decimalSpecFor('es')).toEqual({ decimal: ',', thousands: '.' });
    expect(decimalSpecFor('en')).toEqual({ decimal: '.', thousands: ',' });
    expect(decimalSpecFor('plain')).toEqual({ decimal: '.', thousands: null });
    expect(amountFormatKey({ decimal: '.', thousands: null })).toBe('plain');
    expect(amountFormatKey({ decimal: ',', thousands: null })).toBeNull();
  });
});

describe('column references', () => {
  it('names columns like a spreadsheet', () => {
    expect([0, 1, 25, 26, 27].map(columnLetter)).toEqual(['A', 'B', 'Z', 'AA', 'AB']);
  });

  it('compares names ignoring case and accents, and indexes exactly', () => {
    expect(sameRef({ Name: 'Fecha de operación' }, { Name: ' FECHA DE OPERACION' })).toBe(true);
    expect(sameRef({ Index: 2 }, { Index: 2 })).toBe(true);
    expect(sameRef({ Index: 2 }, { Name: 'x' })).toBe(false);
    expect(sameRef(null, null)).toBe(true);
  });

  it('builds options by name with a header and by index without one', () => {
    const f = formFromProfile(profile({ skip_rows: 1 }));
    const withHeader = columnOptions(RAW, f);
    expect(withHeader.map((o) => o.label)).toEqual(['A · Fecha', 'B · Concepto', 'C · Importe']);
    expect(withHeader[0].ref).toEqual({ Name: 'Fecha' });

    const noHeader = columnOptions(RAW, { ...f, hasHeader: false, skipRows: 2 });
    expect(noHeader.map((o) => o.label)).toEqual(['A', 'B', 'C']);
    expect(noHeader[2].ref).toEqual({ Index: 2 });
  });

  it('finds the header row from a given skip or automatically', () => {
    const f = formFromProfile(profile());
    expect(headerRowIndex(RAW.rows, f)).toBe(1);
    expect(headerRowIndex(RAW.rows, { ...f, skipRows: 0 })).toBe(0);
    expect(headerRowIndex(RAW.rows, { ...f, hasHeader: false })).toBe(-1);
    expect(headerRowIndex([['otra', 'cosa']], f)).toBe(-1);
  });

  it('re-expresses refs when the header checkbox is toggled', () => {
    const f = formFromProfile(profile({ skip_rows: 1 }));
    const off = convertRefs(f, RAW, false);
    expect(off.hasHeader).toBe(false);
    expect(off.bookingDate).toEqual({ Index: 0 });
    expect(off.amount).toEqual({ Index: 2 });
    const on = convertRefs({ ...off, skipRows: 1 }, RAW, true);
    expect(on.description).toEqual({ Name: 'Concepto' });
  });
});

describe('profile from form', () => {
  it('round-trips a profile and builds the debit/credit variant', () => {
    const p = profile();
    expect(profileFromForm(formFromProfile(p))).toEqual(p);

    const f: MappingForm = {
      ...formFromProfile(profile({ has_header: false, skip_rows: 2 })),
      bookingDate: { Index: 0 },
      description: { Index: 1 },
      amountMode: 'split',
      debit: { Index: 2 },
      credit: { Index: 3 },
      counterparty: { Index: 4 },
      decimal: decimalSpecFor('en'),
      dateFormats: ['%Y-%m-%d'],
    };
    const built = profileFromForm(f)!;
    expect(built.has_header).toBe(false);
    expect(built.skip_rows).toBe(2);
    expect(built.columns.amount).toEqual({ DebitCredit: { debit: { Index: 2 }, credit: { Index: 3 } } });
    expect(built.columns.counterparty).toEqual({ Index: 4 });
    expect(built.date_formats).toEqual(['%Y-%m-%d']);
    expect(built.decimal).toEqual({ decimal: '.', thousands: ',' });
  });

  it('returns null while a required column is missing', () => {
    const f = formFromProfile(profile());
    expect(profileFromForm({ ...f, bookingDate: null })).toBeNull();
    expect(profileFromForm({ ...f, amount: null })).toBeNull();
    expect(profileFromForm({ ...f, amountMode: 'split', debit: { Index: 1 }, credit: null })).toBeNull();
  });
});

describe('texts', () => {
  it('describes a profile', () => {
    expect(describeProfile(profile({ skip_rows: 5 }))).toBe(
      'Fechas dd/mm/aaaa, importe con signo, cabecera en la fila 6.',
    );
    expect(describeProfile(profile())).toBe('Fechas dd/mm/aaaa, importe con signo, cabecera detectada automáticamente.');
  });

  it('builds row error reasons from kind, column and problem only', () => {
    expect(rowErrorText({ kind: 'invalid_amount', column: 'Importe', problem: 'malformed' })).toBe(
      'Importe no válido en la columna «Importe»: formato no reconocido.',
    );
    expect(rowErrorText({ kind: 'invalid_amount', column: 'Importe', problem: 'too_many_decimals' })).toContain(
      'más de dos decimales',
    );
    expect(rowErrorText({ kind: 'invalid_date', column: 'Fecha' })).toBe('Fecha no válida en la columna «Fecha».');
    expect(rowErrorText({ kind: 'missing_value', column: 'Concepto' })).toBe('Falta un valor en la columna «Concepto».');
    expect(rowErrorText({ kind: 'ambiguous_amount' })).toBe('Cargo y abono tienen importe a la vez.');
  });

  it('summarises a batch like the history does', () => {
    expect(batchCounts({ inserted: 38, updated: 0, duplicates: 3 })).toBe('38 nuevos · 3 duplicados');
    expect(batchCounts({ inserted: 1, updated: 0, duplicates: 0 })).toBe('1 nuevo');
    expect(batchCounts({ inserted: 0, updated: 0, duplicates: 44 })).toBe('Sin cambios · 44 duplicados');
  });
});
