import type { SpectralAxisInfo } from "../shared/types/spectral";
import { formatSignificant } from "./formatSignificant";
import { bandMarks, type BandMark } from "./lineList";
import { bandText, type DepthRow, type DepthSpectrum } from "./tsoLightCurves";

export const DEPTH_COLOR = "#fbbf24";
export const DEPTH_BAND_COLOR = "rgba(103,232,249,0.10)";

const WHITE_LIGHT = "white light";
const MISSING = "--";
const DEPTH_DIGITS = 4;
const ERROR_DIGITS = 2;
const PIXEL_DIGITS = 3;

export interface DepthPlot {
  series: { x: number[]; y: (number | null)[]; yErr: (number | null)[]; color: string; label: string; mode: "points" }[];
  referenceLines: { axis: "y"; value: number; label: string; dashed: true }[];
}

export interface DepthTableRow {
  key: string;
  band: string;
  depth: string;
  error: string;
  counts: string;
  pixels: string;
  pixelLoss: boolean;
}

export function depthAxis(lo: number, hi: number): SpectralAxisInfo {
  return {
    kind: "wave",
    ctype: "WAVE",
    unit: "um",
    header_unit: "um",
    header_scale: 1,
    values: [lo, hi],
    crval: lo,
    cdelt: hi - lo,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
    tabulated: true,
  };
}

export function depthBandMarks(species: readonly string[], lo: number | null, hi: number | null): BandMark[] {
  if (lo === null || hi === null || !Number.isFinite(lo) || !Number.isFinite(hi) || lo >= hi || species.length === 0) return [];
  return bandMarks({
    axis: depthAxis(lo, hi),
    mode: "wavelength_vac",
    restUm: null,
    convention: "optical",
    shiftKms: null,
    redshift: 0,
    families: [],
    species,
    xMin: lo,
    xMax: hi,
  });
}

function byCentre(rows: readonly DepthRow[]): DepthRow[] {
  return [...rows].sort((a, b) => a.centre - b.centre);
}

export function depthPlot(spectrum: DepthSpectrum): DepthPlot | null {
  if (!spectrum.rows.some((row) => row.depth !== null)) return null;
  const rows = byCentre(spectrum.rows);
  const whiteDepth = spectrum.white?.depth ?? null;
  return {
    series: [
      {
        x: rows.map((row) => row.centre),
        y: rows.map((row) => row.depth),
        yErr: rows.map((row) => row.err),
        color: DEPTH_COLOR,
        label: "depth",
        mode: "points",
      },
    ],
    referenceLines: whiteDepth === null ? [] : [{ axis: "y", value: whiteDepth, label: WHITE_LIGHT, dashed: true }],
  };
}

export function depthPlotBands(marks: readonly BandMark[]): { x0: number; x1: number; color: string; label: string }[] {
  return marks.map((mark) => ({ x0: mark.lo, x1: mark.hi, color: DEPTH_BAND_COLOR, label: mark.label }));
}

function numberText(value: number | null, digits: number): string {
  return value === null ? MISSING : formatSignificant(value, digits);
}

function tableRow(row: DepthRow, band: string): DepthTableRow {
  return {
    key: row.docId,
    band,
    depth: numberText(row.depth, DEPTH_DIGITS),
    error: numberText(row.err, ERROR_DIGITS),
    counts: `${row.nIn} / ${row.nOut}`,
    pixels: numberText(row.pixelRatio, PIXEL_DIGITS),
    pixelLoss: row.pixelLoss,
  };
}

export function depthTableRows(spectrum: DepthSpectrum): DepthTableRow[] {
  const bins = byCentre(spectrum.rows).map((row) => tableRow(row, bandText(row.band)));
  return spectrum.white ? [tableRow(spectrum.white, WHITE_LIGHT), ...bins] : bins;
}

export function bandBins(marks: readonly BandMark[]): [number, number][] {
  return marks.filter((mark) => mark.kind === "molecular_band" && mark.hi > mark.lo).map((mark) => [mark.lo, mark.hi]);
}
