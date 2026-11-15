# pocket-ledger — Design System

> A ledger on kraft paper under a desk lamp. Cream paper, espresso ink, one amber highlight. Serious about numbers, easy on the eyes. At night the lamp dims: warm dark paper, the same ink logic inverted.

Based on the "Runway" style reference (warm-paper fintech), adapted for a **dense desktop data app**: light + dark themes, tabular figures for every amount, WCAG 2.2 AA contrast, and es-ES number/date formatting. Deviations from the source are marked **[adapted]**.

## 1. Principles

1. **Paper, not glass.** Surfaces are flat layers separated by warm hairlines, not floating panels. Shadows are faint and warm.
2. **One highlight.** Amber marks the primary action and the current selection. It never decorates and is never used as text colour.
3. **Numbers first.** Every amount uses tabular figures, right alignment, and an explicit sign. Colour is a second cue, never the only one.
4. **Dense but calm.** Compact spacing, tight line-heights, generous outer margins.
5. **Quiet motion.** 120–180 ms ease-out for state changes; no decorative animation; respect `prefers-reduced-motion`.

## 2. Colour tokens

Semantic tokens are what components use. Raw palette values appear only here.

### Light (default)

| Token | Value | Use | Contrast |
|---|---|---|---|
| `--bg` | `#f8f7f5` Cream Canvas | App background | — |
| `--surface` | `#ffffff` Pure Paper | Cards, tables, panels | — |
| `--surface-sunken` | `#f1efea` | Table header, input fill, hover row | — |
| `--surface-selected` | `#e3dfd5` Linen | Active nav item, selected row | — |
| `--border` | `#e3dfd5` Linen | Card borders, row rules | — |
| `--border-strong` | `#d5d2cd` Stone Mist | Inputs, ghost buttons, nested dividers | — |
| `--text` | `#261b07` Espresso | Headings, amounts, primary text | 16.6:1 on bg |
| `--text-body` | `#61594a` Bark | Body, descriptions | 6.5:1 on bg |
| `--text-muted` | `#736b5e` **[adapted]** | Labels, captions, metadata | 4.9:1 on bg |
| `--icon-muted` | `#8f897e` Driftwood | Non-text only (icons, chart axes) | 3.2:1 (UI ≥ 3:1) |
| `--accent` | `#f9a600` Amber Signal | Primary button fill, selection bar, focus ring, key chart series | text on it: Espresso 8.4:1 |
| `--accent-border` | `#e89b01` Burnished Gold | Outlined/hover accent borders (non-text) | — |
| `--accent-text` | `#8a5a00` **[adapted]** | Links and amber-coloured text | 5.5:1 on bg |
| `--positive` | `#4f6b1a` **[adapted]** olive | Income amounts, under-budget | 5.7:1 on bg |
| `--negative` | `#b23b29` **[adapted]** deep terracotta | Over-budget, errors, alerts | 5.5:1 on bg |
| `--terracotta` | `#f0624f` | Decorative/chart only, never text | — |
| `--badge-honey` | `#f8da9d` | Tag fill (Espresso text 12.5:1) | — |
| `--badge-wisteria` | `#d5befa` | Tag fill (Espresso text 10.1:1) | — |
| `--focus-ring` | `0 0 0 2px var(--surface), 0 0 0 4px var(--accent)` | Keyboard focus | — |

### Dark **[adapted — not in source]**

Warm dark paper, never neutral grey or black.

| Token | Value | Contrast |
|---|---|---|
| `--bg` | `#1a1408` | — |
| `--surface` | `#241c0e` | — |
| `--surface-sunken` | `#1f180b` | — |
| `--surface-selected` | `#3a2f1c` | — |
| `--border` | `#3a3020` | — |
| `--border-strong` | `#4d4130` | — |
| `--text` | `#f3efe6` | 16:1 on bg |
| `--text-body` | `#cfc7b8` | 10:1 on surface |
| `--text-muted` | `#a39a88` | 6.0:1 on surface |
| `--icon-muted` | `#857c6c` | ≥ 3:1 |
| `--accent` | `#f9a600` (text on it: `#261b07`) | 9.1:1 as fill vs bg |
| `--accent-border` | `#c98a12` | — |
| `--accent-text` | `#e8b34a` | 8.8:1 on surface |
| `--positive` | `#a8c46a` | 8.6:1 on surface |
| `--negative` | `#ff8a73` | 7.3:1 on surface |
| `--badge-honey` | `#5a4519` (text `#f8da9d`) | — |
| `--badge-wisteria` | `#3f3257` (text `#e4d6fc`) | — |

Theme follows the OS (`prefers-color-scheme`) with a manual override (Light / Dark / System) stored in app settings, applied as `data-theme` on `<html>`.

### Why olive and terracotta for money **[adapted]**

