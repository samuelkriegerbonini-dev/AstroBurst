import { describe, it, expect } from "vitest";
import {
  centroidJumps,
  checkStarCurve,
  differentialMag,
  ensembleFlux,
  frameTimes,
  jdZero,
  lightCurve,
  lightCurveCsv,
  lightCurveCsvFileName,
  lightCurveExtras,
  resolveTimeAxis,
  seriesRms,
  LIGHT_CURVE_CSV_COLUMNS,
} from "../differentialPhotometry";
import type { StarPhotometry } from "../../services/analysis";
import type { TimeSeriesFrame, TimeSeriesResult } from "../../shared/types/analysis";

function star(x: number, y: number, flux: number, err: number, extra: Partial<StarPhotometry> = {}): StarPhotometry {
  return {
    x,
    y,
    peak: 100,
    net_flux: flux,
    flux_err: err,
    mag_inst: -2.5 * Math.log10(flux),
    snr: flux / err,
    fwhm: 3,
    aperture_radius: 5,
    aperture_pixels: 78,
    aperture_area: 78.5,
    bg_mean: 10,
    bg_sigma: 1,
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
    ...extra,
  };
}

function frame(index: number, targets: (StarPhotometry | null)[], extra: Partial<TimeSeriesFrame> = {}): TimeSeriesFrame {
  return {
    index,
    path: `/run/f${index}.fits`,
    file_name: `f${index}.fits`,
    jd_mid: 2460310.5 + index / 1440,
    time_source: "DATE-OBS + EXPTIME/2 (60 s)",
    exptime: 60,
    filter: "V",
    airmass: 1.2,
    offset: index === 0 ? null : { dx: 0, dy: 0, confidence: 20, registered: true },
    photcal_label: null,
    targets,
    errors: targets.map(() => null),
    skipped: null,
    ...extra,
  };
}

function result(frames: TimeSeriesFrame[]): TimeSeriesResult {
  return {
    reference_path: "/run/f0.fits",
    targets: [
      { x: 10, y: 10, label: "T", role: "target" },
      { x: 40, y: 40, label: "C1", role: "comp" },
      { x: 70, y: 70, label: "C2", role: "comp" },
    ],
    frames,
    n_frames: frames.length,
    n_skipped: frames.filter((f) => f.skipped !== null).length,
    warnings: [],
    elapsed_ms: 5,
  };
}

const T = star(10, 10, 500, 5);
const C1 = star(40, 40, 1000, 10);
const C2 = star(70, 70, 1000, 10);

describe("differentialMag and ensembleFlux", () => {
  it("gives 1.5051 mag for T = 500 against an ensemble of 2000 with the propagated error", () => {
    const ens = ensembleFlux(frame(0, [T, C1, C2]), [1, 2]);
    expect(ens).not.toBeNull();
    expect(ens!.flux).toBe(2000);
    expect(ens!.err).toBeCloseTo(Math.sqrt(200), 10);
    const diff = differentialMag(T, ens!);
    expect(diff!.mag).toBeCloseTo(1.5051, 4);
    const expectedErr = 1.0857 * Math.sqrt((5 / 500) ** 2 + (Math.sqrt(200) / 2000) ** 2);
    expect(diff!.err).toBeCloseTo(expectedErr, 10);
  });

  it("returns null for a missing or non-positive comparison and for a non-positive target", () => {
    expect(ensembleFlux(frame(0, [T, null, C2]), [1, 2])).toBeNull();
    expect(ensembleFlux(frame(0, [T, star(1, 1, -3, 1), C2]), [1, 2])).toBeNull();
    expect(ensembleFlux(frame(0, [T, C1, C2]), [])).toBeNull();
    expect(differentialMag({ net_flux: 0, flux_err: 1 }, { flux: 100, err: 1 })).toBeNull();
    expect(differentialMag({ net_flux: 10, flux_err: 1 }, { flux: NaN, err: 1 })).toBeNull();
  });
});

describe("lightCurve", () => {
  it("drops the point of a frame whose comparison is null and keeps the flux", () => {
    const res = result([frame(0, [T, C1, C2]), frame(1, [T, null, C2]), frame(2, [T, C1, C2])]);
    const curve = lightCurve(res, 0, [1, 2], "jd");
    expect(curve).toHaveLength(3);
    expect(curve[1].mag).toBeNull();
    expect(curve[1].err).toBeNull();
    expect(curve[1].flux).toBe(500);
    expect(curve[0].mag).toBeCloseTo(1.5051, 4);
    expect(curve[2].skipped).toBe(false);
  });

  it("uses the JD axis when every measured frame has one and the frame index otherwise", () => {
    const withJd = result([frame(0, [T, C1, C2]), frame(1, [T, C1, C2])]);
    expect(resolveTimeAxis(withJd, "jd")).toBe("jd");
    expect(lightCurve(withJd, 0, [1, 2], "jd").map((p) => p.t)).toEqual([2460310.5, 2460310.5 + 1 / 1440]);
    expect(lightCurve(withJd, 0, [1, 2], "index").map((p) => p.t)).toEqual([0, 1]);
    const missing = result([frame(0, [T, C1, C2]), frame(1, [T, C1, C2], { jd_mid: null, time_source: null })]);
    expect(resolveTimeAxis(missing, "jd")).toBe("index");
    expect(lightCurve(missing, 0, [1, 2], "jd").map((p) => p.t)).toEqual([0, 1]);
    const skippedOnly = result([frame(0, [T, C1, C2]), frame(1, [null, null, null], { jd_mid: null, skipped: "dimensions differ" })]);
    expect(resolveTimeAxis(skippedOnly, "jd")).toBe("jd");
    const skippedPoint = lightCurve(skippedOnly, 0, [1, 2], "jd")[1];
    expect(skippedPoint).toMatchObject({ mag: null, skipped: true });
    expect(Number.isNaN(skippedPoint.t)).toBe(true);
    expect(jdZero(lightCurve(withJd, 0, [1, 2], "jd"))).toBe(2460310);
    expect(jdZero(lightCurve(skippedOnly, 0, [1, 2], "jd"))).toBe(2460310);
    expect(frameTimes(skippedOnly, "jd").map((p) => p.skipped)).toEqual([false, true]);
    expect(frameTimes(missing, "jd").map((p) => p.t)).toEqual([0, 1]);
  });

  it("marks skipped frames and never uses the target itself as a comparison", () => {
    const res = result([frame(0, [T, C1, C2]), frame(1, [null, null, null], { skipped: "dimensions 64 x 64 differ" })]);
    const curve = lightCurve(res, 0, [0, 1, 2], "index");
    expect(curve[0].mag).toBeCloseTo(1.5051, 4);
    expect(curve[1]).toMatchObject({ frameIndex: 1, mag: null, flux: null, skipped: true });
  });
});

