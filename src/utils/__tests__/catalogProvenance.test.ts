import { describe, it, expect } from "vitest";
import {
  catalogueCsvText,
  coneCsvHeaderLines,
  crossMatchCsvHeaderLines,
  detectionLine,
  gaiaEpochText,
  gaiaListLine,
  zeroPointApertureText,
  type CatalogueCsvContext,
} from "../catalogProvenance";
import { provenanceLines, unitLines, type ProvenanceEntry } from "../csvProvenance";
import { catalogRowsCsv, matchesCsv, sourcesCsv } from "../catalogCsv";
import type {
  CatalogQueryInfo,
  ConeSearchResult,
  CrossMatchEntry,
  CrossMatchResult,
  MeasuredSource,
  PlacedCatalogRow,
} from "../../shared/types/catalog";

const CONTEXT: CatalogueCsvContext = { appVersion: "v0.8.0", exportedUtc: "2026-11-11T12:00:00.000Z", fileName: "m16.fits" };

const FULL_CONE_QUERY: CatalogQueryInfo = {
  radius_arcmin: 3.017, field_radius_arcmin: 3.017, mag_limit: null, max_rows: 5000, row_cap_reached: false, cone_smaller_than_field: false,
};

const POSITIONS_TEXT =
  "cat_ra, cat_dec: Gaia position propagated to positions_epoch; d_ra_arcsec = (ra - cat_ra) x cos(mean dec) x 3600 and d_dec_arcsec = (dec - cat_dec) x 3600, star minus Gaia";

const SOURCE_UNITS = {
  x: "px", y: "px", ra: "deg", dec: "deg", flux: "image unit (net flux in the aperture)", mag_inst: "mag", mag_ab: "mag (AB)", fwhm: "px",
  aperture_radius: "px",
};

const MATCH_UNITS = {
  ...SOURCE_UNITS,
  cat_ra: "deg", cat_dec: "deg", g: "mag (Vega)", bp: "mag (Vega)", rp: "mag (Vega)", bp_rp: "mag (Vega)",
  sep_arcsec: "arcsec", d_ra_arcsec: "arcsec", d_dec_arcsec: "arcsec",
};

function source(overrides: Partial<MeasuredSource> = {}): MeasuredSource {
  return {
    x: 30, y: 31.5, ra: 274.700123, dec: -13.800456, flux: 1234.5, fwhm: 4.7, snr: 120, mag_inst: -7.73, mag_ab: null, saturated: false,
    aperture_radius: 5,
    ...overrides,
  };
}

function placedRow(): PlacedCatalogRow {
  return {
    id: "4146599932126633216", ra: 274.7, dec: -13.8, ra_epoch: 274.7000012, dec_epoch: -13.8000034, pm_ra_masyr: 1.2, pm_dec_masyr: -3.4,
    g: 12.1, bp: 12.5, rp: 11.6, bp_rp: 0.9, parallax_mas: 0.5, x: 10.25, y: 20.5, on_image: true,
  };
}

function matchEntry(): CrossMatchEntry {
  return {
    star: source(),
    row: { id: "4146599932126633216", ra: 274.70012, dec: -13.80046, g: 12.1, bp: 12.5, rp: 11.6, bp_rp: 0.9 },
    sep_arcsec: 0.2,
    d_ra_arcsec: 0.01,
    d_dec_arcsec: -0.004,
    zp_status: "used",
  };
}

function coneResult(): ConeSearchResult {
  return {
    rows: [placedRow()],
    epoch_year: 2024.5,
    n_total: 1,
    n_on_image: 1,
    radius_arcmin: 12,
    center_ra: 274.7,
    center_dec: -13.8,
    elapsed_ms: 40,
    warnings: [],
    catalog_source: "disk",
    mag_limit: 18,
    max_rows: 5000,
    catalog_table: "I/355/gaiadr3",
  };
}

function crossMatchResult(overrides: Partial<CrossMatchResult> = {}): CrossMatchResult {
  const matches = [matchEntry()];
  return {
    matches,
    sources: [source(), source({ x: 40 })],
    astrometry: { median_d_ra_arcsec: 0.01, median_d_dec_arcsec: -0.004, rms_arcsec: 0.052, n: 1 },
    zero_point: {
      zp: 25.31, zp_err: 0.02, colour_coeff: 0.08, rms: 0.05, n_used: 38, n_rejected: 2, n_without_colour: 1, band: "G", colour_term_used: true,
    },
    photcal_present: true,
    epoch_year: 2024.5,
    n_detected: 60,
    n_catalog: 120,
    band: "G",
    match_radius_arcsec: 2,
    elapsed_ms: 9,
    warnings: [],
    catalog_source: "network",
    aperture: { kind: "fixed", radius_px: 5 },
    catalog_query: FULL_CONE_QUERY,
    catalog_table: "I/355/gaiadr3",
    mag_system: "vega",
    detection_sigma: 5,
    max_stars: 500,
    ...overrides,
  };
}

