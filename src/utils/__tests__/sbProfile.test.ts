import { describe, it, expect } from "vitest";
import {
  SB_CSV_COLUMNS,
  binMeanError,
  firstSurfaceBrightness,
  formatPositionAngles,
  formatRadius,
  parseBinWidth,
  profileFetchReducer,
  profileNotes,
  psfReferenceLine,
  sbProfileCsv,
  sbProfileCsvWithProvenance,
  sbReferenceLines,
  sbSeries,
  sbTotalFlux,
  withUnit,
} from "../sbProfile";
import { CSV_LINE_END } from "../catalogCsv";
import { provenanceLines, unitLines, withProvenance, type ProvenanceEntry } from "../csvProvenance";
import type { PhotCal } from "../../services/analysis";
import type { SbBin, SbProfile } from "../../shared/types/regions";

function bin(overrides: Partial<SbBin> & { sma: number }): SbBin {
  const { sma } = overrides;
  return {
    sma_inner: sma - 0.5,
    sma_outer: sma + 0.5,
    count: 10,
    cumulative_count: 10,
    mean: 100 / sma,
    median: 90 / sma,
    std: 4,
    cumulative_sum: 100,
    sma_arcsec: sma / 20,
    mu_ab: 20 + sma / 10,
    mu_err: 0.01,
    mag_ab_cumulative: 15,
    ...overrides,
  };
}

function profile(overrides: Partial<SbProfile> = {}): SbProfile {
  return {
    x: 50,
    y: 50,
    sma_max: 3,
    ellipticity: 0.4,
    angle_deg: 30,
    bin_width: 1,
    background: null,
    bins: [
      bin({ sma: 0.5, cumulative_sum: 50, cumulative_count: 4 }),
      bin({ sma: 1.5, cumulative_sum: 80, cumulative_count: 14 }),
      bin({ sma: 2.5, cumulative_sum: 100, cumulative_count: 30 }),
    ],
    r50_px: 0.5,
    r80_px: 1.5,
    r90_px: 2.0,
    petrosian_radius_px: 2.4,
    pixel_scale_arcsec: 0.05,
    pixel_area_arcsec2: 0.0025,
    sky_pa_deg: 120,
    photcal: null,
    calibration_warnings: [],
    total_mag_ab: 14.5,
    notes: [],
    masked: false,
    dq_excluded: null,
    region_excluded: 0,
    elapsed_ms: 1,
    ...overrides,
  };
}