describe("checkStarCurve and seriesRms", () => {
  it("has zero rms for a check star of two identical comparisons", () => {
    const res = result([frame(0, [T, C1, C2]), frame(1, [T, C1, C2]), frame(2, [T, C1, C2])]);
    const check = checkStarCurve(res, 1, [1, 2]);
    expect(check.map((p) => Math.abs(p.mag ?? 1))).toEqual([0, 0, 0]);
    expect(seriesRms(check)).toBe(0);
  });

  it("computes the population standard deviation of the finite magnitudes", () => {
    const points = [1, 2, 3, null, NaN].map((mag, i) => ({ frameIndex: i, t: i, mag, err: null, flux: null, skipped: false }));
    expect(seriesRms(points)).toBeCloseTo(Math.sqrt(2 / 3), 12);
    expect(seriesRms(points.slice(0, 1))).toBeNull();
    expect(seriesRms([])).toBeNull();
  });
});

describe("centroidJumps", () => {
  it("flags a 3 px jump and not a 1 px one after removing the registered offset", () => {
    const res = result([
      frame(0, [T, C1, C2]),
      frame(1, [star(11, 10, 500, 5), C1, C2]),
      frame(2, [star(11, 13, 500, 5), C1, C2]),
      frame(3, [star(17, 13, 500, 5), star(46, 40, 1000, 10), star(76, 70, 1000, 10)], { offset: { dx: 6, dy: 0, confidence: 30, registered: true } }),
      frame(4, [star(17, 13, 500, 5), star(46, 40, 1000, 10), star(76, 70, 1000, 10)], { offset: { dx: 6, dy: 0, confidence: 2, registered: false } }),
    ]);
    expect(centroidJumps(res)).toEqual([2, 4]);
    expect(centroidJumps(res, 10)).toEqual([]);
  });

  it("skips frames without a measurement and skipped frames", () => {
    const res = result([
      frame(0, [T, C1, C2]),
      frame(1, [null, null, null], { skipped: "failed to load" }),
      frame(2, [null, C1, C2]),
      frame(3, [star(10.5, 10, 500, 5), C1, C2]),
    ]);
    expect(centroidJumps(res)).toEqual([]);
  });
});

describe("lightCurveCsv", () => {
  it("writes the documented column order and blanks for missing values", () => {
    const res = result([
      frame(0, [T, C1, C2]),
      frame(1, [null, null, null], { jd_mid: null, time_source: null, airmass: null, offset: null, skipped: "dimensions 64 x 64 differ from the reference 128 x 128" }),
      frame(2, [star(10, 10, 500, 5, { saturated: true }), C1, C2], { filter: "R, wide", offset: { dx: 1.5, dy: -0.25, confidence: 12, registered: true } }),
    ]);
    const rows = lightCurve(res, 0, [1, 2], "jd");
    const csv = lightCurveCsv(res, rows, lightCurveExtras(res, 0, [1, 2]));
    const lines = csv.trimEnd().split("\n");
    expect(lines[0]).toBe(LIGHT_CURVE_CSV_COLUMNS.join(","));
    expect(lines[0]).toBe(
      "index,file,jd_mid,time_source,exptime,filter,airmass,diff_mag,diff_err,target_flux,target_err,target_snr,ensemble_flux,fwhm,bg_mean,dx,dy,registered,saturated,skipped",
    );
    expect(lines).toHaveLength(4);
    const first = lines[1].split(",");
    expect(first[0]).toBe("0");
    expect(first[1]).toBe("f0.fits");
    expect(first[2]).toBe("2460310.500000");
    expect(first[7]).toBe("1.50515");
    expect(first[9]).toBe("500.000");
    expect(first[12]).toBe("2000.000");
    expect(first[15]).toBe("");
    expect(first[17]).toBe("");
    expect(first[18]).toBe("false");
    expect(lines[2]).toBe("1,f1.fits,,,60,V,,,,,,,,,,,,,,dimensions 64 x 64 differ from the reference 128 x 128");
    const third = lines[3];
    expect(third).toContain('"R, wide"');
    expect(third.endsWith(",1.500,-0.250,true,true,")).toBe(true);
  });

  it("names the file after the reference frame", () => {
    expect(lightCurveCsvFileName("/data/run/frame_0000.fits")).toBe("frame_0000_lightcurve.csv");
    expect(lightCurveCsvFileName("C:\\obs\\stack.fits#hdu=1")).toBe("stack_lightcurve.csv");
  });
});
