import type { Region, RegionSky, RegionStats, RegionStatsEntry } from "../shared/types/regions";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { exportEntries, provenanceLines, summedPixelUnit, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { shapeSummary } from "./regionGeometry";

export interface RegionCsvRow {
  region: Region;
  entry: RegionStatsEntry | undefined;
  background?: Region | null;
}

export interface RegionCsvContext {
  exportedUtc: string;
  fileName: string | null;
  image: string | null;
  bunit: string | null;
  summedUnit: string | null;
  calibration: string | null;
  excludeDq: boolean;
  dqExcludedPx: number | null;
  regionExcludedPx: number;
  clipSigma: number;
  clipMaxiters: number;
  notes: readonly string[];
}

export const REGION_SIGMA_NOTE = "1.4826 x MAD of the region pixels (robust); std is the sample standard deviation";
export const REGION_NET_NOTE = "sum - background median x n; the background region of each row is in the column background";
export const REGION_NET_ERR_NOTE =
  "sqrt(sum_err^2 + n^2 bg_sigma^2 / bg_n) with an error layer, sqrt(n bg_sigma^2 + n^2 bg_sigma^2 / bg_n) without one (column net_err_terms)";
export const REGION_MEMBERSHIP_NOTE = "a pixel belongs to a region when its centre is inside; no partial pixels";
export const REGION_POSITIONS_NOTE = "0-based pixels, integer pixel centres";

const DEFAULT_FILE_NAME = "regions.csv";
const CSV_SUFFIX = "_regions.csv";

export function regionSkyOf(stats: RegionStats | null | undefined): RegionSky | null {
  if (!stats) return null;
  return stats.sky !== undefined ? stats.sky : (stats.calibrated ?? null);
}

export const REGION_CSV_COLUMNS: CsvColumn<RegionCsvRow>[] = [
  { header: "id", value: (r) => r.region.id },
  { header: "shape", value: (r) => shapeSummary(r.region.shape) },
  { header: "text", value: (r) => r.region.props.text },
  { header: "include", value: (r) => r.region.props.include },
  { header: "n", value: (r) => r.entry?.stats?.count },
  { header: "n_excluded", value: (r) => r.entry?.stats?.n_excluded },
  { header: "area_px", value: (r) => r.entry?.stats?.area },
  { header: "mean", value: (r) => r.entry?.stats?.mean },
  { header: "median", value: (r) => r.entry?.stats?.median },
  { header: "sigma", value: (r) => r.entry?.stats?.sigma },
  { header: "std", value: (r) => r.entry?.stats?.std },
  { header: "min", value: (r) => r.entry?.stats?.min },
  { header: "max", value: (r) => r.entry?.stats?.max },
  { header: "sum", value: (r) => r.entry?.stats?.sum },
  { header: "sum_err", value: (r) => r.entry?.stats?.sum_err },
  { header: "net_sum", value: (r) => r.entry?.stats?.net_sum },
  { header: "net_err", value: (r) => r.entry?.stats?.net_err },
  { header: "net_snr", value: (r) => r.entry?.stats?.net_snr },
  { header: "net_err_terms", value: (r) => r.entry?.stats?.net_err_terms },
  { header: "flux_source", value: (r) => r.entry?.stats?.calibrated?.flux_source },
  { header: "flux_jy", value: (r) => r.entry?.stats?.calibrated?.flux_jy },
  { header: "flux_err_jy", value: (r) => r.entry?.stats?.calibrated?.flux_err_jy },
  { header: "mag_ab", value: (r) => r.entry?.stats?.calibrated?.mag_ab },
  { header: "mag_ab_err", value: (r) => r.entry?.stats?.calibrated?.mag_ab_err },
  { header: "sb_mag_arcsec2", value: (r) => r.entry?.stats?.calibrated?.sb_mag_arcsec2 },
  { header: "area_arcsec2", value: (r) => regionSkyOf(r.entry?.stats)?.area_arcsec2 },
  { header: "ra", value: (r) => regionSkyOf(r.entry?.stats)?.ra },
  { header: "dec", value: (r) => regionSkyOf(r.entry?.stats)?.dec },
  { header: "pa_sky_deg", value: (r) => regionSkyOf(r.entry?.stats)?.pa_sky_deg },
  { header: "n_nan", value: (r) => r.entry?.stats?.n_nan },
  { header: "n_padding", value: (r) => r.entry?.stats?.n_padding },
  { header: "mad", value: (r) => r.entry?.stats?.mad },
  { header: "clipped_mean", value: (r) => r.entry?.stats?.clipped_mean },
  { header: "clipped_median", value: (r) => r.entry?.stats?.clipped_median },
  { header: "clipped_sigma", value: (r) => r.entry?.stats?.clipped_sigma },
  { header: "n_rejected", value: (r) => r.entry?.stats?.n_rejected },
  { header: "background", value: (r) => (r.background ? shapeSummary(r.background.shape) : null) },
  { header: "bg_median", value: (r) => r.entry?.stats?.background?.median },
  { header: "bg_sigma", value: (r) => r.entry?.stats?.background?.sigma },
  { header: "bg_n", value: (r) => r.entry?.stats?.background?.count },
];

function dqExclusionText(context: RegionCsvContext): string {
  if (!context.excludeDq) return "off";
  return context.dqExcludedPx !== null ? `applied, ${context.dqExcludedPx} pixels excluded` : "requested, not applied";
}

export function regionCsvProvenance(context: RegionCsvContext, nRegions: number): ProvenanceEntry[] {
  return [
    ...exportEntries(context.fileName, context.exportedUtc),
    ["image", context.image],
    ["regions", nRegions],
    ["bunit", context.bunit],
    ["calibration", context.calibration ?? "none"],
    ["dq_exclusion", dqExclusionText(context)],
    ["exclude_regions_px", context.regionExcludedPx],
    [
      "sigma_clip",
      `${context.clipSigma} sigma, at most ${context.clipMaxiters} iterations (columns clipped_mean, clipped_median, clipped_sigma, n_rejected)`,
    ],
    ["sigma", REGION_SIGMA_NOTE],
    ["net_sum", REGION_NET_NOTE],
    ["net_err", REGION_NET_ERR_NOTE],
    ["membership", REGION_MEMBERSHIP_NOTE],
    ["positions", REGION_POSITIONS_NOTE],
    ...[...new Set(context.notes)].map((note): ProvenanceEntry => ["note", note]),
  ];
}

export function regionCsvUnits(context: RegionCsvContext): Record<string, string> {
  const pixel = context.bunit ?? "image unit";
  const summed = context.summedUnit ?? summedPixelUnit(context.bunit) ?? "image unit, summed over pixels";
  return {
    area_px: "pixel",
    mean: pixel,
    median: pixel,
    sigma: pixel,
    std: pixel,
    min: pixel,
    max: pixel,
    sum: summed,
    sum_err: summed,
    net_sum: summed,
    net_err: summed,
    flux_jy: "Jy",
    flux_err_jy: "Jy",
    mag_ab: "mag (AB)",
    mag_ab_err: "mag",
    sb_mag_arcsec2: "mag/arcsec^2 (AB)",
    area_arcsec2: "arcsec^2",
    ra: "deg (ICRS)",
    dec: "deg (ICRS)",
    pa_sky_deg: "deg (east of north)",
    mad: pixel,
    clipped_mean: pixel,
    clipped_median: pixel,
    clipped_sigma: pixel,
    bg_median: pixel,
    bg_sigma: pixel,
    bg_n: "pixel",
  };
}

export function regionsTableCsv(regions: Region[], stats: ReadonlyMap<string, RegionStatsEntry>, context?: RegionCsvContext): string {
  const byId = new Map(regions.map((region) => [region.id, region]));
  const body = buildCsv(
    REGION_CSV_COLUMNS,
    regions.map((region) => ({
      region,
      entry: stats.get(region.id),
      background: region.backgroundId ? (byId.get(region.backgroundId) ?? null) : null,
    })),
  );
  if (!context) return body;
  return withProvenance(
    [...provenanceLines(regionCsvProvenance(context, regions.length)), ...unitLines(regionCsvUnits(context))],
    body,
    CSV_LINE_END,
  );
}

export function regionsCsvFileName(filePath: string | null): string {
  const withoutFragment = (filePath ?? "").split("#")[0];
  const base = withoutFragment.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  return stem ? `${stem}${CSV_SUFFIX}` : DEFAULT_FILE_NAME;
}