describe("sbSeries", () => {
  it("plots the mean with its standard error and the median against the semi-major axis in pixels", () => {
    const layout = sbSeries(profile(), "mean", "px");
    expect(layout.series).toHaveLength(2);
    expect(layout.series[0].label).toBe("mean");
    expect(layout.series[0].x).toEqual([0.5, 1.5, 2.5]);
    expect(layout.series[0].yErr).toEqual([4 / Math.sqrt(10), 4 / Math.sqrt(10), 4 / Math.sqrt(10)]);
    expect(layout.series[1].label).toBe("median");
    expect(layout.invertY).toBe(false);
    expect(layout.yLabel).toBe("mean");
    expect(layout.xLabel).toBe("semi-major axis (px)");
    expect(sbSeries(profile({ background: { median: 1, sigma: 0.1, count: 5 } }), "mean", "px").yLabel).toBe("mean - bg");
  });

  it("inverts the y axis in mu mode and carries mu_err as the error bar", () => {
    const layout = sbSeries(profile(), "mu", "px");
    expect(layout.invertY).toBe(true);
    expect(layout.series).toHaveLength(1);
    expect(layout.series[0].y).toEqual([20.05, 20.15, 20.25]);
    expect(layout.series[0].yErr).toEqual([0.01, 0.01, 0.01]);
    expect(layout.yLabel).toContain("mag/arcsec^2");
  });

  it("normalises the enclosed energy to the last bin so it ends at 1 and grows monotonically", () => {
    const layout = sbSeries(profile(), "ee", "px");
    const y = layout.series[0].y as number[];
    expect(y[y.length - 1]).toBe(1);
    for (let i = 1; i < y.length; i++) expect(y[i]).toBeGreaterThanOrEqual(y[i - 1]);
    expect(layout.series[0].x).toEqual([1, 2, 3]);
    expect(sbTotalFlux(profile())).toBe(100);
  });

  it("returns null enclosed fractions when the total flux is not positive", () => {
    const negative = profile({ bins: [bin({ sma: 0.5, cumulative_sum: -3 })] });
    expect(sbSeries(negative, "ee", "px").series[0].y).toEqual([null]);
    expect(sbTotalFlux(negative)).toBeNull();
    expect(sbTotalFlux(profile({ bins: [] }))).toBeNull();
  });

  it("uses sma_arcsec on the x axis when arcsec is requested and a pixel scale exists", () => {
    const layout = sbSeries(profile(), "mu", "arcsec");
    expect(layout.series[0].x).toEqual([0.025, 0.075, 0.125]);
    expect(layout.xLabel).toBe("semi-major axis (arcsec)");
    const ee = sbSeries(profile(), "ee", "arcsec");
    expect(ee.series[0].x[2]).toBeCloseTo(0.15, 12);
  });

  it("falls back to pixels when arcsec is requested without a pixel scale", () => {
    const layout = sbSeries(profile({ pixel_scale_arcsec: null }), "mean", "arcsec");
    expect(layout.series[0].x).toEqual([0.5, 1.5, 2.5]);
    expect(layout.xLabel).toBe("semi-major axis (px)");
  });

  it("drops non-finite means and mu values instead of plotting them", () => {
    const p = profile({ bins: [bin({ sma: 0.5, mean: null, mu_ab: null, mu_err: null, std: null, count: 0 })] });
    expect(sbSeries(p, "mean", "px").series[0].y).toEqual([null]);
    expect(sbSeries(p, "mean", "px").series[0].yErr).toEqual([null]);
    expect(sbSeries(p, "mu", "px").series[0].y).toEqual([null]);
  });

  it("keeps the value of a single-pixel bin but draws no error bar for it", () => {
    const single = bin({ sma: 0.5, count: 1, std: null, mu_err: null });
    const p = profile({ bins: [single, bin({ sma: 1.5 })] });
    const mean = sbSeries(p, "mean", "px").series[0];
    expect(mean.y[0]).toBe(200);
    expect(mean.yErr).toEqual([null, 4 / Math.sqrt(10)]);
    const mu = sbSeries(p, "mu", "px").series[0];
    expect(mu.y).toEqual([20.05, 20.15]);
    expect(mu.yErr).toEqual([null, 0.01]);
    expect(firstSurfaceBrightness(p)).toBe(20.05);
  });

  it("draws no error bar for a single-pixel bin even when the payload carries a zero spread", () => {
    const p = profile({ bins: [bin({ sma: 0.5, count: 1, std: 0, mu_err: 0 })] });
    expect(sbSeries(p, "mean", "px").series[0].yErr).toEqual([null]);
    expect(sbSeries(p, "mu", "px").series[0].yErr).toEqual([null]);
  });

  it("the linear y title carries the unit of the image", () => {
    expect(sbSeries(profile(), "mean", "px", "MJy/sr").yLabel).toBe("mean (MJy/sr)");
    expect(sbSeries(profile({ background: { median: 1, sigma: 0.1, count: 5 } }), "mean", "px", "MJy/sr").yLabel).toBe("mean - bg (MJy/sr)");
    expect(sbSeries(profile(), "mean", "px", null).yLabel).toBe("mean");
    expect(sbSeries(profile(), "mean", "px", undefined).yLabel).toBe("mean");
    expect(sbSeries(profile(), "mean", "px", "  ").yLabel).toBe("mean");
    expect(sbSeries(profile(), "mu", "px", "MJy/sr").yLabel).toBe("mu (mag/arcsec^2)");
    expect(sbSeries(profile(), "ee", "px", "MJy/sr").yLabel).toBe("enclosed fraction");
    expect(sbSeries(profile(), "mean", "px", "MJy/sr").series).toEqual(sbSeries(profile(), "mean", "px").series);
  });
});

