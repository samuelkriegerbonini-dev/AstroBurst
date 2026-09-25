import { describe, it, expect } from "vitest";
import {
  centroidJumps,
  checkReference,
  checkRmsLabel,
  checkStarCurve,
  differentialMag,
  ensembleFlux,
  frameFilesNotice,
  frameJdLabel,
  frameMeasurementErrors,
  frameTimes,
  inFrameOrder,
  jdOffsetLabel,
  jdZero,
  lightCurve,
  lightCurveCsv,
  lightCurveCsvFileName,
  lightCurveExtras,
  measuredTargets,
  referenceTimeSource,
  resolveTimeAxis,
  resultCoversPath,
  seriesRms,
  timeAxisLabel,
  timeSeriesRoleHint,
  LIGHT_CURVE_CSV_COLUMNS,
  MAX_TIME_SERIES_TARGETS,
  NO_COMP_HINT,
  NO_TARGET_HINT,
} from "../differentialPhotometry";
import { niceTicks } from "../plotScale";
import type { StarPhotometry } from "../../services/analysis";
import type { TimeSeriesFrame, TimeSeriesResult, TimeSeriesTarget } from "../../shared/types/analysis";

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
  it("flags frames 3 px from the reference centroid after removing the registered offset, and not 1 px ones", () => {
    const res = result([
      frame(0, [T, C1, C2]),
      frame(1, [star(11, 10, 500, 5), C1, C2]),
      frame(2, [star(11, 13, 500, 5), C1, C2]),
      frame(3, [star(17, 13, 500, 5), star(46, 40, 1000, 10), star(76, 70, 1000, 10)], { offset: { dx: 6, dy: 0, confidence: 30, registered: true } }),
      frame(4, [star(17, 13, 500, 5), star(46, 40, 1000, 10), star(76, 70, 1000, 10)], { offset: { dx: 6, dy: 0, confidence: 2, registered: false } }),
    ]);
    expect(centroidJumps(res)).toEqual([2, 3, 4]);
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
      "index,file,jd_mid,time_source,exptime,filter,airmass,diff_mag,diff_err,target_flux,target_err,target_snr,ensemble_flux,fwhm,bg_mean,dx,dy,registered,saturated,centroid_jump,skipped,errors",
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
    expect(first[19]).toBe("false");
    expect(lines[2]).toBe("1,f1.fits,,,60,V,,,,,,,,,,,,,,,dimensions 64 x 64 differ from the reference 128 x 128,");
    const third = lines[3];
    expect(third).toContain('"R, wide"');
    expect(third.endsWith(",1.500,-0.250,true,true,false,,")).toBe(true);
  });

  it("names the failed comparison star of a frame whose differential magnitude is missing", () => {
    const failure = "C2 shifted to (131.0, 70.0) falls outside the 128 x 128 frame";
    const res = result([frame(0, [T, C1, C2]), frame(1, [T, C1, null], { errors: [null, null, failure] })]);
    const rows = lightCurve(res, 0, [1, 2], "jd");
    expect(rows[1].mag).toBeNull();
    expect(frameMeasurementErrors(res.frames[1], res.targets, 0, [1, 2])).toEqual([`C2: ${failure}`]);
    expect(frameMeasurementErrors(res.frames[0], res.targets, 0, [1, 2])).toEqual([]);
    const lines = lightCurveCsv(res, rows, lightCurveExtras(res, 0, [1, 2])).trimEnd().split("\n");
    const col = lines[0].split(",").indexOf("errors");
    expect(lines[1].split(",")[col]).toBe("");
    expect(lines[2].endsWith(`,"C2: ${failure}"`)).toBe(true);
  });

  it("reports no measurement errors for a skipped frame, whose reason is already in the skipped column", () => {
    const skipped = frame(1, [null, null, null], { skipped: "failed to load", errors: ["x", "y", "z"] });
    expect(frameMeasurementErrors(skipped, result([skipped]).targets, 0, [1, 2])).toEqual([]);
  });

  it("names the file after the reference frame", () => {
    expect(lightCurveCsvFileName("/data/run/frame_0000.fits")).toBe("frame_0000_lightcurve.csv");
    expect(lightCurveCsvFileName("C:\\obs\\stack.fits#hdu=1")).toBe("stack_lightcurve.csv");
  });
});

