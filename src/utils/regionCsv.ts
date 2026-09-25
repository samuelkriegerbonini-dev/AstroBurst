import type { Region, RegionStatsEntry } from "../shared/types/regions";
import { buildCsv, type CsvColumn } from "./catalogCsv";
import { shapeSummary } from "./regionGeometry";

export interface RegionCsvRow {
  region: Region;
  entry: RegionStatsEntry | undefined;
}

const DEFAULT_FILE_NAME = "regions.csv";
const CSV_SUFFIX = "_regions.csv";

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
  { header: "net_snr", value: (r) => r.entry?.stats?.net_snr },
  { header: "flux_source", value: (r) => r.entry?.stats?.calibrated?.flux_source },
  { header: "flux_jy", value: (r) => r.entry?.stats?.calibrated?.flux_jy },
  { header: "flux_err_jy", value: (r) => r.entry?.stats?.calibrated?.flux_err_jy },
  { header: "mag_ab", value: (r) => r.entry?.stats?.calibrated?.mag_ab },
  { header: "mag_ab_err", value: (r) => r.entry?.stats?.calibrated?.mag_ab_err },
  { header: "sb_mag_arcsec2", value: (r) => r.entry?.stats?.calibrated?.sb_mag_arcsec2 },
  { header: "area_arcsec2", value: (r) => r.entry?.stats?.calibrated?.area_arcsec2 },
  { header: "ra", value: (r) => r.entry?.stats?.calibrated?.ra },
  { header: "dec", value: (r) => r.entry?.stats?.calibrated?.dec },
  { header: "pa_sky_deg", value: (r) => r.entry?.stats?.calibrated?.pa_sky_deg },
];

export function regionsTableCsv(regions: Region[], stats: ReadonlyMap<string, RegionStatsEntry>): string {
  return buildCsv(
    REGION_CSV_COLUMNS,
    regions.map((region) => ({ region, entry: stats.get(region.id) })),
  );
}

export function regionsCsvFileName(filePath: string | null): string {
  const withoutFragment = (filePath ?? "").split("#")[0];
  const base = withoutFragment.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  return stem ? `${stem}${CSV_SUFFIX}` : DEFAULT_FILE_NAME;
}