describe("sbReferenceLines", () => {
  it("marks R50, R80 and the Petrosian radius on the x axis in the requested unit", () => {
    const px = sbReferenceLines(profile(), "px");
    expect(px.map((l) => [l.label, l.value, l.axis])).toEqual([
      ["R50", 0.5, "x"],
      ["R80", 1.5, "x"],
      ["R_P", 2.4, "x"],
    ]);
    const arcsec = sbReferenceLines(profile(), "arcsec");
    expect(arcsec.map((l) => Number(l.value.toFixed(6)))).toEqual([0.025, 0.075, 0.12]);
  });

  it("omits the reference lines whose radius is null", () => {
    const lines = sbReferenceLines(profile({ r50_px: null, petrosian_radius_px: null }), "px");
    expect(lines.map((l) => l.label)).toEqual(["R80"]);
    expect(sbReferenceLines(profile({ r50_px: null, r80_px: null, petrosian_radius_px: null }), "px")).toEqual([]);
  });
});

describe("units, PSF mark, notes and bin errors", () => {
  it("withUnit adds the unit in parentheses only when there is one", () => {
    expect(withUnit("value", "nanomaggy")).toBe("value (nanomaggy)");
    expect(withUnit("value - bg", null)).toBe("value - bg");
    expect(withUnit("value", " ")).toBe("value");
    expect(withUnit("value", undefined)).toBe("value");
  });

  it("the PSF mark is the half-light radius of the stars, or half the FWHM", () => {
    const fwhmOnly = { fwhmArcsec: 1.0, halfLightRadiusArcsec: null };
    expect(psfReferenceLine(fwhmOnly, 0.25, "px")).toEqual({ axis: "x", value: 2, label: "PSF FWHM/2", color: "#c4b5fd", dashed: true });
    expect(psfReferenceLine(fwhmOnly, 0.25, "arcsec")?.value).toBe(0.5);
    const halfLight = { fwhmArcsec: 1.0, halfLightRadiusArcsec: 0.55 };
    const px = psfReferenceLine(halfLight, 0.25, "px");
    expect(px?.label).toBe("PSF R50");
    expect(Math.abs((px?.value ?? Number.NaN) - 2.2)).toBeLessThan(1e-12);
    expect(psfReferenceLine(halfLight, 0.25, "arcsec")).toEqual({ axis: "x", value: 0.55, label: "PSF R50", color: "#c4b5fd", dashed: true });
    expect(psfReferenceLine({ fwhmArcsec: null, halfLightRadiusArcsec: null }, 0.25, "px")).toBeNull();
    expect(psfReferenceLine(fwhmOnly, null, "px")).toBeNull();
    expect(psfReferenceLine(fwhmOnly, 0, "px")).toBeNull();
  });

  it("profileNotes reads the notes a result may carry", () => {
    expect(profileNotes({ notes: ["a", 3, "b"] })).toEqual(["a", "b"]);
    expect(profileNotes({})).toEqual([]);
    expect(profileNotes(null)).toEqual([]);
    expect(profileNotes({ notes: "x" })).toEqual([]);
  });

  it("binMeanError is the standard error of the mean of a bin", () => {
    expect(binMeanError(2, 4)).toBe(1);
    expect(binMeanError(2, 1)).toBeNull();
    expect(binMeanError(null, 9)).toBeNull();
    expect(binMeanError(undefined, 9)).toBeNull();
    expect(binMeanError(Number.NaN, 9)).toBeNull();
    expect(binMeanError(0, 2)).toBe(0);
  });
});