The source forbids blue/green/red as interface colours. A finance app still needs a positive/negative cue, so we use **desaturated warm** variants (olive, burnt terracotta) that stay inside the paper palette. Amounts always carry a sign (`+1.234,56 €` / `−56,20 €`), so colour is never the only signal.

### Chart palette **[adapted]**

The source's blue→green gradients are replaced with the warm palette. Categorical order (max 6, then "Otros" in `--icon-muted`):

| # | Light | Dark |
|---|---|---|
| 1 | `#f9a600` amber | `#f9a600` |
| 2 | `#b23b29` terracotta | `#ff8a73` |
| 3 | `#7a5cc2` wisteria-deep | `#c4a8f5` |
| 4 | `#4f6b1a` olive | `#a8c46a` |
| 5 | `#61594a` bark | `#cfc7b8` |
| 6 | `#9a6a06` gold-deep | `#e8b34a` |

Income vs expense charts: income `--positive`, expense `--text` (espresso bars), balance line `--accent`. Area fills at 12–16 % opacity. Gridlines `--border`, axis labels `--text-muted`.

## 3. Typography

- **Family:** `"Inter Variable", "Inter", ui-sans-serif, system-ui, "Segoe UI", sans-serif` (Interphases Pro substitute). Bundled locally with the app; no remote font loading (CSP).
- **Features:** `font-feature-settings: "ss01" 1, "cv11" 1;` everywhere. **Amounts and any numeric column:** add `font-variant-numeric: tabular-nums;` (`"tnum" 1`).
- **Weights:** 400 body · 492 emphasis/buttons/labels · 584 headings and key figures. Never ≥ 700.
- **Line-height:** ≤ 1.25 for UI text **[adapted: 1.4 allowed for multi-line notes/paragraphs only, for readability]**.

| Role | Size | Weight | LH | Tracking | Use |
|---|---|---|---|---|---|
| `caption` | 12px | 492 | 1.25 | +0.6px, uppercase optional | Table headers, eyebrow labels |
| `body-sm` | 13px **[adapted]** | 400 | 1.25 | -0.1px | Dense table rows |
| `body` | 14px | 400 | 1.25 | -0.14px | Default UI text |
| `body-lg` | 16px | 400/492 | 1.25 | -0.16px | Form fields, dialogs |
| `title-sm` | 20px | 584 | 1.25 | -0.22px | Card titles |
| `title` | 24px | 584 | 1.13 | -0.31px | Page titles, KPI values |
| `figure` | 36px | 584 | 1.13 | -0.61px | Hero KPI (month balance) |

Display sizes (56–72px) from the source are marketing-only and not used in the app.

## 4. Number, date and text formatting (es-ES)

- Amounts: `Intl.NumberFormat('es-ES', { style: 'currency', currency: 'EUR' })` → `1.234,56 €`. Use the true minus sign `−` (U+2212) and explicit `+` for income in transaction lists.
- Note: es-ES does not group 4-digit numbers by default (`1234,56 €`). Use `useGrouping: 'always'` so columns align (`1.234,56 €`).
- Dates: `dd/MM/yyyy` in tables; `15 nov 2026` in headers; month pickers `noviembre 2026`.
- Percentages: `23,4 %` (space before `%`, es-ES convention).
- All strings via i18n keys; default locale `es-ES`.

## 5. Spacing, radius, elevation

- **Scale:** 4, 8, 12, 16, 20, 24, 32, 40, 48, 56 px. Odd source values (5, 7, 30, 63, 105) dropped **[adapted]**.
- **Density:** table row 36px (compact 32px), card padding 20–24px, page gutter 32px, section gap 24–32px.
- **Radius:** 8px cards, buttons, inputs, nav items; 4px badges/tags; 999px only for status dots.
- **Elevation:**
  - `--shadow-card`: `0 4px 8px 0 rgb(38 27 7 / 0.06)` (light); dark: none, use `--border` only.
  - `--shadow-button`: `inset 0 2px 4px 0 rgb(255 255 255 / 0.56), 0 4px 8px 0 rgb(38 27 7 / 0.06), 0 1px 2px 0 rgb(38 27 7 / 0.36)` (amber button, light); dark: `0 1px 2px 0 rgb(0 0 0 / 0.5)`.
  - `--shadow-popover`: `0 8px 24px 0 rgb(38 27 7 / 0.12)` for menus/dialogs.

## 6. Layout (app shell)

- **Window min size:** 1100 × 700. Content max-width 1440px, left-aligned next to sidebar.
- **Sidebar** 232px: wordmark, nav (Resumen, Movimientos, Categorías, Reglas, Presupuestos, Cuentas, Importar), bottom: sync status + consent days remaining, settings.
- **Top bar** per page: page title, month selector (‹ noviembre 2026 ›), search, primary action.
- **Grid:** 12 columns, 24px gutters. KPI row of 4 tiles; charts 8+4; tables full width.
- No coloured background bands; sections separated by whitespace and Linen rules.

