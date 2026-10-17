import { FILE_STATUS } from "./constants";

interface StatusRow {
  status: string;
}

interface SelectableRow extends StatusRow {
  id: string;
}

export function isSelectableStatus(status: string): boolean {
  return status === FILE_STATUS.DONE || status === FILE_STATUS.SPECTRUM;
}

export function selectableRows<T extends StatusRow>(rows: readonly T[]): T[] {
  return rows.filter((row) => isSelectableStatus(row.status));
}

export function fallbackSelectionId(rows: readonly SelectableRow[], selectedId: string | null, filtersActive: boolean): string | null {
  if (!selectedId) return null;
  if (!filtersActive || rows.some((row) => row.id === selectedId)) return selectedId;
  return rows.find((row) => isSelectableStatus(row.status))?.id ?? null;
}

export function nextSelectableIndex(rows: readonly StatusRow[], currentIndex: number, dir: 1 | -1): number | null {
  let index = currentIndex === -1 ? (dir === 1 ? -1 : rows.length) : currentIndex;
  for (;;) {
    index += dir;
    if (index < 0 || index >= rows.length) return null;
    if (isSelectableStatus(rows[index].status)) return index;
  }
}
