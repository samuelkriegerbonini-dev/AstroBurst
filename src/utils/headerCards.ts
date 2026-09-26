const COMMENTARY_KEYS = new Set(["COMMENT", "HISTORY"]);
const COMMENTARY_CATEGORY = "processing";
const FALLBACK_CATEGORY = "other";

export interface HeaderCard {
  key: string;
  value: string;
}

export interface HeaderCardRow extends HeaderCard {
  index: number;
}

export interface HeaderCardGroup {
  category: string;
  rows: HeaderCardRow[];
}

export function isCommentaryKey(key: string): boolean {
  return COMMENTARY_KEYS.has(key.trim().toUpperCase());
}

export function groupHeaderCards(
  cards: readonly HeaderCard[],
  categories: Readonly<Record<string, Readonly<Record<string, string>>>>,
  order: readonly string[],
  query: string,
): HeaderCardGroup[] {
  const categoryOf = new Map<string, string>();
  for (const [category, entries] of Object.entries(categories)) {
    for (const key of Object.keys(entries)) categoryOf.set(key, category);
  }
  const q = query.trim().toLowerCase();
  const rowsByCategory = new Map<string, HeaderCardRow[]>(order.map((category) => [category, []]));
  cards.forEach((card, index) => {
    if (q && !card.key.toLowerCase().includes(q) && !card.value.toLowerCase().includes(q)) return;
    const category = isCommentaryKey(card.key) ? COMMENTARY_CATEGORY : categoryOf.get(card.key) ?? FALLBACK_CATEGORY;
    const rows = rowsByCategory.get(category) ?? rowsByCategory.get(FALLBACK_CATEGORY);
    rows?.push({ index, key: card.key, value: card.value });
  });
  return order
    .map((category) => ({ category, rows: rowsByCategory.get(category) ?? [] }))
    .filter((group) => group.rows.length > 0);
}

export function headerCardLine(key: string, value: string, keyWidth = 0): string {
  const name = key.padEnd(keyWidth);
  return isCommentaryKey(key) ? `${name.trimEnd()} ${value}` : `${name} = ${value}`;
}
