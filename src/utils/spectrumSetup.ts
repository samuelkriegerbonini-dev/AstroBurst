import type { ContinuumWindows } from "../shared/types/cube";
import type { LineModel, SpectralAxisMode, VelocityConvention } from "../shared/types/spectral";
import type { Spectrum1d, WavelengthDisplayUnit } from "../shared/types/spectrum1d";
import { DEFAULT_SPECTRUM_VIEW, type SpectrumViewState } from "../hooks/useSpectrumViewStore";
import type { LineFamily } from "./lineList";
import { documentLayout, documentSourcePath } from "./spectrumDocument";
import { channelWidthAt, type ChannelRange } from "./spectrumRange";
import { fileBaseName } from "./values";

export interface SpectrumSetup {
  fromKey: string;
  fromLabel: string;
  instrument: string;
  mode: SpectralAxisMode | null;
  restUm: number | null;
  convention: VelocityConvention;
  redshift: number;
  lineListVisible: boolean;
  lineFamilies: LineFamily[];
  bandSpecies: string[];
  pickedLineId: string | null;
  lineModel: LineModel;
  wavelengthUnit: WavelengthDisplayUnit;
  rangeUm: [number, number] | null;
  windowsUm: [[number, number], [number, number]] | null;
}

type CarriedSetup = Pick<
  SpectrumViewState,
  "mode" | "restUm" | "convention" | "redshift" | "lineListVisible" | "lineFamilies" | "bandSpecies" | "pickedLineId" | "lineModel" | "wavelengthUnit"
>;

const SETUP_FIELDS: readonly (keyof SpectrumViewState)[] = [
  "mode",
  "restUm",
  "convention",
  "redshift",
  "lineListVisible",
  "lineFamilies",
  "bandSpecies",
  "pickedLineId",
  "lineModel",
  "wavelengthUnit",
  "range",
  "windows",
];

const INSTRUMENT_CARDS = ["INSTRUME", "GRATING", "FILTER", "PUPIL"] as const;

function carried(from: CarriedSetup): CarriedSetup {
  return {
    mode: from.mode,
    restUm: from.restUm,
    convention: from.convention,
    redshift: from.redshift,
    lineListVisible: from.lineListVisible,
    lineFamilies: [...from.lineFamilies],
    bandSpecies: [...from.bandSpecies],
    pickedLineId: from.pickedLineId,
    lineModel: from.lineModel,
    wavelengthUnit: from.wavelengthUnit,
  };
}

function sameValue(a: unknown, b: unknown): boolean {
  if (Object.is(a, b)) return true;
  if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
  return a.every((value, i) => sameValue(value, b[i]));
}

function nearestFinite(values: readonly number[], target: number): number {
  let best = -1;
  let bestDistance = Infinity;
  values.forEach((v, i) => {
    if (!Number.isFinite(v)) return;
    const d = Math.abs(v - target);
    if (d < bestDistance) {
      bestDistance = d;
      best = i;
    }
  });
  return best;
}

export function channelRangeForWavelengths(vacuumUm: readonly number[], a: number, b: number): ChannelRange | null {
  if (!Number.isFinite(a) || !Number.isFinite(b)) return null;
  let lowest = -1;
  let highest = -1;
  vacuumUm.forEach((v, i) => {
    if (!Number.isFinite(v)) return;
    if (lowest < 0 || v < vacuumUm[lowest]) lowest = i;
    if (highest < 0 || v > vacuumUm[highest]) highest = i;
  });
  if (lowest < 0) return null;
  const values = vacuumUm as number[];
  const lo = vacuumUm[lowest] - channelWidthAt(values, lowest) / 2;
  const hi = vacuumUm[highest] + channelWidthAt(values, highest) / 2;
  if (!(a >= lo && a <= hi && b >= lo && b <= hi)) return null;
  const i = nearestFinite(vacuumUm, a);
  const j = nearestFinite(vacuumUm, b);
  return { z0: Math.min(i, j), z1: Math.max(i, j) };
}

function wavelengthsOf(vacuumUm: readonly number[] | null, z0: number, z1: number): [number, number] | null {
  if (vacuumUm === null || !(z0 >= 0 && z0 <= z1 && z1 < vacuumUm.length)) return null;
  const a = vacuumUm[z0];
  const b = vacuumUm[z1];
  return Number.isFinite(a) && Number.isFinite(b) ? [a, b] : null;
}

export function setupInstrument(spectrum: Spectrum1d): string {
  return INSTRUMENT_CARDS.map((card) => [card, (spectrum.cards[card] ?? "").trim().toUpperCase()] as const)
    .filter(([, value]) => value !== "")
    .map(([card, value]) => `${card}=${value}`)
    .join(" ");
}

export function setupCarriesTo(setup: SpectrumSetup, spectrum: Spectrum1d): boolean {
  return setup.instrument === setupInstrument(spectrum);
}

export function setupFromView(
  key: string,
  label: string,
  view: SpectrumViewState,
  vacuumUm: readonly number[] | null,
  instrument = "",
): SpectrumSetup {
  const rangeUm = view.range === null ? null : wavelengthsOf(vacuumUm, view.range.z0, view.range.z1);
  const first = view.windows === null ? null : wavelengthsOf(vacuumUm, view.windows[0][0], view.windows[0][1]);
  const second = view.windows === null ? null : wavelengthsOf(vacuumUm, view.windows[1][0], view.windows[1][1]);
  return {
    fromKey: key,
    fromLabel: label,
    instrument,
    ...carried(view),
    rangeUm,
    windowsUm: first !== null && second !== null ? [first, second] : null,
  };
}

export function viewFromSetup(setup: SpectrumSetup, vacuumUm: readonly number[] | null): Partial<SpectrumViewState> {
  const range = setup.rangeUm === null || vacuumUm === null ? null : channelRangeForWavelengths(vacuumUm, ...setup.rangeUm);
  const first = setup.windowsUm === null || vacuumUm === null ? null : channelRangeForWavelengths(vacuumUm, ...setup.windowsUm[0]);
  const second = setup.windowsUm === null || vacuumUm === null ? null : channelRangeForWavelengths(vacuumUm, ...setup.windowsUm[1]);
  const windows: ContinuumWindows | null =
    first !== null && second !== null
      ? [
          [first.z0, first.z1],
          [second.z0, second.z1],
        ]
      : null;
  return { ...carried(setup), range, windows };
}

export function setupIsDefault(view: SpectrumViewState): boolean {
  return SETUP_FIELDS.every((field) => sameValue(view[field], DEFAULT_SPECTRUM_VIEW[field]));
}

export function clearedSetup(): Partial<SpectrumViewState> {
  return { ...carried(DEFAULT_SPECTRUM_VIEW), range: null, windows: null };
}

export function setupLabel(spectrum: Spectrum1d): string {
  return `${fileBaseName(documentSourcePath(spectrum.source))}, ${documentLayout(spectrum.source)}`;
}
