import type { Region, RegionShape, RegionStats } from "../shared/types/regions";

export const EXCLUDE_TITLE = "Exclude: pixels inside are left out of every other region's statistics and profiles (DS9 '-' prefix)";
export const EXCLUDED_CELL_TITLE = "pixels removed by DQ flags and/or exclude regions";

export function isExcludeRegion(region: Region): boolean {
  return !region.props.include && region.shape.shape !== "line" && region.shape.shape !== "point";
}

export function excludeShapes(regions: readonly Region[]): RegionShape[] {
  return regions.filter(isExcludeRegion).map((r) => r.shape);
}

export function exclusionsFor(region: Region | undefined, regions: readonly Region[]): RegionShape[] {
  return region && isExcludeRegion(region) ? [] : excludeShapes(regions);
}

export function backgroundCandidates(regions: readonly Region[], region: Region): Region[] {
  return regions.filter(
    (a) => a.shape.shape === "annulus" && a.id !== region.id && (!isExcludeRegion(a) || a.id === region.backgroundId),
  );
}

export function excludedCellLabel(stats: Pick<RegionStats, "n_excluded">): string | null {
  return stats.n_excluded > 0 ? `(−${stats.n_excluded} excl)` : null;
}

export function toggledInclude(region: Region): Partial<Region> {
  return { props: { ...region.props, include: !region.props.include } };
}
