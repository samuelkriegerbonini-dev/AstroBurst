import { typedInvoke } from "../infrastructure/tauri";
import { cancelProgress } from "./progress";
import { FORCED_PHOTOMETRY_PROGRESS_EVENT } from "../shared/types/forcedPhotometry";
import type { ForcedPhotometryRequest, ForcedPhotometryResult } from "../shared/types/forcedPhotometry";

export function forcedPhotometry(request: ForcedPhotometryRequest): Promise<ForcedPhotometryResult> {
  const { bands, positions, aperture } = request;
  return typedInvoke<ForcedPhotometryResult>("forced_photometry_cmd", {
    bands,
    positions,
    aperture,
    options: request.options ?? null,
  });
}

export function cancelForcedPhotometry(): Promise<boolean> {
  return cancelProgress(FORCED_PHOTOMETRY_PROGRESS_EVENT);
}
