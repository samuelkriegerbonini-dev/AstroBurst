import { typedInvoke } from "../infrastructure/tauri";
import type { HeaderData } from "../shared/types/header";
import type { PlaneKind } from "../shared/types/fits.types";

export function getHeader(path: string): Promise<Record<string, string>> {
  return typedInvoke<Record<string, string>>("get_header", { path });
}

export function getFullHeader(path: string, palette?: string): Promise<HeaderData> {
  return typedInvoke<HeaderData>("get_full_header", { path, palette: palette ?? null });
}

export interface FitsExtension {
  index: number;
  extname: string | null;
  naxis: number;
  naxis1: number;
  naxis2: number;
  naxis3: number;
  bitpix: number;
  has_data: boolean;
  extver: number | null;
  ref: string;
  kind: PlaneKind;
  is_dq: boolean;
  is_err: boolean;
}

export function getFitsExtensions(path: string): Promise<{ extensions: FitsExtension[] }> {
  return typedInvoke<{ extensions: FitsExtension[] }>("get_fits_extensions", { path });
}

export interface HduRawHeader {
  cards: [string, string][];
  index: Record<string, string>;
}

export function getHeaderByHdu(path: string, hduIndex: number): Promise<HduRawHeader> {
  return typedInvoke<HduRawHeader>("get_header_by_hdu", { path, hduIndex });
}

export type DetectionConfidence = "High" | "Medium" | "Low";

export interface NarrowbandFilterDetection {
  path: string;
  filter: string | null;
  hubble_channel?: string | null;
  confidence?: DetectionConfidence;
  matched_keyword?: string;
  matched_value?: string;
}

export interface SuggestedChannelDetection {
  filter: string;
  confidence: DetectionConfidence;
  matched_keyword: string;
  matched_value: string;
}

export interface NarrowbandChannelSuggestion {
  file_path: string;
  file_name: string;
  detection: SuggestedChannelDetection | null;
}

export interface NarrowbandDetection {
  filters: NarrowbandFilterDetection[];
  palette: {
    r_file: NarrowbandChannelSuggestion | null;
    g_file: NarrowbandChannelSuggestion | null;
    b_file: NarrowbandChannelSuggestion | null;
    unmapped: NarrowbandChannelSuggestion[];
    is_complete: boolean;
    palette_name: string;
  };
}

export function detectNarrowbandFilters(paths: string[], palette?: string): Promise<NarrowbandDetection> {
  return typedInvoke<NarrowbandDetection>("detect_narrowband_filters", { paths, palette: palette ?? null });
}