function lineOf(lines: string[], key: string): string | undefined {
  return lines.find((line) => line === `# ${key}:` || line.startsWith(`# ${key}: `));
}

describe("gaiaEpochText", () => {
  it("prints the epoch of the Gaia positions", () => {
    expect(gaiaEpochText(2024.5)).toBe("J2024.50");
    expect(gaiaEpochText(null)).toBe("J2016.0 (no observation date in the header)");
    expect(gaiaEpochText(Number.NaN)).toBe("J2016.0 (no observation date in the header)");
  });
});

describe("zeroPointApertureText", () => {
  it("states the aperture of the zero point as the panel does", () => {
    expect(zeroPointApertureText({ kind: "fixed", radius_px: 5 })).toBe("aperture r = 5.00 px");
    expect(
      zeroPointApertureText({ kind: "auto", fwhm_factor: 1.5, median_radius_px: 7.06, min_radius_px: 4.62, max_radius_px: 9.06 }),
    ).toBe("aperture 1.5 x FWHM per star, median r = 7.06 px (4.62 to 9.06)");
    expect(
      zeroPointApertureText({ kind: "auto", fwhm_factor: 1.5, median_radius_px: null, min_radius_px: null, max_radius_px: null }),
    ).toBe("aperture 1.5 x FWHM per star, median r = -- px (-- to --)");
  });
});

describe("gaiaListLine", () => {
  it("says which Gaia list the cross-match used", () => {
    expect(gaiaListLine(crossMatchResult({ n_catalog: 120, catalog_source: "network", catalog_query: FULL_CONE_QUERY }))).toBe(
      "Gaia list: 120 rows within 3.0' of the centre, no G limit, row cap 5000, from VizieR",
    );
    const cappedQuery: CatalogQueryInfo = {
      radius_arcmin: 1, field_radius_arcmin: 1.5085, mag_limit: 13.8, max_rows: 200, row_cap_reached: true, cone_smaller_than_field: true,
    };
    expect(gaiaListLine(crossMatchResult({ n_catalog: 200, catalog_source: "memory", catalog_query: cappedQuery }))).toBe(
      "Gaia list: 200 rows within 1.0' of the centre (the field reaches 1.5'), G < 13.8, row cap 200 (reached), from the memory cache",
    );
    expect(gaiaListLine(crossMatchResult({ catalog_query: undefined }))).toBeNull();
  });
});

describe("detectionLine", () => {
  it("says how the stars were detected", () => {
    const sources = Array.from({ length: 58 }, (_, i) => source({ x: i }));
    expect(detectionLine(crossMatchResult({ detection_sigma: 5, max_stars: 500, n_detected: 60, sources }))).toBe(
      "detection at 5 sigma: 60 detections, at most 500 kept, 58 measured",
    );
    expect(detectionLine(crossMatchResult({ detection_sigma: undefined, max_stars: undefined }))).toBeNull();
  });
});

describe("coneCsvHeaderLines", () => {
  it("heads the catalogue CSV with the cone, the epoch and the units", () => {
    const lines = coneCsvHeaderLines(coneResult(), CONTEXT);
    const entries: ProvenanceEntry[] = [
      ["software", "AstroBurst v0.8.0"],
      ["exported_utc", "2026-11-11T12:00:00.000Z"],
      ["file", "m16.fits"],
      ["catalogue", "Gaia DR3 (VizieR I/355/gaiadr3)"],
      ["catalogue_source", "disk"],
      ["cone_centre_ra_deg", 274.7],
      ["cone_centre_dec_deg", -13.8],
      ["cone_radius_arcmin", 12],
      ["g_limit_mag", 18],
      ["max_rows", 5000],
      ["n_rows", 1],
      ["n_on_image", 1],
      ["positions_epoch", "J2024.50"],
      [
        "positions",
        "ra, dec: Gaia DR3 at J2016.0; ra_epoch, dec_epoch: propagated to positions_epoch with the proper motions; x, y: 0-based pixel of ra_epoch, dec_epoch",
      ],
      ["magnitude_system", "Vega"],
    ];
    const units = {
      ra: "deg", dec: "deg", ra_epoch: "deg", dec_epoch: "deg", pm_ra_masyr: "mas/yr", pm_dec_masyr: "mas/yr", parallax_mas: "mas",
      g: "mag (Vega)", bp: "mag (Vega)", rp: "mag (Vega)", bp_rp: "mag (Vega)", x: "px", y: "px",
    };
    expect(entries).toHaveLength(15);
    expect(Object.keys(units)).toHaveLength(13);
    expect(lines).toEqual([...provenanceLines(entries), ...unitLines(units)]);
    expect(lines.slice(0, 3).map((line) => line.split(":")[0])).toEqual(["# software", "# exported_utc", "# file"]);
    expect(lineOf(lines, "catalogue")).toContain("Gaia DR3 (VizieR I/355/gaiadr3)");
    expect(lineOf(lines, "positions_epoch")).toContain("J2024.50");
  });
});

