import { typedInvoke } from "../infrastructure/tauri";
import type { ContinuumWindows } from "../shared/types/cube";
import type {
  LineMeasurement,
  LineModel,
  RadialVelocityCorrectionError,
  RadialVelocityCorrectionResponse,
  SpectralAxisInfo,
  SpectrumSource,
  VelocityConvention,
} from "../shared/types/spectral";

export function getSpectralAxis(path: string): Promise<SpectralAxisInfo> {
  return typedInvoke<SpectralAxisInfo>("spectral_axis_cmd", { path });
}

export function getRadialVelocityCorrection(
  path: string,
  ra?: number | null,
  dec?: number | null,
): Promise<RadialVelocityCorrectionResponse> {
  return typedInvoke<RadialVelocityCorrectionResponse>("radial_velocity_correction_cmd", {
    path,
    ra: ra ?? null,
    dec: dec ?? null,
  });
}

export function isCorrectionError(
  response: RadialVelocityCorrectionResponse | null | undefined,
): response is RadialVelocityCorrectionError {
  return !!response && typeof (response as RadialVelocityCorrectionError).error === "string";
}

export interface MeasureLineOptions {
  z0: number;
  z1: number;
  continuum?: ContinuumWindows | null;
  restUm?: number | null;
  convention?: VelocityConvention;
  model?: LineModel;
  velocityShiftKms?: number | null;
}

export function measureSpectralLine(
  path: string,
  source: SpectrumSource,
  opts: MeasureLineOptions,
): Promise<LineMeasurement> {
  return typedInvoke<LineMeasurement>("measure_spectral_line_cmd", {
    path,
    source,
    z0: opts.z0,
    z1: opts.z1,
    continuum: opts.continuum ?? null,
    restUm: opts.restUm ?? null,
    convention: opts.convention ?? "optical",
    model: opts.model ?? "none",
    velocityShiftKms: opts.velocityShiftKms ?? null,
  });
}
