import type {
  ConeSearchResult,
  CrossMatchEntry,
  CrossMatchResult,
  MeasuredSource,
  PlacedCatalogRow,
  ZeroPointAperture,
} from "../shared/types/catalog";
import { provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "./csvProvenance";
import { CSV_LINE_END, catalogRowsCsv, matchesCsv, sourcesCsv } from "./catalogCsv";

export const GAIA_CATALOGUE_NAME = "Gaia DR3";
export const GAIA_MAGNITUDE_SYSTEM = "Vega";

export interface CatalogueCsvContext {
  appVersion: string;
  exportedUtc: string;
  fileName: string | null;
}

const APERTURE_PREFIX = "aperture ";
const VEGA_MAG_UNIT = `mag (${GAIA_MAGNITUDE_SYSTEM})`;

const CATALOGUE_SOURCE_TEXT = {
  network: ", from VizieR",
  memory: ", from the memory cache",
  disk: ", from the disk cache",
};

const CONE_POSITIONS_TEXT =
  "ra, dec: Gaia DR3 at J2016.0; ra_epoch, dec_epoch: propagated to positions_epoch with the proper motions; x, y: 0-based pixel of ra_epoch, dec_epoch";

const MATCH_POSITIONS_TEXT =
  "cat_ra, cat_dec: Gaia position propagated to positions_epoch; d_ra_arcsec = (ra - cat_ra) x cos(mean dec) x 3600 and d_dec_arcsec = (dec - cat_dec) x 3600, star minus Gaia";

const CONE_UNITS = {
  ra: "deg",
  dec: "deg",
  ra_epoch: "deg",
  dec_epoch: "deg",
  pm_ra_masyr: "mas/yr",
  pm_dec_masyr: "mas/yr",
  parallax_mas: "mas",
  g: VEGA_MAG_UNIT,
  bp: VEGA_MAG_UNIT,
  rp: VEGA_MAG_UNIT,
  bp_rp: VEGA_MAG_UNIT,
  x: "px",
  y: "px",
};

const SOURCE_UNITS = {
  x: "px",
  y: "px",
  ra: "deg",
  dec: "deg",
  flux: "image unit (net flux in the aperture)",
  mag_inst: "mag",
  mag_ab: "mag (AB)",
  fwhm: "px",
  aperture_radius: "px",
};

const MATCH_UNITS = {
  ...SOURCE_UNITS,
  cat_ra: "deg",
  cat_dec: "deg",
  g: VEGA_MAG_UNIT,
  bp: VEGA_MAG_UNIT,
  rp: VEGA_MAG_UNIT,
  bp_rp: VEGA_MAG_UNIT,
  sep_arcsec: "arcsec",
  d_ra_arcsec: "arcsec",
  d_dec_arcsec: "arcsec",
};

function radiusText(value: number | null): string {
  return value === null || !Number.isFinite(value) ? "--" : value.toFixed(2);
}

function isNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

export function gaiaEpochText(epochYear: number | null): string {
  return epochYear !== null && Number.isFinite(epochYear)
    ? `J${epochYear.toFixed(2)}`
    : "J2016.0 (no observation date in the header)";
}

export function zeroPointApertureText(aperture: ZeroPointAperture): string {
  if (aperture.kind === "fixed") return `${APERTURE_PREFIX}r = ${radiusText(aperture.radius_px)} px`;
  return `${APERTURE_PREFIX}${String(aperture.fwhm_factor)} x FWHM per star, median r = ${radiusText(aperture.median_radius_px)} px (${radiusText(aperture.min_radius_px)} to ${radiusText(aperture.max_radius_px)})`;
}

export function gaiaListLine(cross: CrossMatchResult): string | null {
  const q = cross.catalog_query;
  if (q === undefined) return null;
  const field = q.cone_smaller_than_field ? ` (the field reaches ${q.field_radius_arcmin.toFixed(1)}')` : "";
  const limit = q.mag_limit === null ? "no G limit" : `G < ${String(q.mag_limit)}`;
  const reached = q.row_cap_reached ? " (reached)" : "";
  const source = cross.catalog_source === undefined ? "" : CATALOGUE_SOURCE_TEXT[cross.catalog_source];
  return `Gaia list: ${cross.n_catalog} rows within ${q.radius_arcmin.toFixed(1)}' of the centre${field}, ${limit}, row cap ${q.max_rows}${reached}${source}`;
}

export function detectionLine(cross: CrossMatchResult): string | null {
  if (!isNumber(cross.detection_sigma) || !isNumber(cross.max_stars)) return null;
  return `detection at ${String(cross.detection_sigma)} sigma: ${cross.n_detected} detections, at most ${cross.max_stars} kept, ${cross.sources.length} measured`;
}

function catalogueText(catalogTable: string | undefined): string {
  return catalogTable === undefined ? GAIA_CATALOGUE_NAME : `${GAIA_CATALOGUE_NAME} (VizieR ${catalogTable})`;
}

function contextEntries(context: CatalogueCsvContext): ProvenanceEntry[] {
  return [
    ["software", `AstroBurst ${context.appVersion}`],
    ["exported_utc", context.exportedUtc],
    ["file", context.fileName === null ? null : context.fileName.replace(/[\r\n]/g, " ")],
  ];
}

export function coneCsvHeaderLines(cone: ConeSearchResult, context: CatalogueCsvContext): string[] {
  const entries: ProvenanceEntry[] = [
    ...contextEntries(context),
    ["catalogue", catalogueText(cone.catalog_table)],
    ["catalogue_source", cone.catalog_source ?? null],
    ["cone_centre_ra_deg", cone.center_ra],
    ["cone_centre_dec_deg", cone.center_dec],
    ["cone_radius_arcmin", cone.radius_arcmin],
    ["g_limit_mag", cone.mag_limit ?? null],
    ["max_rows", cone.max_rows ?? null],
    ["n_rows", cone.n_total],
    ["n_on_image", cone.n_on_image],
    ["positions_epoch", gaiaEpochText(cone.epoch_year)],
    ["positions", CONE_POSITIONS_TEXT],
    ["magnitude_system", GAIA_MAGNITUDE_SYSTEM],
  ];
  return [...provenanceLines(entries), ...unitLines(CONE_UNITS)];
}

function zeroPointModelText(cross: CrossMatchResult): string {
  const zp = cross.zero_point;
  if (zp === null) return "none";
  const relation = `Gaia ${cross.band} - mag_inst = zero_point_mag`;
  return typeof zp.colour_coeff === "number" ? `${relation} + colour_coeff x (BP-RP)` : relation;
}

export function crossMatchCsvHeaderLines(
  kind: "sources" | "matches",
  cross: CrossMatchResult,
  context: CatalogueCsvContext,
): string[] {
  const zp = cross.zero_point;
  const q = cross.catalog_query;
  const astrometry = cross.astrometry;
  const entries: ProvenanceEntry[] = [
    ...contextEntries(context),
    ["catalogue", catalogueText(cross.catalog_table)],
    ["catalogue_source", cross.catalog_source ?? null],
    ["cone_radius_arcmin", q?.radius_arcmin ?? null],
    ["g_limit_mag", q?.mag_limit ?? null],
    ["max_rows", q?.max_rows ?? null],
    ["n_catalogue_rows", cross.n_catalog],
    ["positions_epoch", gaiaEpochText(cross.epoch_year)],
    ["band", `Gaia ${cross.band}`],
    ["magnitude_system", GAIA_MAGNITUDE_SYSTEM],
    ["match_radius_arcsec", cross.match_radius_arcsec],
    ["detection_sigma", cross.detection_sigma ?? null],
    ["max_stars", cross.max_stars ?? null],
    ["n_detected", cross.n_detected],
    ["n_measured", cross.sources.length],
    ["n_matches", cross.matches.length],
    ["aperture", cross.aperture === undefined ? null : zeroPointApertureText(cross.aperture).slice(APERTURE_PREFIX.length)],
    ["zero_point_mag", zp?.zp ?? null],
    ["zero_point_err_mag", zp?.zp_err ?? null],
  ];
  if (typeof zp?.colour_coeff === "number") {
    entries.push(["zero_point_err_definition", "standard error of zero_point_mag at BP-RP = 0"]);
  }
  entries.push(
    ["colour_coeff", zp?.colour_coeff ?? null],
    ["zero_point_rms_mag", zp?.rms ?? null],
    ["n_used", zp?.n_used ?? null],
    ["n_rejected", zp?.n_rejected ?? null],
    ["n_without_colour", zp?.n_without_colour ?? null],
    ["zero_point_model", zeroPointModelText(cross)],
  );
  if (cross.photcal_present) {
    entries.push(["calibration_note", "the image is already flux-calibrated: the Gaia zero point is informational"]);
  }
  if (kind === "sources") return [...provenanceLines(entries), ...unitLines(SOURCE_UNITS)];
  entries.push(
    ["positions", MATCH_POSITIONS_TEXT],
    ["astrometry_median_d_ra_arcsec", astrometry?.median_d_ra_arcsec ?? null],
    ["astrometry_median_d_dec_arcsec", astrometry?.median_d_dec_arcsec ?? null],
    ["astrometry_rms_arcsec", astrometry?.rms_arcsec ?? null],
  );
  return [...provenanceLines(entries), ...unitLines(MATCH_UNITS)];
}

export type CatalogueCsvItems =
  | { kind: "catalog"; rows: PlacedCatalogRow[] }
  | { kind: "sources"; rows: MeasuredSource[] }
  | { kind: "matches"; matches: CrossMatchEntry[] };

function catalogueCsvBody(items: CatalogueCsvItems): string {
  if (items.kind === "catalog") return catalogRowsCsv(items.rows);
  if (items.kind === "sources") return sourcesCsv(items.rows);
  return matchesCsv(items.matches);
}

export function catalogueCsvText(items: CatalogueCsvItems, headerLines: readonly string[]): string {
  return withProvenance([...headerLines], catalogueCsvBody(items), CSV_LINE_END);
}
