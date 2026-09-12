import { typedInvoke } from "../infrastructure/tauri";
import type {
  Region,
  RegionShape,
  RegionSystem,
  RegionWire,
  RegionStatsResult,
  RadialProfile,
  LineCut,
  RegionImportResult,
  RegionExportResult,
} from "../shared/types";

export interface RegionStatsRequest {
  id: string;
  shape: RegionShape;
  background?: RegionShape | null;
}

export interface RegionStatsOptions {
  excludeDq?: boolean;
  sigma?: number;
  maxiters?: number;
}

export function regionStats(
  path: string,
  regions: RegionStatsRequest[],
  opts: RegionStatsOptions = {},
): Promise<RegionStatsResult> {
  return typedInvoke<RegionStatsResult>("region_stats_cmd", {
    path,
    regions: regions.map((r) => ({ id: r.id, shape: r.shape, background: r.background ?? null })),
    excludeDq: opts.excludeDq ?? false,
    sigma: opts.sigma ?? null,
    maxiters: opts.maxiters ?? null,
  });
}

export function radialProfile(
  path: string,
  x: number,
  y: number,
  maxRadius: number,
  opts: { background?: [number, number] | null; excludeDq?: boolean } = {},
): Promise<RadialProfile> {
  return typedInvoke<RadialProfile>("radial_profile_cmd", {
    path,
    x,
    y,
    maxRadius,
    background: opts.background ?? null,
    excludeDq: opts.excludeDq ?? false,
  });
}

export function lineCut(
  path: string,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  excludeDq = false,
): Promise<LineCut> {
  return typedInvoke<LineCut>("line_cut_cmd", { path, x1, y1, x2, y2, excludeDq });
}

export function importRegions(path: string, regText: string): Promise<RegionImportResult> {
  return typedInvoke<RegionImportResult>("regions_import_cmd", { path, regText });
}

export function exportRegions(
  path: string,
  regions: RegionWire[],
  system: RegionSystem,
  sexagesimal = true,
): Promise<RegionExportResult> {
  return typedInvoke<RegionExportResult>("regions_export_cmd", { path, regions, system, sexagesimal });
}

export function toWire(r: Region): RegionWire {
  return { shape: r.shape, props: r.props };
}
