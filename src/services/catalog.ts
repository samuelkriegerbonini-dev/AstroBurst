import { typedInvoke } from "../infrastructure/tauri";
import type {
  CatalogCsvKind,
  CatalogExportResult,
  ConeSearchOptions,
  ConeSearchResult,
  CrossMatchEntry,
  CrossMatchOptions,
  CrossMatchResult,
  MeasuredSource,
  PlacedCatalogRow,
} from "../shared/types/catalog";

export function catalogConeSearch(path: string, options: ConeSearchOptions = {}): Promise<ConeSearchResult> {
  return typedInvoke<ConeSearchResult>("catalog_cone_search_cmd", {
    path,
    radiusArcmin: options.radiusArcmin ?? null,
    magLimit: options.magLimit ?? null,
    maxRows: options.maxRows ?? null,
  });
}

export function catalogCrossMatch(path: string, options: CrossMatchOptions = {}): Promise<CrossMatchResult> {
  return typedInvoke<CrossMatchResult>("catalog_crossmatch_cmd", {
    path,
    sigma: options.sigma ?? null,
    maxStars: options.maxStars ?? null,
    radiusArcsec: options.radiusArcsec ?? null,
    band: options.band ?? null,
    colourTerm: options.colourTerm ?? null,
    apertureRadius: options.apertureRadius ?? null,
  });
}

export type CatalogExportItems =
  | { kind: "catalog"; rows: PlacedCatalogRow[] }
  | { kind: "sources"; rows: MeasuredSource[] }
  | { kind: "matches"; matches: CrossMatchEntry[] };

export function catalogExportCsv(path: string, outputPath: string, items: CatalogExportItems): Promise<CatalogExportResult> {
  const kind: CatalogCsvKind = items.kind;
  return typedInvoke<CatalogExportResult>("catalog_export_csv_cmd", {
    path,
    outputPath,
    kind,
    rows: items.kind === "matches" ? null : items.rows,
    matches: items.kind === "matches" ? items.matches : null,
  });
}
