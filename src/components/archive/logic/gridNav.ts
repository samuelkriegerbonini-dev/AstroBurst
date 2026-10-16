export interface NavKey {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
}

export interface NavResult {
  index: number;
  atEnd: boolean;
}

export interface GalleryGrid {
  count: number;
  columns: number;
  pageRows: number;
}

export interface TableGrid {
  count: number;
  pageRows: number;
}

export function galleryColumns(width: number, cardWidth: number, gap: number): number {
  return Math.max(1, Math.floor((width + gap) / (cardWidth + gap)));
}

function clamp(value: number, max: number): number {
  return Math.max(0, Math.min(max, value));
}

export function galleryMove(index: number, e: NavKey, grid: GalleryGrid): NavResult | null {
  const { count } = grid;
  if (count <= 0) return null;
  const columns = Math.max(1, grid.columns);
  const last = count - 1;
  const from = clamp(index, last);
  const rowStart = from - (from % columns);
  const lastRowStart = last - (last % columns);
  const ctrl = e.ctrlKey || e.metaKey;
  let target: number;
  switch (e.key) {
    case "ArrowRight":
      target = from + 1;
      break;
    case "ArrowLeft":
      target = from - 1;
      break;
    case "ArrowDown":
      target = rowStart + columns <= last ? Math.min(from + columns, last) : from;
      break;
    case "ArrowUp":
      target = from - columns >= 0 ? from - columns : from;
      break;
    case "Home":
      target = ctrl ? 0 : rowStart;
      break;
    case "End":
      target = ctrl ? last : Math.min(rowStart + columns - 1, last);
      break;
    case "PageDown":
      target = from + columns * Math.max(1, grid.pageRows);
      break;
    case "PageUp":
      target = from - columns * Math.max(1, grid.pageRows);
      break;
    default:
      return null;
  }
  const next = clamp(target, last);
  return { index: next, atEnd: next >= lastRowStart };
}

export function tableMove(index: number, e: NavKey, grid: TableGrid): NavResult | null {
  const { count } = grid;
  if (count <= 0) return null;
  const last = count - 1;
  const from = clamp(index, last);
  const page = Math.max(1, grid.pageRows);
  let target: number;
  switch (e.key) {
    case "ArrowDown":
      target = from + 1;
      break;
    case "ArrowUp":
      target = from - 1;
      break;
    case "Home":
      target = 0;
      break;
    case "End":
      target = last;
      break;
    case "PageDown":
      target = from + page;
      break;
    case "PageUp":
      target = from - page;
      break;
    default:
      return null;
  }
  const next = clamp(target, last);
  return { index: next, atEnd: next === last };
}
