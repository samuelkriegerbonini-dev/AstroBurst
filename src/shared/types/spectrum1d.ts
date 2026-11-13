import type { DqTableName } from "./dq";
import type { SpectralAxisInfo } from "./spectral";

export type TsoSeriesPick =
  | { kind: "integration"; index: number }
  | { kind: "median" }
  | { kind: "mean"; first: number; last: number };

export type Spectrum1dKind =
  | { kind: "x1d"; hdu: number; n_tables?: number }
  | { kind: "wcs1d"; hdu: number }
  | { kind: "x1dints"; layout: "rows" | "extensions"; n_series: number; n_integrations: number; n_channels: number };

export type Spectrum1dSource =
  | { kind: "x1d"; path: string; hdu: number; extver: number | null }
  | { kind: "wcs1d"; path: string; hdu: number }
  | { kind: "csv"; path: string }
  | { kind: "x1dints"; paths: string[]; detector: string; sporder: number | null; pick: TsoSeriesPick };

export type WavelengthUnitSource = "file" | "assumed" | "user";
export type WavelengthDisplayUnit = "um" | "nm" | "A";
export type FluxDisplayChoice = "native" | "Jy" | "mJy" | "uJy" | "flam_cgs" | "flam_si" | "ab";

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
  wavelength_unit_source?: WavelengthUnitSource;
}

export interface Spectrum1d extends Omit<Spectrum1dWire, "axis" | "flux" | "err"> {
  axis: SpectralAxisInfo;
  flux: number[];
  err: number[] | null;
}
