import { describe, it, expect } from "vitest";
import {
  PHOTOMETRY_COLUMN_KEYS,
  PHOTOMETRY_CSV_COLUMNS,
  flagDuplicates,
  growthReferenceLines,
  medianSnr,
  parsePositions,
  photometryCsvFileName,
  photometryTableCsv,
  rowValue,
  sortRows,
  type PhotometryTableRow,
} from "../photometryTable";
import { CSV_LINE_END } from "../catalogCsv";
import type { StarPhotometry } from "../../services/analysis";

function phot(overrides: Partial<StarPhotometry> = {}): StarPhotometry {
  return {
    x: 10,
    y: 20,
    peak: 900,
    net_flux: 1000,
    flux_err: 10,
    mag_inst: -7.5,
    snr: 100,
    fwhm: 3.2,
    aperture_radius: 5,
    aperture_pixels: 79,
    aperture_area: 78.5,
    bg_mean: 100,
    bg_sigma: 2,
    bg_pixels: 300,
    saturated: false,
    saturation_source: "none",
    n_masked: 0,
    n_saturated: 0,
    err_used: false,
    aperture_correction: null,
    flux_total: null,
    plateau_radius: null,
    flux_jy: null,
    flux_err_jy: null,
    mag_ab: null,
    mag_ab_err: null,
    mag_ab_total: null,
    st_mag: null,
    sky_inner: 10,
    sky_outer: 15,
    growth_curve: [],
    ee50_radius: null,
    ee80_radius: null,
    ...overrides,
  };
}

function row(index: number, p: Partial<StarPhotometry> | null, overrides: Partial<PhotometryTableRow> = {}): PhotometryTableRow {
  return { index, label: `s${index}`, photometry: p ? phot(p) : null, sky: null, error: p ? null : "failed", ...overrides };
}

describe("parsePositions", () => {
  it("accepts space, comma and tab separated pairs and skips comments and blanks", () => {
    const { points, errors } = parsePositions("# header\n10 20\n\n10,20\n10.5\t20.25\n  30 , 40  \n");
    expect(points).toEqual([
      [10, 20],
      [10, 20],
      [10.5, 20.25],
      [30, 40],
    ]);
    expect(errors).toEqual([]);
  });

  it("reports the line number of a malformed entry and keeps the valid ones", () => {
    const { points, errors } = parsePositions("1 2\nabc\n3 4 5\n7 NaN\n\n8 9");
    expect(points).toEqual([
      [1, 2],
      [8, 9],
    ]);
    expect(errors).toHaveLength(3);
    expect(errors[0]).toContain("line 2");
    expect(errors[1]).toContain("line 3");
    expect(errors[2]).toContain("line 4");
  });

  it("returns nothing for an empty text", () => {
    expect(parsePositions("")).toEqual({ points: [], errors: [] });
  });
});

describe("flagDuplicates", () => {
  it("flags the second of two rows half a pixel apart and not rows two pixels apart", () => {
    const rows = [row(0, { x: 10, y: 10 }), row(1, { x: 10.3, y: 10.4 }), row(2, { x: 12, y: 10 }), row(3, null)];
    expect([...flagDuplicates(rows)]).toEqual([1]);
    expect(flagDuplicates(rows, 0.1).size).toBe(0);
    expect(flagDuplicates(rows, 3).size).toBe(2);
  });

  it("flags matches across grid cell boundaries", () => {
    const rows = [row(0, { x: 0.99, y: 0.99 }), row(1, { x: 1.01, y: 1.01 })];
    expect([...flagDuplicates(rows)]).toEqual([1]);
  });

  it("ignores an invalid tolerance", () => {
    const rows = [row(0, { x: 1, y: 1 }), row(1, { x: 1, y: 1 })];
    expect(flagDuplicates(rows, 0).size).toBe(0);
    expect(flagDuplicates(rows, Number.NaN).size).toBe(0);
  });
});

describe("sortRows", () => {
  const rows = [row(0, { snr: 5 }), row(1, null), row(2, { snr: 50 }), row(3, { snr: Number.NaN }), row(4, { snr: 20 })];

  it("sorts numerically with nulls last in both directions", () => {
    expect(sortRows(rows, "snr", "asc").map((r) => r.index)).toEqual([0, 4, 2, 1, 3]);
    expect(sortRows(rows, "snr", "desc").map((r) => r.index)).toEqual([2, 4, 0, 1, 3]);
  });

  it("sorts strings and booleans and keeps the input order for ties", () => {
    const mixed = [row(0, { saturated: true }), row(1, { saturated: false }), row(2, { saturated: true })];
    expect(sortRows(mixed, "saturated", "desc").map((r) => r.index)).toEqual([0, 2, 1]);
    expect(sortRows(mixed, "label", "desc").map((r) => r.index)).toEqual([2, 1, 0]);
    expect(sortRows(rows, "index", "asc")).not.toBe(rows);
  });
});

