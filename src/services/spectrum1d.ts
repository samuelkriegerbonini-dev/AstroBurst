import { typedInvoke } from "../infrastructure/tauri";
import type { Spectrum1d, Spectrum1dKind, Spectrum1dWire, TsoSeriesPick } from "../shared/types/spectrum1d";
import { isSpectrumCsvPath } from "../utils/spectrumDocument";

export { isSpectrumCsvPath };

export type WavelengthUnitOverride = "um" | "nm" | "Angstrom";

export interface SpectrumTableOption {
  hdu: number;
  extver: number | null;
  n_rows: number;
  n_channels: number;
  sporder: number | null;
  detector: string | null;
  slit_name: string | null;
  source_id: number | null;
  source_name: string | null;
  int_num: number | null;
  srctype: string | null;
  label: string;
}

export interface SpectrumSeriesOption {
  key: { detector: string; sporder: number | null };
  label: string;
  n_integrations: number;
  n_channels: number;
  flux_unit: string;
}

export interface SpectrumSeriesSet {
  set_id: string;
  series: SpectrumSeriesOption[];
  notes: string[];
  warnings: string[];
}

export type SpectrumRequest =
  | { kind: "file"; path: string; hdu: number | null; wavelengthUnit: WavelengthUnitOverride | null }
  | { kind: "series"; paths: string[]; detector: string; sporder: number | null; pick: TsoSeriesPick };

function nanForNull(value: number | null): number {
  return value ?? Number.NaN;
}

export function fromSpectrum1dWire(wire: Spectrum1dWire): Spectrum1d {
  return {
    ...wire,
    axis: {
      ...wire.axis,
      values: wire.axis.values.map(nanForNull),
      crval: nanForNull(wire.axis.crval),
      cdelt: nanForNull(wire.axis.cdelt),
    },
    flux: wire.flux.map(nanForNull),
    err: wire.err ? wire.err.map(nanForNull) : null,
  };
}

export async function readSpectrum1d(
  path: string,
  hdu: number | null = null,
  wavelengthUnit: WavelengthUnitOverride | null = null,
): Promise<Spectrum1d> {
  const args = wavelengthUnit === null ? { path, hdu } : { path, hdu, wavelengthUnit };
  return fromSpectrum1dWire(await typedInvoke<Spectrum1dWire>("read_spectrum1d_cmd", args));
}

export async function readSpectrumCsv(path: string): Promise<Spectrum1d> {
  return fromSpectrum1dWire(await typedInvoke<Spectrum1dWire>("read_spectrum_csv_cmd", { path }));
}

export function readSpectrumDocument(
  path: string,
  hdu: number | null = null,
  wavelengthUnit: WavelengthUnitOverride | null = null,
): Promise<Spectrum1d> {
  return isSpectrumCsvPath(path) ? readSpectrumCsv(path) : readSpectrum1d(path, hdu, wavelengthUnit);
}

export async function listSpectrumTables(path: string): Promise<SpectrumTableOption[]> {
  return (await typedInvoke<{ tables: SpectrumTableOption[] }>("spectrum1d_tables_cmd", { path })).tables;
}

export function listSpectrumSeries(paths: string[]): Promise<SpectrumSeriesSet> {
  return typedInvoke<SpectrumSeriesSet>("tso_open_cmd", { paths });
}

export async function readSeriesSpectrum(
  paths: string[],
  series: { detector: string; sporder: number | null },
  pick: TsoSeriesPick,
): Promise<Spectrum1d> {
  return fromSpectrum1dWire(await typedInvoke<Spectrum1dWire>("tso_spectrum_cmd", { paths, series, pick }));
}

export function readSpectrumRequest(request: SpectrumRequest): Promise<Spectrum1d> {
  if (request.kind === "series") {
    return readSeriesSpectrum(request.paths, { detector: request.detector, sporder: request.sporder }, request.pick);
  }
  return readSpectrumDocument(request.path, request.hdu, request.wavelengthUnit);
}

export function probeSpectrum1d(path: string): Promise<Spectrum1dKind | null> {
  return typedInvoke<Spectrum1dKind | null>("probe_spectrum1d_cmd", { path });
}
