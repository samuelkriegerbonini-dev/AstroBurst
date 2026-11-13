import type { ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";
import type { ResolutionRequest, ResolutionSettings } from "../../shared/types/resolution";
import { sizeSource } from "../../utils/forcedPhotometryRequest";
import { adjacentPairs, bandOrder, cellFlags } from "../../utils/forcedPhotometry";
import { resolutionRequest, significant, type PsfChoice } from "../../utils/resolutionText";

export interface ForcedTableColumn {
  band: number;
  label: string;
  title: string;
}

export interface ForcedTableColour {
  a: number;
  b: number;
  label: string;
  title: string;
}

export function tableColumns(result: ForcedPhotometryResult): ForcedTableColumn[] {
  return bandOrder(result.bands)
    .filter((index) => result.bands[index].error === null)
    .map((index) => ({ band: index, label: result.bands[index].label, title: result.bands[index].file_name }));
}

export function tableColours(result: ForcedPhotometryResult): ForcedTableColour[] {
  return adjacentPairs(result.bands).map(([a, b]) => {
    const labelA = result.bands[a].label;
    const labelB = result.bands[b].label;
    return { a, b, label: `${labelA}-${labelB}`, title: `${labelA} minus ${labelB}, AB magnitudes` };
  });
}

export function flagsText(result: ForcedPhotometryResult, source: number): string {
  const cells = result.sources[source].cells;
  return tableColumns(result)
    .map((column) => {
      const flags = cellFlags(cells[column.band] ?? null, result.bands[column.band]);
      return flags.length > 0 ? `${column.label}: ${flags.join(", ")}` : "";
    })
    .filter((text) => text !== "")
    .join("; ");
}

export function sizeRequests(
  result: ForcedPhotometryResult,
  band: number,
  settings: ResolutionSettings,
  psf: PsfChoice,
  limit: number,
): (ResolutionRequest | null)[] {
  const sizeBandInfo = result.bands[band];
  return result.sources.slice(0, limit).map((source) => {
    const size = sizeSource(source.cells[band] ?? null, sizeBandInfo);
    return size === null ? null : resolutionRequest(settings, psf, sizeBandInfo.pixel_scale_arcsec, size);
  });
}

export function fwhmMissing(result: ForcedPhotometryResult, source: number, band: number): boolean {
  const cell = result.sources[source].cells[band] ?? null;
  if (cell === null || !cell.on_image || cell.error !== null || result.bands[band].error !== null) return false;
  return !(cell.fwhm_px !== null && Number.isFinite(cell.fwhm_px) && cell.fwhm_px > 0);
}

export function sizePsfText(psf: PsfChoice): string {
  return `${significant(psf.fwhmArcsec)} arcsec (${psf.reference})`;
}
