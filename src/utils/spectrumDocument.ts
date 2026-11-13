import type { PaletteAction } from "../components/CommandPalette";
import type { DqTableName } from "../shared/types/dq";
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
  "air and vacuum wavelengths are converted with Morton (2000), for the plot and for line velocities";

export const SPECTRUM_FILE_CORRECTION_NOTE = "The correction is not computed for a spectrum file.";

const NOT_WAVELENGTH_FRAME ="the spectral axis is not a wavelength: no air or vacuum frame applies";
const NATIVE_FLUX_UNIT = "native units";
const CSV_PATH_RE = /\.csv$/i;
const ASSUMED_FRAME_NOTE = /(air|vacuum) wavelengths assumed/;
const DQ_EXCLUSION_MASKS: Record<DqTableName, number> = { jwst: 1, roman: 1, hst: 0x3fff, unknown: 0 };

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

export function documentFrameLine(spectrum: Spectrum1d, choice: WavelengthFrameChoice | null): string {
  const fileFrame = spectrum.wavelength_frame;
  if (fileFrame === "not_wavelength" || choice === null) return NOT_WAVELENGTH_FRAME;
  const assumed = spectrum.notes.some((note) => ASSUMED_FRAME_NOTE.test(note));
  if (choice === fileFrame) {
    return assumed ? `${choice} wavelengths, assumed (no card of the file names the frame)` : `${choice} wavelengths, as the file reads`;
  }
  return assumed
    ? `${choice} wavelengths, set here (${fileFrame} was assumed for the file)`
    : `${choice} wavelengths, set here (the file reads as ${fileFrame})`;
}

export function documentDqExclusions(spectrum: Spectrum1d): boolean[] | null {
  const mask = DQ_EXCLUSION_MASKS[spectrum.dq_table];
  if (spectrum.dq === null || mask === 0) return null;
  return spectrum.dq.map((bits) => (bits & mask) !== 0);
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
