import { typedInvoke } from "../infrastructure/tauri";
import type { TransitFitOptions, TransitFitRequest, TransitFitResult } from "../shared/types/transit";

export function fitTransit(request: TransitFitRequest, options: TransitFitOptions = {}): Promise<TransitFitResult> {
  return typedInvoke<TransitFitResult>("light_curve_transit_fit_cmd", { request, includeSeries: options.includeSeries ?? true });
}