function crossMatchEntries(cross: CrossMatchResult, colourTerm: boolean): ProvenanceEntry[] {
  const head: ProvenanceEntry[] = [
    ["software", "AstroBurst v0.8.0"],
    ["exported_utc", "2026-11-11T12:00:00.000Z"],
    ["file", "m16.fits"],
    ["catalogue", "Gaia DR3 (VizieR I/355/gaiadr3)"],
    ["catalogue_source", "network"],
    ["cone_radius_arcmin", 3.017],
    ["g_limit_mag", null],
    ["max_rows", 5000],
    ["n_catalogue_rows", 120],
    ["positions_epoch", "J2024.50"],
    ["band", "Gaia G"],
    ["magnitude_system", "Vega"],
    ["match_radius_arcsec", 2],
    ["detection_sigma", 5],
    ["max_stars", 500],
    ["n_detected", 60],
    ["n_measured", cross.sources.length],
    ["n_matches", cross.matches.length],
    ["aperture", "r = 5.00 px"],
    ["zero_point_mag", 25.31],
    ["zero_point_err_mag", 0.02],
  ];
  const definition: ProvenanceEntry[] = colourTerm
    ? [["zero_point_err_definition", "standard error of zero_point_mag at BP-RP = 0"]]
    : [];
  const tail: ProvenanceEntry[] = [
    ["colour_coeff", colourTerm ? 0.08 : null],
    ["zero_point_rms_mag", 0.05],
    ["n_used", 38],
    ["n_rejected", 2],
    ["n_without_colour", 1],
    [
      "zero_point_model",
      colourTerm ? "Gaia G - mag_inst = zero_point_mag + colour_coeff x (BP-RP)" : "Gaia G - mag_inst = zero_point_mag",
    ],
    ["calibration_note", "the image is already flux-calibrated: the Gaia zero point is informational"],
  ];
  return [...head, ...definition, ...tail];
}

const MATCH_ENTRIES: ProvenanceEntry[] = [
  ["positions", POSITIONS_TEXT],
  ["astrometry_median_d_ra_arcsec", 0.01],
  ["astrometry_median_d_dec_arcsec", -0.004],
  ["astrometry_rms_arcsec", 0.052],
];

