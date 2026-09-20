import type {
  CorrectionFrame,
  RadialVelocityCorrectionResult,
  SpectralAxisInfo,
  SpectralAxisMode,
  VelocityConvention,
} from "../shared/types/spectral";

export const SPEED_OF_LIGHT_KMS = 299792.458;
export const AIR_FORMULA_MIN_UM = 0.2;
export const AIR_VACUUM_FORMULA = "Greisen et al. 2006 (FITS Paper III eq. 65)";

const AIR_VACUUM_TOLERANCE_UM = 1e-10;
const AIR_VACUUM_MAX_ITERATIONS = 50;
const MICRON = "μm";
const EM_DASH = "—";

export const SPECTRAL_MODES: readonly SpectralAxisMode[] = ["wavelength_vac", "wavelength_air", "frequency", "velocity"];
export const VELOCITY_CONVENTIONS: readonly VelocityConvention[] = ["optical", "radio", "relativistic"];
export const CORRECTION_FRAMES: readonly CorrectionFrame[] = ["none", "heliocentric", "barycentric"];

export function airRefractiveIndex(lambdaUm: number): number {
  const l2 = lambdaUm * lambdaUm;
  return 1 + 1e-6 * (287.6155 + 1.62887 / l2 + 0.0136 / (l2 * l2));
}

function airFormulaApplies(lambdaUm: number): boolean {
  return lambdaUm >= AIR_FORMULA_MIN_UM;
}

export function vacuumToAirUm(lambdaVacuumUm: number): number {
  if (!airFormulaApplies(lambdaVacuumUm)) return lambdaVacuumUm;
  return lambdaVacuumUm / airRefractiveIndex(lambdaVacuumUm);
}

export function airToVacuumUm(lambdaAirUm: number): number {
  if (!airFormulaApplies(lambdaAirUm)) return lambdaAirUm;
  let lambdaVacuum = lambdaAirUm * airRefractiveIndex(lambdaAirUm);
  for (let i = 0; i < AIR_VACUUM_MAX_ITERATIONS; i++) {
    const next = lambdaAirUm * airRefractiveIndex(lambdaVacuum);
    const converged = Math.abs(next - lambdaVacuum) < AIR_VACUUM_TOLERANCE_UM;
    lambdaVacuum = next;
    if (converged) break;
  }
  return lambdaVacuum;
}

export function velocityKms(observedUm: number, restUm: number, convention: VelocityConvention): number {
  switch (convention) {
    case "optical":
      return (SPEED_OF_LIGHT_KMS * (observedUm - restUm)) / restUm;
    case "radio":
      return (SPEED_OF_LIGHT_KMS * (observedUm - restUm)) / observedUm;
    case "relativistic": {
      const o2 = observedUm * observedUm;
      const r2 = restUm * restUm;
      return (SPEED_OF_LIGHT_KMS * (o2 - r2)) / (o2 + r2);
    }
  }
}

export function wavelengthFromVelocityUm(velocityKmsValue: number, restUm: number, convention: VelocityConvention): number {
  const beta = velocityKmsValue / SPEED_OF_LIGHT_KMS;
  switch (convention) {
    case "optical":
      return restUm * (1 + beta);
    case "radio":
      return restUm / (1 - beta);
    case "relativistic":
      return restUm * Math.sqrt((1 + beta) / (1 - beta));
  }
}

export function frequencyGhzFromWavelengthUm(wavelengthUm: number): number {
  return SPEED_OF_LIGHT_KMS / wavelengthUm;
}

export function wavelengthUmFromFrequencyGhz(frequencyGhz: number): number {
  return SPEED_OF_LIGHT_KMS / frequencyGhz;
}

export function isVelocityKind(axis: SpectralAxisInfo | null): boolean {
  return axis !== null && (axis.kind === "vrad" || axis.kind === "vopt" || axis.kind === "velo");
}

export function availableModes(axis: SpectralAxisInfo | null): SpectralAxisMode[] {
  if (!axis) return [];
  if (isVelocityKind(axis) || axis.kind === "zopt") return ["velocity"];
  if (axis.kind === "wave" || axis.kind === "awav" || axis.kind === "freq") return [...SPECTRAL_MODES];
  return [];
}

export function modeLabel(mode: SpectralAxisMode): string {
  switch (mode) {
    case "wavelength_vac":
      return "Vacuum wavelength";
    case "wavelength_air":
      return "Air wavelength";
    case "frequency":
      return "Frequency";
    case "velocity":
      return "Velocity";
  }
}

