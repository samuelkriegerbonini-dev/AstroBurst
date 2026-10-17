const FILTER_TOKEN = /^f\d{3,4}[a-z]/i;
const SEPARATORS = /[_-]+/;
const LAST_THREE_TOKENS = /[^_-]+[_-]+[^_-]+[_-]+[^_-]+$/;
const DROPPED = "...";
const GZIP_FITS = /\.(?:fits|fit|fts)\.gz$/i;
const LAST_EXTENSION = /\.[^.]+$/;

export interface FileListEntry {
  name: string;
  path: string;
}

export interface FileListName {
  text: string;
  clipStart: boolean;
}

function stemOf(name: string): string {
  return name.replace(GZIP_FITS.test(name) ? GZIP_FITS : LAST_EXTENSION, "") || name;
}

function distinctNames(entries: readonly FileListEntry[]): string[] | null {
  const names = entries.map((entry) => entry.name);
  return new Set(names).size === names.length ? names : null;
}

function tokensOf(text: string): string[] {
  return text.split(SEPARATORS).filter(Boolean);
}

function shortForm(stem: string): string {
  const fields = stem.split("_");
  const filterField = fields.findIndex((field) => field.split("-").some((token) => FILTER_TOKEN.test(token)));
  if (filterField > 0) return `${DROPPED}${fields.slice(filterField).join("_")}`;
  if (filterField === 0 || tokensOf(stem).length <= 3) return stem;
  const cut = stem.search(LAST_THREE_TOKENS);
  return cut > 0 ? `${DROPPED}${stem.slice(cut)}` : stem;
}

function headTokens(stem: string, tail: string): string[] {
  return tokensOf(stem.slice(0, stem.length - tail.length));
}

function countOf(values: readonly string[]): Map<string, number> {
  const counts = new Map<string, number>();
  values.forEach((value) => counts.set(value, (counts.get(value) ?? 0) + 1));
  return counts;
}

function rowsByValue(values: readonly string[]): Map<string, number[]> {
  const rowsOf = new Map<string, number[]>();
  values.forEach((value, row) => {
    const rows = rowsOf.get(value);
    if (rows) rows.push(row);
    else rowsOf.set(value, [row]);
  });
  return rowsOf;
}

function nameLabels(stems: readonly string[]): FileListName[] {
  const shown = stems.map(shortForm);
  const collided = new Set<number>();
  for (const [text, rows] of rowsByValue(shown)) {
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
      collided.add(row);
      shown[row] = (pickedCount.get(picked[k]) ?? 0) > 1 ? stems[row] : picked[k];
    });
  }
  return shown.map((text, row) => ({ text, clipStart: text === stems[row] && !collided.has(row) }));
}

function pathParts(path: string): string[] {
  return path.split(/[\\/]+/).filter(Boolean);
}

function withFolders(text: string, entries: readonly FileListEntry[]): string[] | null {
  const keys = entries.map((entry) => pathParts(entry.path).join("/"));
  const distinct = [...new Set(keys)];
  if (distinct.length < 2) return null;
  const folders = distinct.map((key) => pathParts(key).slice(0, -1));
  const deepest = Math.max(0, ...folders.map((names) => names.length));
  for (let depth = 1; depth <= deepest; depth++) {
    const texts = folders.map((names) => [...names.slice(-depth), text].join("/"));
    if (new Set(texts).size < texts.length) continue;
    const labelOf = new Map(distinct.map((key, k) => [key, texts[k]]));
    return keys.map((key) => labelOf.get(key) ?? text);
  }
  return null;
}

export function fileListNames(entries: readonly FileListEntry[]): FileListName[] {
  const stems = entries.map((entry) => stemOf(entry.name));
  const distinct = [...new Set(stems)];
  const labelOf = new Map(nameLabels(distinct).map((label, k) => [distinct[k], label]));
  const labels = stems.map((stem) => labelOf.get(stem) ?? { text: stem, clipStart: false });
  for (const rows of rowsByValue(labels.map((label) => label.text)).values()) {
    if (rows.length < 2) continue;
    const listed = rows.map((row) => entries[row]);
    const texts = withFolders(labels[rows[0]].text, listed) ?? distinctNames(listed);
    if (texts) rows.forEach((row, k) => {
      labels[row] = { text: texts[k], clipStart: false };
    });
  }
  return labels;
}
