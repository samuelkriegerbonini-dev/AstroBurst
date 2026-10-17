import type { DqTableName } from "./dq";
import type { SpectralAxisInfo } from "./spectral";

export type Spectrum1dKind = { kind: "x1d"; hdu: number } | { kind: "wcs1d"; hdu: number };

export type Spectrum1dSource =
  | { kind: "x1d"; path: string; hdu: number; extver: number | null }
  | { kind: "wcs1d"; path: string; hdu: number }
  | { kind: "csv"; path: string };

export type WavelengthFrame = "air" | "vacuum" | "not_wavelength";

export type WavelengthFrameChoice = Exclude<WavelengthFrame, "not_wavelength">;

export type SpectrumPrecision = "f64" | "f32_widened";

export interface Spectrum1dAxisWire extends Omit<SpectralAxisInfo, "values" | "crval" | "cdelt"> {
  values: (number | null)[];
  crval: number | null;
  cdelt: number | null;
}

export interface Spectrum1dWire {
  source: Spectrum1dSource;
  axis: Spectrum1dAxisWire;
  wavelength_frame: WavelengthFrame;
  precision: SpectrumPrecision;
  flux: (number | null)[];
  err: (number | null)[] | null;
  dq: number[] | null;
  dq_table: DqTableName;
  flux_unit: string | null;
  n_rows: number;
  cards: Record<string, string>;
  notes: string[];
}

export interface Spectrum1d extends Omit<Spectrum1dWire, "axis" | "flux" | "err"> {
  axis: SpectralAxisInfo;
  flux: number[];
  err: number[] | null;
}
