import type { TsoExposureCompleteness, TsoLayout, TsoSeriesSummary } from "../shared/types/tso";
import { tsoTimeName } from "./tsoLightCurves";

export interface LoadedSeriesFile {
  path: string;
  name: string;
}

const SEGMENT_MARKER = /-seg[0-9]/i;

export function exposureStem(fileName: string): string | null {
  const marker = SEGMENT_MARKER.exec(fileName);
  return marker ? fileName.slice(0, marker.index) : null;
}

export function siblingSegments(activePath: string, activeName: string, files: readonly LoadedSeriesFile[]): LoadedSeriesFile[] {
  const stem = exposureStem(activeName);
  if (stem === null) return [];
  return files.filter((file) => file.path !== activePath && exposureStem(file.name) === stem);
}

export function setPaths(activePath: string, chosen: readonly string[]): string[] {
  return [...new Set([activePath, ...chosen])];
}

export function levelText(level: number): string {
  return level === 3 ? "level 3" : "level 2b";
}

export function layoutText(layout: TsoLayout): string {
  return layout === "rows" ? "one row per integration" : "one table per integration";
}

export function seriesOptionText(series: TsoSeriesSummary): string {
  return `${series.label} (${series.n_integrations} integrations x ${series.n_channels} channels)`;
}

export function seriesFactsText(series: TsoSeriesSummary): string {
  return `${series.n_integrations} integrations x ${series.n_channels} channels, flux in ${series.flux_unit}, time axis ${tsoTimeName(series.time_scale)} (${series.time_source})`;
}

function rangeText([first, last]: readonly [number, number]): string {
  return first === last ? `${first}` : `${first} to ${last}`;
}

export function completenessText(c: TsoExposureCompleteness): string {
  if (c.nints === null) return `${c.exposure}: ${c.found} integrations (NINTS unknown)`;
  const counted = `${c.exposure}: ${c.found} of ${c.nints} integrations`;
  return c.missing.length === 0 ? counted : `${counted}, missing ${c.missing.map(rangeText).join(", ")}`;
}
