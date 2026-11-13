import type { CompareColumn } from "../../shared/types/catalogCompare";

export function columnOptions(columns: readonly CompareColumn[], current: string, numericOnly: boolean): string[] {
  const names = columns.filter((column) => !numericOnly || column.numeric).map((column) => column.name);
  return current !== "" && !names.includes(current) ? [...names, current] : names;
}
