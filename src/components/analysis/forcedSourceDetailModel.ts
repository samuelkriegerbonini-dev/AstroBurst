import { forcedSedPoints, isLimit } from "../../utils/forcedPhotometry";
import { sizeSource } from "../../utils/forcedPhotometryRequest";
import type { ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";

export function sourceTitle(result: ForcedPhotometryResult, source: number): string {
  const { id, ra_deg, dec_deg } = result.sources[source];
  return `${id} at RA ${ra_deg.toFixed(6)} Dec ${dec_deg.toFixed(6)} deg`;
}

export function sedNotes(result: ForcedPhotometryResult, source: number): string[] {
  const { points, basis } = forcedSedPoints(result, source);
  const notes: string[] = [];
  if (basis === "total") notes.push("Each band is corrected to total flux with its own aperture correction.");
  else if (points.length >= 2)
    notes.push(
      "Aperture fluxes: bands with different PSFs enclose different fractions of the source. Turn on the correction to total flux to compare them.",
    );
  const plotted = new Set(points.map((point) => point.index));
  const cells = result.sources[source].cells;
  const unplotted = result.bands
    .filter((band, index) => {
      const cell = cells[index] ?? null;
      return band.error === null && !plotted.has(index) && cell !== null && (Number.isFinite(cell.flux_jy) || isLimit(cell));
    })
    .map((band) => band.label);
  if (unplotted.length > 0) notes.push(`Not plotted, no pivot wavelength: ${unplotted.join(", ")}.`);
  if (points.some((point) => point.limit)) notes.push(`Arrows are ${String(result.limit_sigma)} sigma upper limits.`);
  return notes;
}

export function sourceSizeBand(result: ForcedPhotometryResult, source: number, preferredPath: string | null): number | null {
  const cells = result.sources[source].cells;
  const hasSize = (index: number) => sizeSource(cells[index] ?? null, result.bands[index]) !== null;
  const preferred = result.bands.findIndex((band, index) => band.path === preferredPath && hasSize(index));
  if (preferred >= 0) return preferred;
  const first = result.bands.findIndex((_, index) => hasSize(index));
  return first >= 0 ? first : null;
}
