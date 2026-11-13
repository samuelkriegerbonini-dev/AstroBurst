import { typedInvoke } from "../infrastructure/tauri";
import type { FilterIdentification, IdentifiedFilter } from "../shared/types/filters";

export function identifyFilter(path: string, dataset?: string | null): Promise<FilterIdentification> {
  return typedInvoke<FilterIdentification>("identify_filter_cmd", { path, dataset: dataset ?? null });
}

export function isIdentifiedFilter(answer: FilterIdentification): answer is IdentifiedFilter {
  return !("error" in answer);
}
