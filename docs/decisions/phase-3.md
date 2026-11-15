---
title: Phase 3 decisions
---

# Phase 3 decisions (data core)

Taken with the owner on 2026-11-15.

## Default category tree (seeded, Spanish, editable)

| Group | Subcategories |
|---|---|
| Ingresos | Nómina, Reembolsos, Otros ingresos |
| Vivienda | Alquiler/Hipoteca, Comunidad, Mantenimiento |
| Suministros | Luz, Agua, Gas, Internet y móvil |
| Alimentación | Supermercado, Restaurantes, Cafeterías |
| Transporte | Combustible, Transporte público, Taxi/VTC, Parking y peajes |
| Salud | Farmacia, Médico, Deporte |
| Ocio | Suscripciones, Viajes, Cultura, Salidas |
| Compras | Ropa, Tecnología, Hogar, Regalos |
| Finanzas | Comisiones, Seguros, Impuestos, Ahorro/Inversión |
| Educación | Cursos, Libros |
| Transferencias internas | — (system category, excluded from income/expense) |
| Sin categoría | — (system category, default) |

Seeded categories carry a stable `key` (e.g. `food.groceries`) so later migrations and rules can refer to them even if the owner renames them.

## Internal-transfer matching

- Candidates: one outflow and one inflow on **two different own accounts**, same currency, **exactly opposite amounts**, booked, neither already matched.
- Window: **±2 days** on booking date.
- Tie-breaking: smallest date distance, then the oldest unmatched candidate (by booking date, then id). A transaction is never matched twice.
- Matched pairs are linked and both get the "Transferencias internas" category; they are excluded from income/expense totals. The owner can unlink.

## Import formats (first)

1. **Openbank export** as a fixed profile. Exact column layout is **UNVERIFIED** until the owner provides the header row of a real export (no data). Implemented as a declarative profile so only the profile changes.
2. **Generic CSV** with column mapping (date, value date, description, amount or debit/credit columns, decimal/thousand separators, date format, delimiter, header rows to skip), saveable as a named profile.

Both feed the same normalization → dedup → transfer → categorization pipeline as bank sync.

## Frontend

Angular latest stable (22.x), standalone + signals + zoneless, plain CSS with DESIGN.md tokens, `@angular/cdk` for headless pieces only. No visual component library.

## Build toolchain

SQLCipher is built with vendored OpenSSL, which needs a native Windows Perl at build time: Strawberry Perl portable in `C:\Dev\tools\strawberry-perl` (dev environment PATH only). Not needed at runtime.