describe("photometryTableCsv", () => {
  it("writes the columns in the specified order with blank nulls", () => {
    expect(PHOTOMETRY_CSV_COLUMNS.map((c) => c.header)).toEqual([...PHOTOMETRY_COLUMN_KEYS]);
    expect(PHOTOMETRY_COLUMN_KEYS[0]).toBe("index");
    expect(PHOTOMETRY_COLUMN_KEYS[PHOTOMETRY_COLUMN_KEYS.length - 1]).toBe("error");
    const rows = [
      row(0, { x: 1.5, y: 2.5, mag_ab: 17.25, saturated: true, err_used: true }, { sky: { ra: 150.1, dec: 2.2 } }),
      row(1, null),
    ];
    const csv = photometryTableCsv(rows);
    const lines = csv.split(CSV_LINE_END);
    expect(lines[0].startsWith("index,label,x,y,ra,dec,net_flux,flux_err,snr,mag_inst,mag_ab,")).toBe(true);
    expect(lines[0].endsWith(",ee50_radius,ee80_radius,saturated,n_masked,err_used,error")).toBe(true);
    const first = lines[1].split(",");
    expect(first.slice(0, 6)).toEqual(["0", "s0", "1.5", "2.5", "150.1", "2.2"]);
    expect(first[10]).toBe("17.25");
    expect(first[first.length - 4]).toBe("true");
    expect(first[first.length - 2]).toBe("true");
    expect(first[first.length - 1]).toBe("");
    const second = lines[2].split(",");
    expect(second[0]).toBe("1");
    expect(second[2]).toBe("");
    expect(second[second.length - 1]).toBe("failed");
    expect(second.length).toBe(PHOTOMETRY_COLUMN_KEYS.length);
  });

  it("blanks non-finite numbers", () => {
    expect(rowValue(row(0, { mag_inst: Number.NaN }), "mag_inst")).toBeNull();
    expect(rowValue(row(0, null), "n_masked")).toBeNull();
  });
});

describe("medianSnr", () => {
  it("takes the median over measured rows only", () => {
    expect(medianSnr([row(0, { snr: 1 }), row(1, null), row(2, { snr: 9 }), row(3, { snr: 4 })])).toBe(4);
    expect(medianSnr([row(0, { snr: 1 }), row(1, { snr: 3 })])).toBe(2);
    expect(medianSnr([row(0, null)])).toBeNull();
  });
});

describe("photometryCsvFileName", () => {
  it("derives the name from the image stem", () => {
    expect(photometryCsvFileName("/data/ngc1234_cal.fits")).toBe("ngc1234_cal_photometry.csv");
    expect(photometryCsvFileName("C:\\img\\m31.fit.gz")).toBe("m31_photometry.csv");
  });
});

describe("growthReferenceLines", () => {
  it("draws the aperture, both sky radii, the plateau and EE50 as vertical lines in that order", () => {
    const lines = growthReferenceLines(phot({ aperture_radius: 5, sky_inner: 10, sky_outer: 15, plateau_radius: 12, ee50_radius: 2.5 }));
    expect(lines.map((l) => l.label)).toEqual(["r_ap", "sky in", "sky out", "plateau", "EE50"]);
    expect(lines.map((l) => l.value)).toEqual([5, 10, 15, 12, 2.5]);
    expect(lines.every((l) => l.axis === "x")).toBe(true);
    expect(lines.map((l) => l.dashed)).toEqual([false, false, false, true, true]);
    expect(lines[0].color).not.toBe(lines[1].color);
    expect(lines[1].color).toBe(lines[2].color);
    expect(lines[3].color).toBe(lines[4].color);
  });

  it("omits null and non-finite radii without disturbing the order of the rest", () => {
    const lines = growthReferenceLines(phot({ aperture_radius: Number.NaN, sky_inner: 10, sky_outer: Number.POSITIVE_INFINITY, plateau_radius: null, ee50_radius: 3 }));
    expect(lines.map((l) => l.label)).toEqual(["sky in", "EE50"]);
  });

  it("returns nothing when no radius is usable", () => {
    expect(growthReferenceLines(phot({ aperture_radius: Number.NaN, sky_inner: Number.NaN, sky_outer: Number.NaN }))).toEqual([]);
  });
});