describe("centroidJumps against the reference-frame centroid", () => {
  const moved = star(15, 10, 250, 5);

  it("flags only the captured frame after a one-frame capture", () => {
    const res = result([0, 1, 2, 3, 4].map((i) => frame(i, [i === 2 ? moved : T, C1, C2])));
    expect(centroidJumps(res)).toEqual([2]);
  });

  it("flags every frame of a capture that lasts several frames and not the frame after it", () => {
    const res = result([0, 1, 2, 3, 4, 5, 6, 7].map((i) => frame(i, [i >= 2 && i <= 5 ? moved : T, C1, C2])));
    expect(centroidJumps(res)).toEqual([2, 3, 4, 5]);
  });

  it("writes the flag to the centroid_jump CSV column", () => {
    const res = result([0, 1, 2, 3].map((i) => frame(i, [i === 2 ? moved : T, C1, C2])));
    const csv = lightCurveCsv(res, lightCurve(res, 0, [1, 2], "jd"), lightCurveExtras(res, 0, [1, 2]));
    const lines = csv.trimEnd().split("\n");
    const col = lines[0].split(",").indexOf("centroid_jump");
    expect(col).toBeGreaterThan(-1);
    expect(lines.slice(1).map((l) => l.split(",")[col])).toEqual(["false", "false", "true", "false"]);
  });

  it("measures frames with unknown drift against the common motion of the other stars, so a slow drift or a mount bump is not flagged", () => {
    const res = result([0, 1, 2, 3, 4, 5, 6, 7].map((i) => frame(i, [star(10 + 0.5 * i, 10, 500, 5), star(40 + 0.5 * i, 40, 1000, 10), star(70 + 0.5 * i, 70, 1000, 10)], { offset: null })));
    expect(centroidJumps(res)).toEqual([]);
    const bump = (i: number) => (i >= 3 ? 5 : 0);
    const bumped = result([0, 1, 2, 3, 4, 5].map((i) => frame(i, [star(10 + bump(i), 10, 500, 5), star(40 + bump(i), 40, 1000, 10), star(70 + bump(i), 70, 1000, 10)], { offset: null })));
    expect(centroidJumps(bumped)).toEqual([]);
  });
});

