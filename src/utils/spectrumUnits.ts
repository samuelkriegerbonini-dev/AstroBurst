import type { SpectralAxisMode } from "../shared/types/spectral";
import type { FluxDisplayChoice, WavelengthDisplayUnit } from "../shared/types/spectrum1d";
import { FLUX_DISPLAY_UNIT_LABEL, availableFluxDisplayUnits, convertFlux, parseSpectralFluxUnit } from "./fluxUnits";
import type { SpectralFluxUnit } from "./fluxUnits";

export const WAVELENGTH_DISPLAY_UNITS: readonly WavelengthDisplayUnit[] = ["um", "nm", "A"];
export const WAVELENGTH_UNIT_FACTOR: Readonly<Record<WavelengthDisplayUnit, number>> = { um: 1, nm: 1e3, A: 1e4 };
export const WAVELENGTH_UNIT_LABEL: Readonly<Record<WavelengthDisplayUnit, string>> = { um: "μm", nm: "nm", A: "Å" };
export const WAVELENGTH_UNIT_ASCII: Readonly<Record<WavelengthDisplayUnit, string>> = {
  um: "um",
  nm: "nm",
  A: "Angstrom",
};

const NATIVE_FLUX_LABEL = "native";
const JY_PER_FNU_CHOICE: Readonly<Partial<Record<FluxDisplayChoice, number>>> = { Jy: 1, mJy: 1e3, uJy: 1e6 };
const CGS_PER_FLAMBDA_CHOICE: Readonly<Partial<Record<FluxDisplayChoice, number>>> = { flam_cgs: 1, flam_si: 10 };
const FNU_TO_FLAMBDA_BASIS =
  "F_lambda = F_nu x c / lambda^2 with the vacuum wavelength of each sample (c = 299792458 m/s)";
const FLAMBDA_TO_FNU_BASIS =
  "F_nu = F_lambda x lambda^2 / c with the vacuum wavelength of each sample (c = 299792458 m/s)";
const AB_BASIS = "AB = -2.5 log10(F_nu / Jy) + 8.90; error = 1.0857 x err / flux";

export function scaleWavelengths(valuesUm: readonly number[], unit: WavelengthDisplayUnit): number[] {
  const factor = WAVELENGTH_UNIT_FACTOR[unit];
  return valuesUm.map((value) => value * factor);
}

export function wavelengthUnitApplies(mode: SpectralAxisMode, formattedUnit: string): boolean {
  return (mode === "wavelength_vac" || mode === "wavelength_air") && formattedUnit === WAVELENGTH_UNIT_LABEL.um;
}

export interface FluxDisplayInput {
  flux: readonly number[];
  err: readonly number[] | null;
  vacuumUm: readonly number[] | null;
  fluxUnit: string | null;
}

export interface FluxDisplay {
  choice: FluxDisplayChoice;
  values: number[];
  errors: number[] | null;
  unit: string;
  scale: number | null;
  invertY: boolean;
  basis: string | null;
  reason: string | null;
}

export function nativeFluxLabel(fluxUnit: string | null): string {
  const trimmed = (fluxUnit ?? "").trim().replace(/^'+|'+$/g, "").trim();
  return trimmed === "" ? NATIVE_FLUX_LABEL : trimmed;
}

function needsWavelength(from: SpectralFluxUnit, choice: FluxDisplayChoice): boolean {
  if (from.kind === "flux_density_nu") return CGS_PER_FLAMBDA_CHOICE[choice] !== undefined;
  if (from.kind === "flux_density_lambda") return JY_PER_FNU_CHOICE[choice] !== undefined || choice === "ab";
  return false;
}

export function fluxDisplayChoices(fluxUnit: string | null, hasVacuumAxis: boolean): FluxDisplayChoice[] {
  const from = parseSpectralFluxUnit(fluxUnit ?? "");
  const available: FluxDisplayChoice[] = availableFluxDisplayUnits(from);
  return hasVacuumAxis ? available : available.filter((choice) => !needsWavelength(from, choice));
}

export function fluxChoiceLabel(choice: FluxDisplayChoice, fluxUnit: string | null): string {
  return choice === "native" ? `file unit (${nativeFluxLabel(fluxUnit)})` : FLUX_DISPLAY_UNIT_LABEL[choice];
}

function nativeDisplay(input: FluxDisplayInput, reason: string | null): FluxDisplay {
  return {
    choice: "native",
    values: [...input.flux],
    errors: input.err === null ? null : [...input.err],
    unit: nativeFluxLabel(input.fluxUnit),
    scale: 1,
    invertY: false,
    basis: null,
    reason,
  };
}

function singleFactor(from: SpectralFluxUnit, choice: FluxDisplayChoice): number | null {
  if (from.kind === "flux_density_nu") {
    const jyPerChoice = JY_PER_FNU_CHOICE[choice];
    return jyPerChoice === undefined ? null : from.jy_per_unit * jyPerChoice;
  }
  if (from.kind === "flux_density_lambda") {
    const cgsPerChoice = CGS_PER_FLAMBDA_CHOICE[choice];
    return cgsPerChoice === undefined ? null : from.cgs_per_unit * cgsPerChoice;
  }
  return null;
}

function conversionBasis(from: SpectralFluxUnit, choice: FluxDisplayChoice): string {
  if (from.kind === "flux_density_nu") return choice === "ab" ? AB_BASIS : FNU_TO_FLAMBDA_BASIS;
  return choice === "ab" ? `${FLAMBDA_TO_FNU_BASIS}; ${AB_BASIS}` : FLAMBDA_TO_FNU_BASIS;
}

function nullAsNaN(values: readonly (number | null)[]): number[] {
  return values.map((value) => value ?? NaN);
}

export function fluxDisplay(input: FluxDisplayInput, choice: FluxDisplayChoice): FluxDisplay {
  if (choice === "native") return nativeDisplay(input, null);
  const label = FLUX_DISPLAY_UNIT_LABEL[choice];
  const nativeLabel = nativeFluxLabel(input.fluxUnit);
  if (!fluxDisplayChoices(input.fluxUnit, input.vacuumUm !== null).includes(choice)) {
    const missingAxis = fluxDisplayChoices(input.fluxUnit, true).includes(choice) ? " without a wavelength axis" : "";
    return nativeDisplay(input, `${label} is not available for flux in ${nativeLabel}${missingAxis}`);
  }
  const from = parseSpectralFluxUnit(input.fluxUnit ?? "");
  const converted = convertFlux(input.flux, input.err, input.vacuumUm ?? [], from, choice);
  if (!converted.ok) return nativeDisplay(input, converted.reason);
  const scale = singleFactor(from, choice);
  return {
    choice,
    values: nullAsNaN(converted.values),
    errors: converted.errors === null ? null : nullAsNaN(converted.errors),
    unit: converted.unit ?? label,
    scale,
    invertY: choice === "ab",
    basis: scale === null ? conversionBasis(from, choice) : `${nativeLabel} x ${String(scale)}`,
    reason: null,
  };
}

export function fluxAxisTitle(display: Pick<FluxDisplay, "unit" | "invertY">): string {
  return display.invertY ? "AB magnitude" : `Flux (${display.unit})`;
}