describe("sbProfileCsv", () => {
  it("writes the documented column order and blanks null cells", () => {
    expect(SB_CSV_COLUMNS.map((c) => c.header)).toEqual([
      "sma_px",
      "sma_arcsec",
      "sma_inner",
      "sma_outer",
      "count",
      "mean",
      "median",
      "std",
      "cumulative_sum",
      "mu_ab",
      "mu_err",
      "mag_ab_cumulative",
    ]);
    const p = profile({
      bins: [bin({ sma: 1.5, sma_arcsec: null, mean: null, median: null, std: null, mu_ab: null, mu_err: null, mag_ab_cumulative: null, count: 0, cumulative_sum: 0 })],
    });
    const lines = sbProfileCsv(p).split(CSV_LINE_END);
    expect(lines[0]).toBe("sma_px,sma_arcsec,sma_inner,sma_outer,count,mean,median,std,cumulative_sum,mu_ab,mu_err,mag_ab_cumulative");
    expect(lines[1]).toBe("1.5,,1,2,0,,,,0,,,");
    expect(lines[2]).toBe("");
  });

  it("leaves std and mu_err blank for a single-pixel bin while keeping its surface brightness", () => {
    const p = profile({ bins: [bin({ sma: 0.5, count: 1, cumulative_count: 1, std: null, mu_err: null })] });
    const cells = sbProfileCsv(p).split(CSV_LINE_END)[1].split(",");
    expect(cells[SB_CSV_COLUMNS.findIndex((c) => c.header === "std")]).toBe("");
    expect(cells[SB_CSV_COLUMNS.findIndex((c) => c.header === "mu_err")]).toBe("");
    expect(cells[SB_CSV_COLUMNS.findIndex((c) => c.header === "mu_ab")]).toBe("20.05");
  });

  const CONTEXT = { fileName: "brick-r.fits", appVersion: "v0.8.0", exportedUtc: "2026-11-12T18:00:00Z" };

  function nanomaggyCalibration(bunit: string): PhotCal {
    return {
      convention: { kind: "generic_zero_point", zp: 22.5, keyword: "MAGZERO", per_second: false, exptime: null },
      bunit,
      notes: [],
      warnings: [],
      label: "nanomaggy",
    };
  }

  it("the profile CSV with provenance keeps the bin table and says how it was measured", () => {
    const p = profile({ background: { median: 0.01, sigma: 0.002, count: 300 }, r50_px: 4.2 });
    const entries: ProvenanceEntry[] = [
      ["software", "AstroBurst v0.8.0"],
      ["exported_utc", "2026-11-12T18:00:00Z"],
      ["file", "brick-r.fits"],
      ["profile", "surface brightness in elliptical annuli"],
      ["centre_x_px", 50],
      ["centre_y_px", 50],
      ["sma_max_px", 3],
      ["ellipticity", 0.4],
      ["angle_deg_image", 30],
      ["sky_pa_deg", 120],
      ["bin_width_px", 1],
      ["background_median", 0.01],
      ["background_sigma", 0.002],
      ["background_count", 300],
      ["background", "subtracted from mean, median and the sums"],
      ["calibration", "none"],
      ["pixel_scale_arcsec", 0.05],
      ["pixel_area_arcsec2", 0.0025],
      ["r50_px", 4.2],
      ["r80_px", 1.5],
      ["r90_px", 2.0],
      ["petrosian_radius_px", 2.4],
      ["total_mag_ab", 14.5],
      ["dq_excluded_px", null],
      ["region_excluded_px", 0],
      ["positions", "0-based pixels, integer pixel centres"],
    ];
    const units = {
      sma_px: "pixel",
      sma_arcsec: "arcsec",
      sma_inner: "pixel",
      sma_outer: "pixel",
      count: "pixel",
      mean: "image unit",
      median: "image unit",
      std: "image unit",
      cumulative_sum: "image unit x pix",
      mu_ab: "mag/arcsec^2",
      mu_err: "mag",
      mag_ab_cumulative: "AB mag",
    };
    expect(entries).toHaveLength(26);
    const text = sbProfileCsvWithProvenance(p, CONTEXT);
    expect(text.endsWith(sbProfileCsv(p))).toBe(true);
    expect(text).toBe(withProvenance([...provenanceLines(entries), ...unitLines(units)], sbProfileCsv(p), CSV_LINE_END));
    const lines = text.split(CSV_LINE_END);
    expect(lines.slice(0, 3)).toEqual(["# software: AstroBurst v0.8.0", "# exported_utc: 2026-11-12T18:00:00Z", "# file: brick-r.fits"]);
    expect(lines).toContain("# profile: surface brightness in elliptical annuli");
    expect(lines).toContain("# background: subtracted from mean, median and the sums");
    expect(lines).toContain("# unit mu_ab: mag/arcsec^2");
    expect(lines.filter((line) => line.startsWith("# ellipticity: "))).toHaveLength(1);
    expect(lines.filter((line) => line.startsWith("# r50_px: "))).toHaveLength(1);
  });

  it("without a background or a calibration the provenance says none", () => {
    const bare = sbProfileCsvWithProvenance(profile({ background: null, photcal: null }), CONTEXT).split(CSV_LINE_END);
    expect(bare).toContain("# background: none subtracted");
    expect(bare).toContain("# calibration: none");
    expect(bare).toContain("# unit mean: image unit");
    expect(bare).toContain("# unit cumulative_sum: image unit x pix");

    const nanomaggy = sbProfileCsvWithProvenance(profile({ photcal: nanomaggyCalibration("nanomaggy") }), CONTEXT).split(CSV_LINE_END);
    expect(nanomaggy).toContain("# calibration: nanomaggy");
    expect(nanomaggy).toContain("# unit mean: nanomaggy");
    expect(nanomaggy).toContain("# unit cumulative_sum: nanomaggy");

    const surface = sbProfileCsvWithProvenance(profile({ photcal: nanomaggyCalibration("MJy/sr") }), CONTEXT).split(CSV_LINE_END);
    expect(surface).toContain("# unit cumulative_sum: MJy/sr x pix");
  });
});