describe("centroidJumps on frames whose drift is unknown", () => {
  const moved = star(15, 10, 250, 5);
  const untracked = { offset: null };
  const rejected = { offset: { dx: 0, dy: 0, confidence: 2, registered: false } };

  it("flags only the captured frame after a one-frame capture when drift is not tracked or its offset was rejected", () => {
    for (const drift of [untracked, rejected]) {
      const res = result([0, 1, 2, 3, 4].map((i) => frame(i, [i === 2 ? moved : T, C1, C2], i === 0 ? {} : drift)));
      expect(centroidJumps(res)).toEqual([2]);
    }
  });

  it("flags every frame of a multi-frame capture and not the frame after it when drift is not tracked or its offset was rejected", () => {
    for (const drift of [untracked, rejected]) {
      const res = result([0, 1, 2, 3, 4, 5, 6, 7].map((i) => frame(i, [i >= 2 && i <= 5 ? moved : T, C1, C2], i === 0 ? {} : drift)));
      expect(centroidJumps(res)).toEqual([2, 3, 4, 5]);
    }
  });

  it("finds a capture riding on an untracked drift of 4 px per frame", () => {
    const shift = (i: number) => 4 * i;
    const res = result(
      [0, 1, 2, 3, 4].map((i) =>
        frame(i, [star(10 + shift(i) + (i === 3 ? 5 : 0), 10, 500, 5), star(40 + shift(i), 40, 1000, 10), star(70 + shift(i), 70, 1000, 10)], untracked),
      ),
    );
    expect(centroidJumps(res)).toEqual([3]);
  });

  it("compares a target with its single comparison star at the full threshold and cannot check a lone star", () => {
    const pair = (frames: TimeSeriesFrame[]) => ({ ...result(frames), targets: result(frames).targets.slice(0, 2) });
    const nudged = star(13, 10, 500, 5);
    expect(centroidJumps(pair([0, 1, 2, 3].map((i) => frame(i, [i === 2 ? nudged : T, C1], untracked))))).toEqual([2]);
    expect(centroidJumps(pair([0, 1, 2, 3].map((i) => frame(i, [i === 2 ? star(11.5, 10, 500, 5) : T, C1], untracked))))).toEqual([]);
    expect(centroidJumps(pair([0, 1, 2, 3].map((i) => frame(i, [i === 2 ? moved : T, null], untracked))))).toEqual([]);
  });

  it("writes the untracked capture flag to the centroid_jump CSV column", () => {
    const res = result([0, 1, 2, 3].map((i) => frame(i, [i === 2 ? moved : T, C1, C2], i === 0 ? {} : untracked)));
    const csv = lightCurveCsv(res, lightCurve(res, 0, [1, 2], "jd"), lightCurveExtras(res, 0, [1, 2]));
    const lines = csv.trimEnd().split("\n");
    const col = lines[0].split(",").indexOf("centroid_jump");
    expect(lines.slice(1).map((l) => l.split(",")[col])).toEqual(["false", "false", "true", "false"]);
  });
});

describe("checkReference", () => {
  it("uses the designated check star over the first comparison when both exist", () => {
    const frames = [1, 1.07, 0.93, 1.05].map((f, i) => frame(i, [T, C1, C2, star(90, 90, 800 * f, 8)]));
    const base = result(frames);
    const res = { ...base, targets: [...base.targets, { x: 90, y: 90, label: "K", role: "check" as const }] };
    const ref = checkReference(res, [1, 2], [3], "index");
    expect(ref).toMatchObject({ index: 3, kind: "check" });
    expect(seriesRms(ref!.curve)).toBeGreaterThan(0.02);
    expect(checkRmsLabel(ref, res.targets)).toBe("Check-star rms (K)");
  });

  it("falls back to the first comparison against the others only when no check star is marked", () => {
    const res = result([frame(0, [T, C1, C2]), frame(1, [T, C1, C2])]);
    const ref = checkReference(res, [1, 2], [], "index");
    expect(ref).toMatchObject({ index: 1, kind: "comp" });
    expect(checkRmsLabel(ref, res.targets)).toBe("C1 vs other comps rms");
    expect(checkReference(res, [1], [], "index")).toBeNull();
    expect(checkRmsLabel(null, res.targets)).toBe("Check-star rms");
  });
});

describe("resultCoversPath", () => {
  const measured = result([frame(0, [T, C1, C2]), frame(1, [T, C1, C2]), frame(2, [T, C1, C2], { skipped: "dimension mismatch" })]);

  it("keeps the light curve when the viewer moves to any measured frame", () => {
    expect(resultCoversPath(measured, "/run/f0.fits")).toBe(true);
    expect(resultCoversPath(measured, "/run/f1.fits")).toBe(true);
    expect(resultCoversPath(measured, "/run/f2.fits")).toBe(true);
  });

  it("drops it for a file outside the series, a null path or no result", () => {
    expect(resultCoversPath(measured, "/other/f1.fits")).toBe(false);
    expect(resultCoversPath(measured, null)).toBe(false);
    expect(resultCoversPath(null, "/run/f0.fits")).toBe(false);
  });
});

