import { typedInvoke } from "../infrastructure/tauri";
import type { HeaderData } from "../shared/types/header";

export function getHeader(path: string): Promise<Record<string, string>> {
  return typedInvoke<Record<string, string>>("get_header", { path });
}

export function getFullHeader(path: string): Promise<HeaderData> {
  return typedInvoke<HeaderData>("get_full_header", { path });
}

export interface FitsExtension {
  index: number;
  name: string;
  ext_type: string;
  naxis: number[];
  bitpix: number;
  card_count: number;
}

export function getFitsExtensions(path: string): Promise<FitsExtension[]> {
  return typedInvoke<FitsExtension[]>("get_fits_extensions", { path });
}

export function getHeaderByHdu(path: string, hduIndex: number): Promise<HeaderData> {
  return typedInvoke<HeaderData>("get_header_by_hdu", { path, hduIndex });
}

export interface NarrowbandDetection {
  palette: {
    r_file: { file_path: string; file_name: string; detection: { filter_name: string; method: string; confidence: number } | null } | null;
    g_file: { file_path: string; file_name: string; detection: { filter_name: string; method: string; confidence: number } | null } | null;
    b_file: { file_path: string; file_name: string; detection: { filter_name: string; method: string; confidence: number } | null } | null;
    unmapped: Array<{ file_path: string; file_name: string; detection: { filter_name: string; method: string; confidence: number } | null }>;
    is_complete: boolean;
    palette_name: string;
  };
}

export function detectNarrowbandFilters(paths: string[], palette?: string): Promise<NarrowbandDetection> {
  return typedInvoke<NarrowbandDetection>("detect_narrowband_filters", { paths, palette: palette ?? null });
}
