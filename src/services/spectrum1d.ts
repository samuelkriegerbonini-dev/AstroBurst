import { typedInvoke } from "../infrastructure/tauri";
import type { Spectrum1d, Spectrum1dKind, Spectrum1dWire } from "../shared/types/spectrum1d";
import { isSpectrumCsvPath } from "../utils/spectrumDocument";

export { isSpectrumCsvPath };

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

export async function readSpectrum1d(path: string, hdu: number | null = null): Promise<Spectrum1d> {
  return fromSpectrum1dWire(await typedInvoke<Spectrum1dWire>("read_spectrum1d_cmd", { path, hdu }));
}

export async function readSpectrumCsv(path: string): Promise<Spectrum1d> {
  return fromSpectrum1dWire(await typedInvoke<Spectrum1dWire>("read_spectrum_csv_cmd", { path }));
}

export function readSpectrumDocument(path: string, hdu: number | null = null): Promise<Spectrum1d> {
  return isSpectrumCsvPath(path) ? readSpectrumCsv(path) : readSpectrum1d(path, hdu);
}

export function probeSpectrum1d(path: string): Promise<Spectrum1dKind | null> {
  return typedInvoke<Spectrum1dKind | null>("probe_spectrum1d_cmd", { path });
}