describe("measuredTargets", () => {
  it("drops ignored stars so 200 points with 150 ignored fit under the 64-target limit", () => {
    const rows: TimeSeriesTarget[] = Array.from({ length: 200 }, (_, i) => ({
      x: i,
      y: i,
      label: `P${i + 1}`,
      role: i === 0 ? "target" : i < 50 ? "comp" : "ignore",
    }));
    const sent = measuredTargets(rows);
    expect(sent).toHaveLength(50);
    expect(sent.some((t) => t.role === "ignore")).toBe(false);
    expect(sent[0]).toEqual(rows[0]);
    expect(sent.length).toBeLessThanOrEqual(MAX_TIME_SERIES_TARGETS);
  });

  it("mirrors the backend limit of 64 targets", () => {
    expect(MAX_TIME_SERIES_TARGETS).toBe(64);
  });
});

describe("frameJdLabel", () => {
  it("prints the full JD without an exponent and resolves a 60 s cadence in frame-index mode", () => {
    const start = 2460310.5 + 30 / 86400;
    const labels = [0, 1, 2].map((k) => frameJdLabel(start + (k * 60) / 86400, "index", 0));
    expect(labels[0]).toBe("2460310.50035");
    expect(labels.every((l) => !/e\+/.test(l))).toBe(true);
    expect(new Set(labels).size).toBe(3);
  });

  it("prints JD - JD0 in jd mode and a dash when the frame has no time", () => {
    expect(frameJdLabel(2460310.5003472, "jd", 2460310)).toBe("0.5003");
    expect(frameJdLabel(null, "index", 0)).toBe("--");
  });
});

describe("jdOffsetLabel", () => {
  it("keeps every tick label distinct for a 30 frame run at 2 s cadence", () => {
    const xs = Array.from({ length: 30 }, (_, i) => 0.3137 + (i * 2 + 1) / 86400);
    const ticks = niceTicks(Math.min(...xs), Math.max(...xs), 6);
    const step = ticks[1] - ticks[0];
    const labels = ticks.map((t) => jdOffsetLabel(t, step));
    expect(step).toBeLessThan(1e-3);
    expect(new Set(labels).size).toBe(labels.length);
  });

  it("keeps half-milliday ticks of a zoomed 60 s run distinct and leaves coarse steps at 3 decimals", () => {
    expect([0.503, 0.5035, 0.504].map((t) => jdOffsetLabel(t, 0.0005))).toEqual(["0.5030", "0.5035", "0.5040"]);
    expect(jdOffsetLabel(0.502, 0.001)).toBe("0.502");
    expect(jdOffsetLabel(0.52, 0.02)).toBe("0.520");
  });

  it("reads out adjacent 60 s frames as different offsets when no tick step is given", () => {
    expect(jdOffsetLabel(0.5 + 150 / 86400, Number.NaN)).toBe("0.501736");
    expect(jdOffsetLabel(0.5 + 210 / 86400, Number.NaN)).toBe("0.502431");
    expect(jdOffsetLabel(0.5 + 150 / 86400)).toBe("0.501736");
    expect(jdOffsetLabel(0.5 + 150 / 86400, 0)).toBe("0.501736");
  });
});

