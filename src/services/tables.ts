import { typedInvoke } from "../infrastructure/tauri";
import type { TableProbe, TableResult } from "../shared/types/tables";

export function probeTable(path: string): Promise<TableProbe> {
  return typedInvoke<TableProbe>("probe_table_cmd", { path });
}

export function readTable(path: string, columns: readonly string[] | null = null): Promise<TableResult> {
  return typedInvoke<TableResult>("read_table_cmd", { path, columns });
}
