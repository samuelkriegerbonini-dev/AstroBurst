import { describe, expect, it } from "vitest";
import { CSV_LINE_END } from "../catalogCsv";
import { REGION_CSV_COLUMNS, regionSkyOf, regionsCsvFileName, regionsTableCsv } from "../regionCsv";
import type { Region, RegionCalibrated, RegionSky, RegionStats, RegionStatsEntry } from "../../shared/types/regions";

const EXPECTED_HEADER =
  "id,shape,text,include,n,n_excluded,area_px,mean,median,sigma,std,min,max,sum,sum_err,net_sum,net_snr," +
  "flux_source,flux_jy,flux_err_jy,mag_ab,mag_ab_err,sb_mag_arcsec2,area_arcsec2,ra,dec,pa_sky_deg";

function region(id: string, overrides: Partial<Region> = {}): Region {
  return {
    id,
    shape: { shape: "circle", x: 10, y: 20, r: 4 },
    props: { color: null, width: null, text: null, dash: null, include: true },
    backgroundId: null,
    ...overrides,
  };
}

function stats(overrides: Partial<RegionStats> = {}): RegionStats {
  return {
    count: 49,
    n_nan: 0,
    n_excluded: 2,
    n_padding: 0,
    area: 50.27,
    bounds: { x0: 6, y0: 16, x1: 14, y1: 24 },
    clipped: false,
    sum: 196,
    mean: 4,
    median: 4,
    mad: 0,
    sigma: 0,
    std: 0.5,
    min: 3,
    max: 5,
    clipped_mean: 4,
    clipped_median: 4,
    clipped_sigma: 0,
    n_rejected: 0,
    background: null,
    net_sum: null,
    net_snr: null,
    sum_err: null,
    weighted_mean: null,
    calibrated: null,
    ...overrides,
  };
}

function calibrated(overrides: Partial<RegionCalibrated> = {}): RegionCalibrated {
  return {
    flux_source: "sum",
    flux_native: 196,
    flux_err_native: null,
    flux_jy: 3.92e-6,
    flux_err_jy: null,
    mag_ab: 22.417,
    mag_ab_err: null,
    st_mag: null,
    area_arcsec2: 0.04165,
    geometric_area_arcsec2: 0.04273,
    sb_mag_arcsec2: 18.967,
    ra: 150.001,
    dec: 2.002,
    pa_sky_deg: null,
    ...overrides,
  };
}

function sky(overrides: Partial<RegionSky> = {}): RegionSky {
  return { ra: 150, dec: 2, pa_sky_deg: 90, area_arcsec2: 0.04, geometric_area_arcsec2: 0.05, ...overrides };
}

function lastCells(row: string, count: number): string[] {
  return row.split(",").slice(-count);
}

function entry(id: string, s: RegionStats | null, error: string | null = null): RegionStatsEntry {
  return { id, stats: s, error };
}

function lines(csv: string): string[] {
  return csv.split(CSV_LINE_END).filter((l) => l.length > 0);
}