describe("inFrameOrder", () => {
  it("puts a reference-first run back in store order, renumbers the frames and raises no jump for a steady drift", () => {
    const drifting = (k: number) =>
      frame(k, [star(10 + 0.5 * k, 10, 500, 5), star(40 + 0.5 * k, 40, 1000, 10), star(70 + 0.5 * k, 70, 1000, 10)], { offset: null });
    const storeOrder = Array.from({ length: 10 }, (_, k) => k);
    const sent = [6, 0, 1, 2, 3, 4, 5, 7, 8, 9];
    const raw = { ...result(sent.map((k, position) => ({ ...drifting(k), index: position }))), reference_path: "/run/f6.fits" };
    expect(centroidJumps(raw)).toEqual([]);
    const ordered = inFrameOrder(raw, storeOrder.map((k) => `/run/f${k}.fits`));
    expect(ordered.frames.map((f) => f.file_name)).toEqual(storeOrder.map((k) => `f${k}.fits`));
    expect(ordered.frames.map((f) => f.index)).toEqual(storeOrder);
    expect(ordered.reference_path).toBe("/run/f6.fits");
    expect(centroidJumps(ordered)).toEqual([]);
    const curve = lightCurve(ordered, 0, [1, 2], "index");
    expect(curve.map((p) => p.t)).toEqual(storeOrder);
    const csvFiles = lightCurveCsv(ordered, curve, lightCurveExtras(ordered, 0, [1, 2]))
      .trimEnd()
      .split("\n")
      .slice(1)
      .map((line) => line.split(",")[1]);
    expect(csvFiles).toEqual(storeOrder.map((k) => `f${k}.fits`));
  });
});

describe("timeSeriesRoleHint", () => {
  it("blocks a run that has a target but no comparison star", () => {
    expect(timeSeriesRoleHint(["target"])).toBe(NO_COMP_HINT);
    expect(timeSeriesRoleHint(["target", "check", "ignore"])).toBe(NO_COMP_HINT);
  });

  it("asks for the target before the comparison stars", () => {
    expect(timeSeriesRoleHint(["comp", "comp"])).toBe(NO_TARGET_HINT);
    expect(timeSeriesRoleHint([])).toBe(NO_TARGET_HINT);
  });

  it("allows a target with at least one comparison star", () => {
    expect(timeSeriesRoleHint(["target", "comp"])).toBeNull();
    expect(timeSeriesRoleHint(["comp", "target", "check"])).toBeNull();
  });

  it("a target-only run yields no differential magnitude in any frame", () => {
    const frames = [0, 1, 2].map((i) => frame(i, [star(10, 10, 5000, 50)]));
    const r = { ...result(frames), targets: [{ x: 10, y: 10, label: "T", role: "target" as const }] };
    const curve = lightCurve(r, 0, [], "index");
    expect(curve.every((p) => p.mag === null)).toBe(true);
    expect(seriesRms(curve)).toBeNull();
  });
});

describe("frameFilesNotice", () => {
  it("says the loaded frame files are measured and names the processed result on screen that is not used", () => {
    const notice = frameFilesNotice({ compositeOnScreen: false, processedLabel: "Calibrated light_001" });
    expect(notice).toContain("loaded frame files");
    expect(notice).toContain("Calibrated light_001");
    expect(notice).toContain("not used");
  });

  it("says the RGB view on screen is not used when a composite is shown", () => {
    expect(frameFilesNotice({ compositeOnScreen: true, processedLabel: null })).toContain("not on the RGB view on screen");
  });

  it("is null when the loaded file itself is on screen", () => {
    expect(frameFilesNotice({ compositeOnScreen: false, processedLabel: null })).toBeNull();
  });
});

describe("time scale labels", () => {
  it("qualifies the JD axis as header time without asserting UTC, geocentric or barycentric", () => {
    const label = timeAxisLabel("jd", 2460310);
    expect(label).toContain("header time");
    expect(label).toContain("not barycentric");
    expect(label).toContain("2460310");
    expect(label).not.toMatch(/UTC|geocentric/);
    expect(timeAxisLabel("index", 0)).toBe("frame index");
  });

  it("reads the time source of the reference frame even after the frames are put back in store order", () => {
    const frames = [0, 1, 2].map((i) => frame(i, [T, C1, C2], { time_source: `DATE-OBS; TIMESYS=TT frame ${i}` }));
    const res = { ...result(frames), reference_path: "/run/f2.fits" };
    expect(referenceTimeSource(res)).toBe("DATE-OBS; TIMESYS=TT frame 2");
    expect(referenceTimeSource({ ...res, reference_path: "/run/missing.fits" })).toBeNull();
  });
});
