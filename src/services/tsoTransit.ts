import { typedInvoke } from "../infrastructure/tauri";
import type { SeriesKey, TsoLightCurvesRequest } from "../shared/types/tso";
import type { TsoTransitFitRequest, TsoTransitSpectrum } from "../shared/types/tsoTransit";

export function tsoTransitSpectrum(
  paths: string[],
  series: SeriesKey,
  request: TsoLightCurvesRequest,
  fit: TsoTransitFitRequest,
): Promise<TsoTransitSpectrum> {
  return typedInvoke<TsoTransitSpectrum>("tso_transit_spectrum_cmd", { paths, series, request, fit });
}