describe("regionsTableCsv", () => {
  it("writes the columns in the documented order", () => {
    expect(REGION_CSV_COLUMNS.map((c) => c.header).join(",")).toBe(EXPECTED_HEADER);
    expect(lines(regionsTableCsv([], new Map()))).toEqual([EXPECTED_HEADER]);
  });

  it("writes one row per region even when its statistics are missing or failed", () => {
    const regions = [region("a"), region("b"), region("c")];
    const table = new Map([
      ["a", entry("a", stats())],
      ["c", entry("c", null, "no finite pixels")],
    ]);
    const rows = lines(regionsTableCsv(regions, table));
    expect(rows).toHaveLength(4);
    expect(rows[1].startsWith('a,"circle (10.0, 20.0) r=4.0",,true,49,2,50.27,4,4,0,0.5,3,5,196,,,,')).toBe(true);
    expect(rows[2]).toBe('b,"circle (10.0, 20.0) r=4.0",,true' + ",".repeat(23));
    expect(rows[3]).toBe('c,"circle (10.0, 20.0) r=4.0",,true' + ",".repeat(23));
  });

  it("leaves every calibrated field blank when the image is uncalibrated", () => {
    const rows = lines(regionsTableCsv([region("a")], new Map([["a", entry("a", stats())]])));
    const cells = rows[1].split(",");
    expect(cells.length).toBeGreaterThan(EXPECTED_HEADER.split(",").length);
    expect(rows[1].endsWith(",".repeat(10))).toBe(true);
  });

  it("writes the calibrated flux, magnitudes, surface brightness, area, sky centre and PA", () => {
    const s = stats({
      net_sum: 147,
      net_snr: 41.2,
      sum_err: 3.5,
      calibrated: calibrated({ flux_source: "net", flux_err_jy: 7e-8, mag_ab_err: 0.02, pa_sky_deg: 120 }),
    });
    const r = region("box", {
      shape: { shape: "box", x: 50, y: 50, width: 8, height: 4, angle: 30 },
      props: { color: "red", width: 2, text: "core, inner", dash: null, include: false },
    });
    const rows = lines(regionsTableCsv([r], new Map([["box", entry("box", s)]])));
    expect(rows[1]).toBe(
      'box,"box (50.0, 50.0) 8.0×4.0 θ=30.0°","core, inner",false,49,2,50.27,4,4,0,0.5,3,5,196,3.5,147,41.2,' +
        "net,0.00000392,7e-8,22.417,0.02,18.967,0.04165,150.001,2.002,120",
    );
  });

  it("writes the sky centre, PA and area of a plate-solved image that has no flux calibration", () => {
    const rows = lines(regionsTableCsv([region("a")], new Map([["a", entry("a", stats({ calibrated: null, sky: sky() }))]])));
    expect(lastCells(rows[1], 10)).toEqual(["", "", "", "", "", "", "0.04", "150", "2", "90"]);
  });

  it("leaves the PA cell blank when the region has no major axis", () => {
    const s = stats({ calibrated: calibrated({ pa_sky_deg: null }), sky: sky({ pa_sky_deg: null }) });
    const rows = lines(regionsTableCsv([region("a")], new Map([["a", entry("a", s)]])));
    expect(lastCells(rows[1], 4)).toEqual(["0.04", "150", "2", ""]);
  });
});

describe("regionSkyOf", () => {
  it("reads the sky block, which the backend sends independently of flux calibration", () => {
    expect(regionSkyOf(stats({ calibrated: null, sky: sky() }))).toEqual(sky());
    expect(regionSkyOf(stats({ calibrated: calibrated({ ra: 1, dec: 1 }), sky: sky() }))?.ra).toBe(150);
  });

  it("falls back to the calibrated block when the response has no sky block", () => {
    const cal = calibrated({ pa_sky_deg: 120 });
    const fallback = regionSkyOf(stats({ calibrated: cal }));
    expect(fallback?.ra).toBe(150.001);
    expect(fallback?.pa_sky_deg).toBe(120);
    expect(fallback?.area_arcsec2).toBe(0.04165);
  });

  it("returns null when the image has neither a WCS nor a pixel area or the statistics are missing", () => {
    expect(regionSkyOf(stats({ sky: null }))).toBeNull();
    expect(regionSkyOf(stats())).toBeNull();
    expect(regionSkyOf(null)).toBeNull();
    expect(regionSkyOf(undefined)).toBeNull();
  });
});

describe("regionsCsvFileName", () => {
  it("names the file after the image stem without the HDU fragment", () => {
    expect(regionsCsvFileName("/data/jw01234_cal.fits#hdu=1")).toBe("jw01234_cal_regions.csv");
    expect(regionsCsvFileName("C:\\images\\m51.fit")).toBe("m51_regions.csv");
    expect(regionsCsvFileName(null)).toBe("regions.csv");
    expect(regionsCsvFileName("")).toBe("regions.csv");
  });
});
