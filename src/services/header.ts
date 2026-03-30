import { safeInvoke } from "../infrastructure/tauri";

export function getHeader(path: string) {
  return safeInvoke("get_header", { path });
}

export function getFullHeader(path: string) {
  return safeInvoke("get_full_header", { path });
}

export function getFitsExtensions(path: string) {
  return safeInvoke("get_fits_extensions", { path });
}

export function getHeaderByHdu(path: string, hduIndex: number) {
  return safeInvoke("get_header_by_hdu", { path, hduIndex });
}

export function detectNarrowbandFilters(paths: string[], palette?: string) {
  return safeInvoke("detect_narrowband_filters", { paths, palette: palette ?? null });
}
