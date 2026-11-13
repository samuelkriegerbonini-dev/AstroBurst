import type { ApertureMarker } from "../components/viewer/painters/aperturePainter";
import type { ForcedPhotometryResult } from "../shared/types/forcedPhotometry";
import { apertureMarks } from "./forcedPhotometryRequest";

export const FORCED_LAYER_ID = "forced-apertures";

export interface ForcedApertureLayer {
  apertures: ApertureMarker[];
  highlightIndex: number | null;
}

export function forcedApertureLayer(result: ForcedPhotometryResult, band: number, selected: number | null): ForcedApertureLayer | null {
  const measured = result.bands[band];
  if (measured === undefined || measured.error !== null) return null;
  const rAp = measured.aperture_radius_px;
  if (rAp === null || !Number.isFinite(rAp) || rAp <= 0) return null;
  const [skyIn, skyOut] = measured.annulus_px ?? [Number.NaN, Number.NaN];
  const marks = apertureMarks(result, band);
  const apertures = marks.map(
    (mark): ApertureMarker => ({ x: mark.x, y: mark.y, rAp, skyIn, skyOut, label: mark.source === selected ? mark.id : "" }),
  );
  const highlight = selected === null ? -1 : marks.findIndex((mark) => mark.source === selected);
  return { apertures, highlightIndex: highlight >= 0 ? highlight : null };
}