describe("crossMatchCsvHeaderLines", () => {
  it("heads the matches CSV with the list, the band, the system, the aperture, the zero point and the residual definition", () => {
    const cross = crossMatchResult();
    const lines = crossMatchCsvHeaderLines("matches", cross, CONTEXT);
    expect(lines).toEqual([...provenanceLines([...crossMatchEntries(cross, true), ...MATCH_ENTRIES]), ...unitLines(MATCH_UNITS)]);
    expect(lines).toContain("# band: Gaia G");
    expect(lines).toContain("# magnitude_system: Vega");
    expect(lines).toContain("# aperture: r = 5.00 px");
    expect(lines).toContain("# zero_point_mag: 25.31");
    expect(lines).toContain("# zero_point_model: Gaia G - mag_inst = zero_point_mag + colour_coeff x (BP-RP)");
    const errIndex = lines.indexOf("# zero_point_err_mag: 0.02");
    expect(errIndex).toBeGreaterThan(0);
    expect(lines[errIndex + 1]).toBe("# zero_point_err_definition: standard error of zero_point_mag at BP-RP = 0");
    expect(lineOf(lines, "calibration_note")).toBeDefined();
    expect(lineOf(lines, "positions")).toBe(`# positions: ${POSITIONS_TEXT}`);
    expect(lineOf(lines, "unit cat_ra")).toBe("# unit cat_ra: deg");
    expect(lineOf(lines, "unit d_ra_arcsec")).toBe("# unit d_ra_arcsec: arcsec");
    expect(lines.some((line) => line.includes("AB") && !line.startsWith("# unit mag_ab:"))).toBe(false);

    const withoutColour = crossMatchResult({ zero_point: { ...cross.zero_point!, colour_coeff: null, colour_term_used: false } });
    const plainLines = crossMatchCsvHeaderLines("matches", withoutColour, CONTEXT);
    expect(plainLines).toEqual([
      ...provenanceLines([...crossMatchEntries(withoutColour, false), ...MATCH_ENTRIES]),
      ...unitLines(MATCH_UNITS),
    ]);
    expect(lineOf(plainLines, "zero_point_err_definition")).toBeUndefined();
  });

  it("heads the sources CSV without the residual definition", () => {
    const cross = crossMatchResult();
    const lines = crossMatchCsvHeaderLines("sources", cross, CONTEXT);
    expect(lines).toEqual([...provenanceLines(crossMatchEntries(cross, true)), ...unitLines(SOURCE_UNITS)]);
    expect(lineOf(lines, "positions")).toBeUndefined();
    expect(lines.some((line) => line.startsWith("# astrometry_"))).toBe(false);
    expect(lineOf(lines, "unit cat_ra")).toBeUndefined();
    expect(lineOf(lines, "unit aperture_radius")).toBe("# unit aperture_radius: px");
  });

  it("writes empty values for what an older result does not carry and for a missing fit", () => {
    const older = crossMatchResult({
      catalog_query: undefined, catalog_table: undefined, aperture: undefined, detection_sigma: undefined, max_stars: undefined,
      zero_point: null, astrometry: null, photcal_present: false, catalog_source: undefined, mag_system: undefined,
    });
    const entries: ProvenanceEntry[] = [
      ["software", "AstroBurst v0.8.0"],
      ["exported_utc", "2026-11-11T12:00:00.000Z"],
      ["file", "m16.fits"],
      ["catalogue", "Gaia DR3"],
      ["catalogue_source", null],
      ["cone_radius_arcmin", null],
      ["g_limit_mag", null],
      ["max_rows", null],
      ["n_catalogue_rows", 120],
      ["positions_epoch", "J2024.50"],
      ["band", "Gaia G"],
      ["magnitude_system", "Vega"],
      ["match_radius_arcsec", 2],
      ["detection_sigma", null],
      ["max_stars", null],
      ["n_detected", 60],
      ["n_measured", 2],
      ["n_matches", 1],
      ["aperture", null],
      ["zero_point_mag", null],
      ["zero_point_err_mag", null],
      ["colour_coeff", null],
      ["zero_point_rms_mag", null],
      ["n_used", null],
      ["n_rejected", null],
      ["n_without_colour", null],
      ["zero_point_model", "none"],
      ["positions", POSITIONS_TEXT],
      ["astrometry_median_d_ra_arcsec", null],
      ["astrometry_median_d_dec_arcsec", null],
      ["astrometry_rms_arcsec", null],
    ];
    const lines = crossMatchCsvHeaderLines("matches", older, CONTEXT);
    expect(lines).toEqual([...provenanceLines(entries), ...unitLines(MATCH_UNITS)]);
    expect(lines).toContain("# catalogue: Gaia DR3");
    expect(lines).toContain("# zero_point_model: none");
    expect(lines).toContain("# zero_point_mag:");
  });

  it("every header line starts with # and holds no line break", () => {
    const context = { ...CONTEXT, fileName: "bad\nname.fits" };
    const all = [
      ...coneCsvHeaderLines(coneResult(), context),
      ...crossMatchCsvHeaderLines("sources", crossMatchResult(), context),
      ...crossMatchCsvHeaderLines("matches", crossMatchResult(), context),
    ];
    for (const line of all) expect(line).toMatch(/^#[^\r\n]*$/);
    expect(all).toContain("# file: bad name.fits");
  });
});

describe("catalogueCsvText", () => {
  it("the copied text is the header lines and then the table, each line ended by CRLF", () => {
    const header = ["# a: 1", "# unit x: px"];
    const prefix = "# a: 1\r\n# unit x: px\r\n";
    const entry = matchEntry();
    expect(catalogueCsvText({ kind: "matches", matches: [entry] }, header)).toBe(prefix + matchesCsv([entry]));
    expect(catalogueCsvText({ kind: "matches", matches: [entry] }, [])).toBe(matchesCsv([entry]));
    const rows = [placedRow()];
    expect(catalogueCsvText({ kind: "catalog", rows }, header)).toBe(prefix + catalogRowsCsv(rows));
    expect(catalogueCsvText({ kind: "catalog", rows }, [])).toBe(catalogRowsCsv(rows));
    const sources = [source()];
    expect(catalogueCsvText({ kind: "sources", rows: sources }, header)).toBe(prefix + sourcesCsv(sources));
    expect(catalogueCsvText({ kind: "sources", rows: sources }, [])).toBe(sourcesCsv(sources));
  });
});
