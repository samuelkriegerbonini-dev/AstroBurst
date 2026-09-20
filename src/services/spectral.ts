import { typedInvoke } from "../infrastructure/tauri";
import type {
  RadialVelocityCorrectionError,
  RadialVelocityCorrectionResponse,
  SpectralAxisInfo,
  VelocityAxisResult,
  VelocityConvention,
} from "../shared/types/spectral";

export function getSpectralAxis(path: string): Promise<SpectralAxisInfo> {
  return typedInvoke<SpectralAxisInfo>("spectral_axis_cmd", { path });
}

export function getVelocityAxis(
  path: string,
  restUm: number,
  convention: VelocityConvention,
): Promise<VelocityAxisResult> {
  return typedInvoke<VelocityAxisResult>("velocity_axis_cmd", { path, restUm, convention });
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
