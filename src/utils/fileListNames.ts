const FILTER_TOKEN = /^f\d{3,4}[a-z]/i;
const LAST_THREE_TOKENS = /[^_-]+[_-][^_-]+[_-][^_-]+$/;
const DROPPED = "...";

function stemOf(name: string): string {
  return name.replace(/\.[^.]+$/, "") || name;
}

function shortForm(stem: string): string {
  const fields = stem.split("_");
  const filterField = fields.findIndex((field) => field.split("-").some((token) => FILTER_TOKEN.test(token)));
  if (filterField > 0) return `${DROPPED}${fields.slice(filterField).join("_")}`;
  if (filterField === 0 || stem.split(/[_-]/).length <= 3) return stem;
  const cut = stem.search(LAST_THREE_TOKENS);
  return cut > 0 ? `${DROPPED}${stem.slice(cut)}` : stem;
}

function headTokens(stem: string, tail: string): string[] {
  return stem.slice(0, stem.length - tail.length).split(/[_-]/).filter(Boolean);
}

function countOf(values: readonly string[]): Map<string, number> {
  const counts = new Map<string, number>();
  values.forEach((value) => counts.set(value, (counts.get(value) ?? 0) + 1));
  return counts;
}

export function fileListNames(names: readonly string[]): string[] {
  const stems = names.map(stemOf);
  const shown = stems.map(shortForm);
  const rowsByText = new Map<string, number[]>();
  shown.forEach((text, row) => {
    const rows = rowsByText.get(text);
    if (rows) rows.push(row);
    else rowsByText.set(text, [row]);
  });
  for (const [text, rows] of rowsByText) {
    if (rows.length < 2 || !text.startsWith(DROPPED)) continue;
    const tail = text.slice(DROPPED.length);
    const heads = rows.map((row) => headTokens(stems[row], tail));
    let differing = 0;
    while (heads.every((head) => head[differing] !== undefined && head[differing] === heads[0][differing])) differing++;
    const picked = rows.map((row, k) => {
      const token = heads[k][differing];
      return token === undefined ? stems[row] : `${token}${DROPPED}${tail}`;
    });
    const pickedCount = countOf(picked);
    rows.forEach((row, k) => {
      shown[row] = (pickedCount.get(picked[k]) ?? 0) > 1 ? stems[row] : picked[k];
    });
  }
  return shown;
}