## 7. Components

**Button / primary** — amber fill, Espresso text, 8px radius, 8px 14px padding, 14px weight 492, `--shadow-button`. Max one per view region.
**Button / secondary** — transparent, 1px `--border-strong`, `--text`. Hover: `--surface-sunken`.
**Button / ghost-icon** — 32×32, transparent, icon `--text-body`, hover `--surface-sunken`.
**Sidebar item** — 8px 12px, 14px weight 400, 16px outline icon (1.5px stroke). Active: `--surface-selected` fill + 2px amber bar on the left edge, weight 492.
**Card** — `--surface`, 1px `--border`, 8px radius, `--shadow-card`, 20–24px padding. Header: `title-sm` + optional caption.
**KPI tile** — label `caption` `--text-muted`; value `title` tabular; delta `caption` with arrow + sign (`▲ 4,2 %` olive / `▼ 1,8 %` terracotta) vs previous month; optional 32px sparkline.
**Table** — header `caption` uppercase on `--surface-sunken`, row rules `--border`, no vertical lines. Row 36px. Amount column right-aligned, tabular, signed. Hover `--surface-sunken`; selected `--surface-selected` (on selected rows muted text switches to `--text-body`, since `--text-muted` on Linen is only 3.9:1). Pending rows: italic description + "Pendiente" badge. Internal transfers: `⇄` icon + muted amount, excluded from totals.
**Tag / badge** — 4px radius, 2px 8px padding, 12px weight 492; honey / wisteria / linen variants. Category chips use a 8px colour dot + text, not full fills.
**Input / select** — 36px height, 8px radius, `--surface` fill, 1px `--border-strong`, focus ring amber. Placeholder `--text-muted`.
**Progress / budget bar** — 6px track `--surface-sunken`, fill `--accent`; ≥ 90 % `--accent-border`; > 100 % `--negative` with overflow label.
**Banner / notice** — consent expiry or sync error: `--surface` with a 3px left border (`--accent` warning, `--negative` error), icon + text + action.
**Empty state** — one line of `--text-body`, one secondary action. No illustrations.

## 8. Accessibility

- All text ≥ 4.5:1, large text and UI components ≥ 3:1 (values above are checked).
- Visible focus ring on every interactive element; full keyboard navigation (tables: arrow keys, Enter to edit).
- Never colour alone: signs, arrows, icons and labels accompany colour.
- Hit targets ≥ 32×32 (desktop).
- `prefers-reduced-motion` disables transitions beyond opacity.

## 9. Don'ts

- No `#000000`, no cool greys, no blue/purple interface accents.
- No amber text on light backgrounds (use `--accent-text`).
- No filled buttons in any colour other than amber.
- No gradients, glassmorphism, glow, or decorative illustrations.
- No proportional figures in amount columns.
- No remote fonts, images or scripts (CSP, local-first).

## 10. CSS quick start

```css
:root {
  --bg:#f8f7f5; --surface:#fff; --surface-sunken:#f1efea; --surface-selected:#e3dfd5;
  --border:#e3dfd5; --border-strong:#d5d2cd;
  --text:#261b07; --text-body:#61594a; --text-muted:#736b5e; --icon-muted:#8f897e;
  --accent:#f9a600; --accent-border:#e89b01; --accent-text:#8a5a00; --on-accent:#261b07;
  --positive:#4f6b1a; --negative:#b23b29;
  --badge-honey:#f8da9d; --badge-wisteria:#d5befa; --on-badge:#261b07;
  --shadow-card:0 4px 8px 0 rgb(38 27 7/.06);
  --shadow-button:inset 0 2px 4px 0 rgb(255 255 255/.56),0 4px 8px 0 rgb(38 27 7/.06),0 1px 2px 0 rgb(38 27 7/.36);
  --radius:8px; --radius-badge:4px;
  --font:"Inter Variable","Inter",ui-sans-serif,system-ui,"Segoe UI",sans-serif;
  color-scheme: light;
}
:root[data-theme="dark"] {
  --bg:#1a1408; --surface:#241c0e; --surface-sunken:#1f180b; --surface-selected:#3a2f1c;
  --border:#3a3020; --border-strong:#4d4130;
  --text:#f3efe6; --text-body:#cfc7b8; --text-muted:#a39a88; --icon-muted:#857c6c;
  --accent:#f9a600; --accent-border:#c98a12; --accent-text:#e8b34a; --on-accent:#261b07;
  --positive:#a8c46a; --negative:#ff8a73;
  --badge-honey:#5a4519; --badge-wisteria:#3f3257; --on-badge:#f3efe6;
  --shadow-card:none; --shadow-button:0 1px 2px 0 rgb(0 0 0/.5);
  color-scheme: dark;
}
.num { font-variant-numeric: tabular-nums; font-feature-settings: "tnum" 1, "ss01" 1; }
```
