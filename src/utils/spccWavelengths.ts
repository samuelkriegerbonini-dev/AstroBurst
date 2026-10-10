import type { ChannelSource } from "./channelMapping";
import { effectiveBinFiles, fileFilterBand, spccInputs, type WizardState } from "./wizard";

export type SpccTriplet = [number, number, number];

export interface SpccWavelengths {
  nm: SpccTriplet | null;
  codes: (string | null)[];
}

export const SPCC_DEFAULT_WAVELENGTHS_NM: SpccTriplet = [640, 530, 460];

export function spccWavelengths(
  state: WizardState,
  files: readonly (ChannelSource & { path: string })[],
): SpccWavelengths {
  const inputs = spccInputs(state);
  const bands = (["r", "g", "b"] as const).map((key) => {
    const input = inputs[key];
    const bin = input ? state.bins.find((b) => b.id === input.binId) : undefined;
    const path = bin ? effectiveBinFiles(state, bin)[0] : undefined;
    return path === undefined ? null : fileFilterBand(files.find((f) => f.path === path), path);
  });
  const [r, g, b] = bands;
  return {
    nm: r && g && b ? [r.nm, g.nm, b.nm] : null,
    codes: bands.map((band) => band?.code ?? null),
  };
}

export function spccWavelengthsLine(nm: SpccTriplet | null): string {
  return nm
    ? `Blackbody approximation at R/G/B ${nm.join("/")} nm (from the channel filters)`
    : `Blackbody approximation at R/G/B ${SPCC_DEFAULT_WAVELENGTHS_NM.join("/")} nm (default; filter wavelengths unknown)`;
}

export function spccResultWavelengths(result: {
  wavelengths_nm?: SpccTriplet;
  wavelength_source?: "filters" | "default";
}): string | null {
  if (!result.wavelengths_nm) return null;
  const source = result.wavelength_source ? ` (${result.wavelength_source})` : "";
  return `${result.wavelengths_nm.join("/")} nm${source}`;
}
