-- 0002_seed_categories: default Spanish category tree (docs/decisions/phase-3.md).
-- Stable English dotted keys; names are editable by the owner, keys are not.
-- Groups are top-level rows; subcategories reference their group by key.

INSERT INTO categories (key, name, kind, sort_order, is_system) VALUES
    ('income',            'Ingresos',               'income',   10,  0),
    ('housing',           'Vivienda',               'expense',  20,  0),
    ('utilities',         'Suministros',            'expense',  30,  0),
    ('food',              'Alimentación',           'expense',  40,  0),
    ('transport',         'Transporte',             'expense',  50,  0),
    ('health',            'Salud',                  'expense',  60,  0),
    ('leisure',           'Ocio',                   'expense',  70,  0),
    ('shopping',          'Compras',                'expense',  80,  0),
    ('finance',           'Finanzas',               'expense',  90,  0),
    ('education',         'Educación',              'expense',  100, 0),
    ('transfer.internal', 'Transferencias internas', 'transfer', 110, 1),
    ('uncategorized',     'Sin categoría',          'system',   120, 1);

WITH v(parent_key, key, name, sort_order) AS (
    VALUES
        ('income',    'income.salary',                'Nómina',              1),
        ('income',    'income.refunds',               'Reembolsos',          2),
        ('income',    'income.other',                 'Otros ingresos',      3),
        ('housing',   'housing.rent_mortgage',        'Alquiler/Hipoteca',   1),
        ('housing',   'housing.community',            'Comunidad',           2),
        ('housing',   'housing.maintenance',          'Mantenimiento',       3),
        ('utilities', 'utilities.electricity',        'Luz',                 1),
        ('utilities', 'utilities.water',              'Agua',                2),
        ('utilities', 'utilities.gas',                'Gas',                 3),
        ('utilities', 'utilities.internet_mobile',    'Internet y móvil',    4),
        ('food',      'food.groceries',               'Supermercado',        1),
        ('food',      'food.restaurants',             'Restaurantes',        2),
        ('food',      'food.cafes',                   'Cafeterías',          3),
        ('transport', 'transport.fuel',               'Combustible',         1),
        ('transport', 'transport.public',             'Transporte público',  2),
        ('transport', 'transport.taxi',               'Taxi/VTC',            3),
        ('transport', 'transport.parking_tolls',      'Parking y peajes',    4),
        ('health',    'health.pharmacy',              'Farmacia',            1),
        ('health',    'health.doctor',                'Médico',              2),
        ('health',    'health.sports',                'Deporte',             3),
        ('leisure',   'leisure.subscriptions',        'Suscripciones',       1),
        ('leisure',   'leisure.travel',               'Viajes',              2),
        ('leisure',   'leisure.culture',              'Cultura',             3),
        ('leisure',   'leisure.going_out',            'Salidas',             4),
        ('shopping',  'shopping.clothing',            'Ropa',                1),
        ('shopping',  'shopping.tech',                'Tecnología',          2),
        ('shopping',  'shopping.home',                'Hogar',               3),
        ('shopping',  'shopping.gifts',               'Regalos',             4),
        ('finance',   'finance.fees',                 'Comisiones',          1),
        ('finance',   'finance.insurance',            'Seguros',             2),
        ('finance',   'finance.taxes',                'Impuestos',           3),
        ('finance',   'finance.savings_investment',   'Ahorro/Inversión',    4),
        ('education', 'education.courses',            'Cursos',              1),
        ('education', 'education.books',              'Libros',              2)
)
INSERT INTO categories (parent_id, key, name, kind, sort_order, is_system)
SELECT p.id, v.key, v.name, p.kind, v.sort_order, 0
FROM v
JOIN categories p ON p.key = v.parent_key;