export function modeUnit(mode: SpectralAxisMode): string {
  switch (mode) {
    case "wavelength_vac":
    case "wavelength_air":
      return MICRON;
    case "frequency":
      return "GHz";
    case "velocity":
      return "km/s";
  }
}

export function conventionLabel(convention: VelocityConvention): string {
  return convention.charAt(0).toUpperCase() + convention.slice(1);
}

export function defaultRestUm(axis: SpectralAxisInfo | null): number | null {
  const rest = axis?.rest_wavelength_um ?? null;
  return rest !== null && Number.isFinite(rest) && rest > 0 ? rest : null;
}

export function parseRestInput(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;
  const value = Number(trimmed);
  return Number.isFinite(value) && value > 0 ? value : null;
}

interface VacuumAxis {
  valuesUm: number[];
  notes: string[];
}

function vacuumWavelengthsUm(axis: SpectralAxisInfo): VacuumAxis | null {
  switch (axis.kind) {
    case "wave":
      return { valuesUm: axis.values, notes: [] };
    case "awav": {
      const notes = [`air wavelengths converted to vacuum with ${AIR_VACUUM_FORMULA}`];
      if (axis.values.some((w) => Number.isFinite(w) && !airFormulaApplies(w))) {
        notes.unshift(
          `some air wavelengths are below ${AIR_FORMULA_MIN_UM} um where ${AIR_VACUUM_FORMULA} does not apply: left unchanged`,
        );
      }
      return { valuesUm: axis.values.map(airToVacuumUm), notes };
    }
    case "freq":
      return { valuesUm: axis.values.map(wavelengthUmFromFrequencyGhz), notes: [] };
    default:
      return null;
  }
}

export interface FormattedAxis {
  values: number[];
  label: string;
  unit: string;
  notes: string[];
}

export function formatAxis(
  axis: SpectralAxisInfo | null,
  mode: SpectralAxisMode,
  restUm: number | null,
  convention: VelocityConvention,
): FormattedAxis | null {
  if (!axis) return null;
  if (isVelocityKind(axis)) {
    return {
      values: axis.values,
      label: "Velocity (km/s)",
      unit: "km/s",
      notes: [`axis ${axis.ctype} is already a velocity axis: values shown as stored`],
    };
  }
  if (axis.kind === "zopt") {
    return {
      values: axis.values.map((z) => velocityKms(1 + z, 1, convention)),
      label: `Velocity, ${convention} (km/s)`,
      unit: "km/s",
      notes: [`redshift axis ${axis.ctype} converted to ${convention} velocity from 1 + z (rest wavelength not used)`],
    };
  }
  const vacuum = vacuumWavelengthsUm(axis);
  if (!vacuum) return null;
  const unit = modeUnit(mode);
  switch (mode) {
    case "wavelength_vac":
      return { values: vacuum.valuesUm, label: `${modeLabel(mode)} (${unit})`, unit, notes: vacuum.notes };
    case "wavelength_air": {
      const values = axis.kind === "awav" ? axis.values : vacuum.valuesUm.map(vacuumToAirUm);
      const notes = axis.kind === "awav" ? [] : [`vacuum wavelengths converted to air with ${AIR_VACUUM_FORMULA}`];
      return { values, label: `${modeLabel(mode)} (${unit})`, unit, notes };
    }
    case "frequency": {
      const values = axis.kind === "freq" ? axis.values : vacuum.valuesUm.map(frequencyGhzFromWavelengthUm);
      return { values, label: `${modeLabel(mode)} (${unit})`, unit, notes: vacuum.notes };
    }
    case "velocity": {
      if (restUm === null || !(restUm > 0)) return null;
      const values = vacuum.valuesUm.map((w) => velocityKms(w, restUm, convention));
      const notes = [...vacuum.notes, `rest wavelength ${restUm} um taken as a vacuum wavelength`];
      return { values, label: `${modeLabel(mode)}, ${convention} (${unit})`, unit, notes };
    }
  }
}

export function applyCorrectionKms(
  valuesKms: number[],
  correction: RadialVelocityCorrectionResult | null,
  frame: CorrectionFrame,
): number[] {
  if (!correction || frame === "none") return valuesKms;
  const shift = frame === "barycentric" ? correction.barycentric_kms : correction.heliocentric_kms;
  return valuesKms.map((v) => v + shift);
}

export function formatAxisValue(value: number, unit: string): string {
  if (!Number.isFinite(value)) return EM_DASH;
  const decimals = unit === "km/s" ? 1 : 4;
  return value.toFixed(decimals);
}
