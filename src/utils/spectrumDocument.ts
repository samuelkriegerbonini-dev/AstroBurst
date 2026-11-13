import type { PaletteAction } from "../components/CommandPalette";
import type { DqTableName } from "../shared/types/dq";
import type { FileSpectrumSource, SpectralAxisInfo } from "../shared/types/spectral";
import type { Spectrum1d, TsoSeriesPick, WavelengthFrame, WavelengthFrameChoice } from "../shared/types/spectrum1d";

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
const NIRISS_SOSS_DETECTOR = "NIS";
const CSV_PATH_RE = /\.csv$/i;
const ASSUMED_FRAME_NOTE = /(air|vacuum) wavelengths assumed/;
const FRAME_ORIGIN_NOTE = /^(.+?): (air|vacuum) wavelengths/;
const DQ_EXCLUSION_MASKS: Record<DqTableName, number> = { jwst: 1, roman: 1, hst: 0x3fff, unknown: 0 };
const INSTRUMENT_CARDS: readonly (readonly [string, string])[] = [
  ["INSTRUME", "instrument"],
  ["GRATING", "disperser"],
  ["FILTER", "filter"],
  ["DETECTOR", "detector"],
];
const TARGET_CARDS = ["TARGNAME", "OBJECT", "target"];
const ERROR_ORIGINS: Record<Spectrum1d["source"]["kind"], string> = {
  x1d: "errors: FLUX_ERROR column",
  x1dints: "errors: FLUX_ERROR column",
  wcs1d: "errors: ERR extension",
  csv: "errors: err column",
};
const MISSING_UNIT_NOTE = /CUNIT1 (is )?(missing|absent)/;

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

export function documentFrameOrigin(spectrum: Spectrum1d): string | null {
  if (spectrum.wavelength_frame === "not_wavelength") return null;
  const match = spectrum.notes.map((note) => FRAME_ORIGIN_NOTE.exec(note)).find((found) => found !== null);
  if (!match || ASSUMED_FRAME_NOTE.test(match.input)) return null;
  return match[1].trim();
}

export function dqExclusionMask(table: DqTableName): number {
  return DQ_EXCLUSION_MASKS[table];
}

export function documentDqExclusions(spectrum: Spectrum1d): boolean[] | null {
  const mask = dqExclusionMask(spectrum.dq_table);
  if (spectrum.dq === null || mask === 0) return null;
  return spectrum.dq.map((bits) => (bits & mask) !== 0);
}

function cardText(cards: Record<string, string>, keys: readonly string[]): string | null {
  for (const key of keys) {
    const text = cards[key]?.trim();
    if (text) return text;
  }
  return null;
}

function dqDetails(spectrum: Spectrum1d): string {
  if (spectrum.dq === null) return "no DQ";
  const excluded = documentDqExclusions(spectrum);
  if (excluded === null) return "DQ: unknown flag table, not used";
  const count = excluded.filter(Boolean).length;
  return `DQ: ${spectrum.dq_table} flags, ${count} ${count === 1 ? "channel" : "channels"} excluded`;
}

export function documentDetails(spectrum: Spectrum1d): string {
  const { cards } = spectrum;
  const instrument = INSTRUMENT_CARDS.map((keys) => cardText(cards, keys))
    .filter((text) => text !== null)
    .join(" ");
  const target = cardText(cards, TARGET_CARDS);
  const sourceType = cardText(cards, ["SRCTYPE"]);
  return [
    instrument || null,
    target === null ? null : `target ${target}`,
    sourceType === null ? null : `source type ${sourceType}`,
    spectrum.err === null ? "no errors" : ERROR_ORIGINS[spectrum.source.kind],
    dqDetails(spectrum),
  ]
    .filter((part) => part !== null)
    .join(", ");
}

export function wavelengthUnitAssumed(spectrum: Spectrum1d): boolean {
  if (spectrum.wavelength_unit_source !== undefined) return spectrum.wavelength_unit_source === "assumed";
  return [...spectrum.notes, ...spectrum.axis.notes].some((note) => MISSING_UNIT_NOTE.test(note));
}

export function documentSourcePath(source: Spectrum1d["source"]): string {
  return source.kind === "x1dints" ? (source.paths[0] ?? "") : source.path;
}

export function documentSourceHdu(source: Spectrum1d["source"]): number | null {
  return source.kind === "x1d" || source.kind === "wcs1d" ? source.hdu : null;
}

export function seriesText(detector: string, sporder: number | null): string {
  const ordersShareDetector = sporder !== null && (sporder >= 2 || (sporder === 1 && detector === NIRISS_SOSS_DETECTOR));
  return ordersShareDetector ? `${detector}, order ${sporder}` : detector;
}

export function seriesPickText(pick: TsoSeriesPick): string {
  switch (pick.kind) {
    case "integration":
      return `integration ${pick.index + 1}`;
    case "median":
      return "median of all integrations";
    case "mean":
      return `mean of integrations ${pick.first + 1} to ${pick.last + 1}`;
  }
}

export function documentChannelCount(spectrum: Spectrum1d): number {
  return spectrum.flux.length;
}

export function documentLayout(source: Spectrum1d["source"]): string {
  switch (source.kind) {
    case "x1d":
      return `EXTRACT1D table in HDU ${source.hdu}`;
    case "wcs1d":
      return `1D array in HDU ${source.hdu}`;
    case "csv":
      return "CSV file";
    case "x1dints":
      return `time-series spectra, ${seriesText(source.detector, source.sporder)}: ${seriesPickText(source.pick)}`;
  }
}

export function documentSummary(spectrum: Spectrum1d): string {
  return `${documentChannelCount(spectrum)} channels, ${documentLayout(spectrum.source)}, flux in ${spectrum.flux_unit ?? NATIVE_FLUX_UNIT}`;
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
    hint: "Spectra tool",
    keywords: ["spectrum", "spectra", "csv", "1d", "line", "cube"],
    run,
  };
}
