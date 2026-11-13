import { typedInvoke } from "../infrastructure/tauri";
import type { Spectrum1d, Spectrum1dWire } from "../shared/types/spectrum1d";
import type {
  SeriesKey,
  Spectrum1dTables,
  TsoLightCurves,
  TsoLightCurvesRequest,
  TsoOpenResult,
  TsoPick,
  WhtltComparison,
  WhtltImport,
} from "../shared/types/tso";
import { fromSpectrum1dWire } from "./spectrum1d";

export function spectrum1dTables(path: string): Promise<Spectrum1dTables> {
  return typedInvoke<Spectrum1dTables>("spectrum1d_tables_cmd", { path });
}

export function tsoOpen(paths: string[]): Promise<TsoOpenResult> {
  return typedInvoke<TsoOpenResult>("tso_open_cmd", { paths });
}

export async function tsoSpectrum(paths: string[], series: SeriesKey, pick: TsoPick): Promise<Spectrum1d> {
  return fromSpectrum1dWire(await typedInvoke<Spectrum1dWire>("tso_spectrum_cmd", { paths, series, pick }));
}

export function tsoLightCurves(
  paths: string[],
  series: SeriesKey,
  request: TsoLightCurvesRequest,
): Promise<TsoLightCurves> {
  return typedInvoke<TsoLightCurves>("tso_light_curves_cmd", { paths, series, request });
}

export function whtltImport(path: string): Promise<WhtltImport> {
  return typedInvoke<WhtltImport>("whtlt_import_cmd", { path });
}

export function tsoWhtltCompare(paths: string[], whtltPath: string): Promise<WhtltComparison[]> {
  return typedInvoke<WhtltComparison[]>("tso_whtlt_compare_cmd", { paths, whtltPath });
}
