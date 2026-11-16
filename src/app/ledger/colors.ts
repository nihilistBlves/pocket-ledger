import { ColorToken } from './ledger.models';

/** The fixed palette (DESIGN.md chart palette) with its Spanish colour names, in picker order. */
export const COLOR_PALETTE: ReadonlyArray<{ token: ColorToken; label: string }> = [
  { token: 'cat-1', label: 'Ámbar' },
  { token: 'cat-2', label: 'Terracota' },
  { token: 'cat-3', label: 'Glicinia' },
  { token: 'cat-4', label: 'Oliva' },
  { token: 'cat-5', label: 'Corteza' },
  { token: 'cat-6', label: 'Oro viejo' },
  { token: 'cat-7', label: 'Verde azulado' },
  { token: 'cat-8', label: 'Ciruela' },
  { token: 'other', label: 'Gris' },
];

/** CSS helper class of a colour token (`cat-3` -> `c3`, `other` -> `c0`). Empty for null. */
export function colorClass(token: ColorToken | null | undefined): string {
  if (!token) return '';
  if (token === 'other') return 'c0';
  return `c${token.slice(4)}`;
}

/** The categorical default of a top-level group: its position (0-based) in the list, first six only. */
export function defaultGroupClass(position: number): string {
  return position >= 0 && position < 6 ? `c${position + 1}` : '';
}

interface Colored {
  id: number;
  parentId: number | null;
  color: ColorToken | null;
}

/** Stored colour class of a category: its own, else its group's; empty when none is stored. */
export function storedColorClass(
  categories: ReadonlyArray<Colored>,
  categoryId: number | null,
  parentId: number | null = null,
): string {
  if (categoryId === null) return '';
  const own = categories.find((c) => c.id === categoryId);
  const topId = parentId ?? own?.parentId ?? categoryId;
  const group = categories.find((c) => c.id === topId);
  return colorClass(own?.color ?? group?.color ?? null);
}

/**
 * Helper class for a category dot: the stored colour (own, else the group's), else the default by the
 * group's position among the top-level categories (the rule Movimientos always used).
 */
export function categoryColorClass(
  categories: ReadonlyArray<Colored>,
  categoryId: number | null,
  parentId: number | null,
): string {
  if (categoryId === null) return '';
  const stored = storedColorClass(categories, categoryId, parentId);
  if (stored) return stored;
  const own = categories.find((c) => c.id === categoryId);
  const topId = parentId ?? own?.parentId ?? categoryId;
  const groups = categories.filter((c) => c.parentId === null);
  return defaultGroupClass(groups.findIndex((g) => g.id === topId));
}
