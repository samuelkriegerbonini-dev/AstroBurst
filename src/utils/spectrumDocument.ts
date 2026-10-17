import type { PaletteAction } from "../components/CommandPalette";
import type { FileSpectrumSource, SpectralAxisInfo } from "../shared/types/spectral";
import type { Spectrum1d, WavelengthFrame, WavelengthFrameChoice } from "../shared/types/spectrum1d";

export interface SpectrumDocument {
  source: FileSpectrumSource;
  spectrum: Spectrum1d;
}

export interface OpenSpectrumCsv {
  path: string;
  over: string | null;
}

export type CubeToolContent = "cube" | "spectrum" | "csv" | "empty";

export interface CubeToolContentInput {
  isCube: boolean;
  isRamp: boolean;
  isSpectrum: boolean;
  fileKey: string | null;
  csv: OpenSpectrumCsv | null;
}

export const OPEN_SPECTRUM_CSV_LABEL = "Open spectrum (CSV)";

export const AIR_FRAME_FORMULA_NOTE =
  "line velocities convert the vacuum rest wavelength to air with Morton (2000); the plot converts between air and vacuum with Greisen et al. 2006";

export const SPECTRUM_FILE_CORRECTION_NOTE = "The correction is not computed for a spectrum file.";

const NOT_WAVELENGTH_FRAME ="the spectral axis is not a wavelength: no air or vacuum frame applies";
const NATIVE_FLUX_UNIT = "native units";
const CSV_PATH_RE = /\.csv$/i;

export function isSpectrumCsvPath(path: string): boolean {
  return CSV_PATH_RE.test(path);
}

export function defaultFrameChoice(frame: WavelengthFrame | null | undefined): WavelengthFrameChoice | null {
  return frame === "air" || frame === "vacuum" ? frame : null;
}

export function documentAxis(axis: SpectralAxisInfo, choice: WavelengthFrameChoice | null): SpectralAxisInfo {
  if (choice === "air" && axis.kind === "wave") return { ...axis, kind: "awav" };
  if (choice === "vacuum" && axis.kind === "awav") return { ...axis, kind: "wave" };
  return axis;
}

export function documentFrameLine(fileFrame: WavelengthFrame, choice: WavelengthFrameChoice | null): string {
  if (fileFrame === "not_wavelength" || choice === null) return NOT_WAVELENGTH_FRAME;
  if (choice === fileFrame) return `${choice} wavelengths, as the file reads`;
  return `${choice} wavelengths, set here (the file reads as ${fileFrame})`;
}

export function documentLayout(source: Spectrum1d["source"]): string {
  switch (source.kind) {
    case "x1d":
      return `EXTRACT1D table in HDU ${source.hdu}`;
    case "wcs1d":
      return `1D array in HDU ${source.hdu}`;
    case "csv":
      return "CSV file";
  }
}

export function documentSummary(spectrum: Spectrum1d): string {
  return `${spectrum.n_rows} channels, ${documentLayout(spectrum.source)}, flux in ${spectrum.flux_unit ?? NATIVE_FLUX_UNIT}`;
}

export function cubeToolContent({ isCube, isRamp, isSpectrum, fileKey, csv }: CubeToolContentInput): CubeToolContent {
  if (csv !== null && csv.over === fileKey) return "csv";
  if (isCube || isRamp) return "cube";
  if (isSpectrum) return "spectrum";
  return csv !== null ? "csv" : "empty";
}

export function openSpectrumCsvPaletteAction(run: () => void): PaletteAction {
  return {
    id: "open-spectrum-csv",
    label: `${OPEN_SPECTRUM_CSV_LABEL}...`,
    hint: "Cube tool",
    keywords: ["spectrum", "csv", "1d", "line", "cube"],
    run,
  };
}
