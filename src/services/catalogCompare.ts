import { typedInvoke } from "../infrastructure/tauri";
import type { CompareProbe, CompareReport, CompareRequest } from "../shared/types/catalogCompare";

export function probeCatalogueTable(tablePath: string): Promise<CompareProbe> {
  return typedInvoke<CompareProbe>("catalog_compare_probe_cmd", { tablePath });
}

export function compareWithCatalogue(request: CompareRequest): Promise<CompareReport> {
  return typedInvoke<CompareReport>("catalog_compare_cmd", { request });
}