describe("formatting helpers", () => {
  it("formats radii in pixels with the arcsec equivalent when a scale is known", () => {
    expect(formatRadius(12.34, 0.031)).toBe('12.3 px (0.38")');
    expect(formatRadius(12.34, null)).toBe("12.3 px");
    expect(formatRadius(null, 0.031)).toBe("--");
    expect(formatRadius(Number.NaN, 0.031)).toBe("--");
  });

  it("formats the image and sky position angles together", () => {
    expect(formatPositionAngles(42, 118.3, 0.4)).toBe("PA 42.0 deg image, 118.3 deg E of N");
    expect(formatPositionAngles(42, null, 0.4)).toBe("PA 42.0 deg image");
  });

  it("shows no position angle for a circle or a round ellipse, which have no major axis", () => {
    expect(formatPositionAngles(0, 90, 0)).toBe("--");
    expect(formatPositionAngles(37, null, 0)).toBe("--");
    expect(formatPositionAngles(37, 127, Number.NaN)).toBe("--");
  });

  it("reads the central surface brightness from the first bin", () => {
    expect(firstSurfaceBrightness(profile())).toBe(20.05);
    expect(firstSurfaceBrightness(profile({ bins: [] }))).toBeNull();
  });
});

describe("profileFetchReducer", () => {
  const empty = { result: null, error: null };

  it("drops the previous region's profile when the request for the newly selected region fails", () => {
    const shown = profileFetchReducer(empty, { type: "success", result: profile({ r50_px: 12.3, total_mag_ab: 17.8 }) });
    const failed = profileFetchReducer(shown, { type: "failure", message: "ellipticity must be in [0, 0.95], got 0.96" });
    expect(failed.result).toBeNull();
    expect(failed.error).toBe("ellipticity must be in [0, 0.95], got 0.96");
  });

  it("replaces an earlier error with the new result on success", () => {
    const b = profile({ r50_px: 4 });
    const failed = { result: null, error: "background region is empty" };
    expect(profileFetchReducer(failed, { type: "success", result: b })).toEqual({ result: b, error: null });
  });

  it("clears both the result and the error on reset and keeps an already empty state", () => {
    const shown = profileFetchReducer(empty, { type: "success", result: profile() });
    expect(profileFetchReducer(shown, { type: "reset" })).toEqual(empty);
    expect(profileFetchReducer(empty, { type: "reset" })).toBe(empty);
  });
});

describe("parseBinWidth", () => {
  it("rejects a decimal comma instead of truncating it to whole pixels", () => {
    expect(parseBinWidth("1,5")).toBeNull();
    expect(parseBinWidth("2,5")).toBeNull();
    expect(parseBinWidth("1.5px")).toBeNull();
  });

  it("reads dot decimals inside the allowed range", () => {
    expect(parseBinWidth("1.5")).toBe(1.5);
    expect(parseBinWidth(" 2 ")).toBe(2);
    expect(parseBinWidth("0.5")).toBe(0.5);
    expect(parseBinWidth("64")).toBe(64);
    expect(parseBinWidth("0.4")).toBeNull();
    expect(parseBinWidth("65")).toBeNull();
    expect(parseBinWidth("")).toBeNull();
  });
});
