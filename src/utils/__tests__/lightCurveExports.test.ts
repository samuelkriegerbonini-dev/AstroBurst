import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  defaultPointLabel,
  etdBlocker,
  etdFileName,
  etdText,
  exportBlockerNotes,
  exoClockBlocker,
  exoClockCsv,
  exoClockFileName,
  formatPrintfF,
  formatPrintfG,
  isDefaultPointLabel,
  magToRelativeFlux,
  withCurrentLabels,
  AREA_MIXED_BLOCKER,
  NO_ROWS_BLOCKER,
  NO_TARGET_BLOCKER,
  type LightCurveExportSource,
} from "../lightCurveExports";
import { lightCurve, MAG_ERR_FACTOR } from "../differentialPhotometry";
import type { StarPhotometry } from "../../services/analysis";
import type { TimeSeriesFrame, TimeSeriesResult, TimeSeriesTarget } from "../../shared/types/analysis";
import type { FrameGeometry } from "../../shared/types/geometry";

function star(flux: number, err: number, extra: Partial<StarPhotometry> = {}): StarPhotometry {
  return {
    x: 10,
    y: 10,
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

function geometry(jdUtc: number, extra: Partial<FrameGeometry> = {}): FrameGeometry {
  return {
    jd_utc: jdUtc,
    jd_tt: jdUtc + 69.184 / 86400,
    jd_tdb: jdUtc + 69.184 / 86400,
    bjd_tdb: jdUtc + 0.0041234,
    hjd_utc: jdUtc + 0.0033211,
    bjd_source: "computed",
    lst_deg: 100,
    hour_angle_deg: 10,
    altitude_deg: 60,
    azimuth_deg: 200,
    airmass_computed: 1.15,
    airmass_formula: "Kasten & Young 1989",
    parallactic_angle_deg: 0,
    sun_altitude_deg: -30,
    moon_altitude_deg: 5,
    moon_illumination: 0.5,
    moon_separation_deg: 80,
    time_scale_notes: [],
    ...extra,
  };
}

function frame(index: number, jdUtc: number, targetFlux: number, extra: Partial<TimeSeriesFrame> = {}): TimeSeriesFrame {
  return {
    index,
    path: `/run/f${index}.fits`,
    file_name: `f${index}.fits`,
    jd_mid: jdUtc,
    time_source: "DATE-OBS + EXPTIME/2 (60 s); UTC assumed (no TIMESYS)",
    exptime: 60,
    filter: "V",
    airmass: 1.2,
    geometry: geometry(jdUtc),
    offset: index === 0 ? null : { dx: 0.5, dy: -0.25, confidence: 20, registered: true },
    photcal_label: null,
    targets: [star(targetFlux, 5), star(1000, 10), star(1000, 10)],
    errors: [null, null, null],
    skipped: null,
    ...extra,
  };
}

const TARGETS: TimeSeriesTarget[] = [
  { x: 10, y: 10, label: "HAT-P-32", role: "target" },
  { x: 40, y: 40, label: "C1", role: "comp" },
  { x: 70, y: 70, label: "C2", role: "comp" },
];

function result(frames: TimeSeriesFrame[], targets: TimeSeriesTarget[] = TARGETS): TimeSeriesResult {
  return {
    reference_path: "/run/f0.fits",
    targets,
    frames,
    n_frames: frames.length,
    n_skipped: frames.filter((f) => f.skipped !== null).length,
    warnings: [],
    area_mixed: false,
    geometry_target: null,
    geometry_notes: [],
    elapsed_ms: 5,
  };
}

function goldenFrames(): TimeSeriesFrame[] {
  return [
    frame(0, 2460310.6000004, 500),
    frame(1, 2460310.6020838, 480),
    frame(2, 2460310.6013893, 470, { skipped: "dimensions 10 x 10 differ from the reference 20 x 20" }),
    frame(3, 2460310.6006949, 490, { airmass: null }),
  ];
}

function source(frames: TimeSeriesFrame[] = goldenFrames(), targets: TimeSeriesTarget[] = TARGETS): LightCurveExportSource {
  return { result: result(frames, targets), targetIdx: targets.findIndex((t) => t.role === "target"), compIdx: [1, 2] };
}

function withFrame(index: number, patch: Partial<TimeSeriesFrame>): TimeSeriesFrame[] {
  return goldenFrames().map((f) => (f.index === index ? { ...f, ...patch } : f));
}

const ETD_GOLDEN = [
  "#JD_UT (+ 2460310)",
  "# JD_UT V-C err",
  "2460310.600000 1.50515 0.0132971",
  "2460310.600695 1.52708 0.0134786",
  "2460310.602084 1.54947 0.0136689",
].join("\n");

const EXPOSURE_START_SOURCE = "DATE-OBS (EXPTIME missing: exposure start used); UTC assumed (no TIMESYS)";

const EXOCLOCK_PROVENANCE = [
  "# AstroBurst differential light curve for ExoClock",
  "# target: HAT-P-32",
  "# comparison stars: C1, C2",
  "# reference frame: f0.fits",
  "# flux: target / sum of comparison fluxes, not normalised; rel_flux_err = rel_flux * diff_err / 1.0857",
  "# filter: V",
  "# exposure s: 60",
  "# aperture px: 5; sky annulus px: 10 to 15",
  "# rows: 3 of 4 frames (1 omitted: skipped or no differential magnitude)",
];

const EXOCLOCK_JD_UTC_GOLDEN = [
  "# AstroBurst differential light curve for ExoClock",
  "# target: HAT-P-32",
  "# comparison stars: C1, C2",
  "# reference frame: f0.fits",
  "# time: jd_utc = JD_UTC at mid-exposure",
  ...EXOCLOCK_PROVENANCE.slice(4),
  "jd_utc,rel_flux,rel_flux_err,airmass,fwhm_px,sky,dx_px,dy_px",
  "2460310.60000040,0.25,0.003061862178,1.2000,3.000,10.000,0.000,0.000",
  "2460310.60069490,0.245,0.003041586757,1.1500,3.000,10.000,0.500,-0.250",
  "2460310.60208380,0.24,0.003021588986,1.2000,3.000,10.000,0.500,-0.250",
  "",
].join("\n");

describe("printf formatting as Siril writes the ETD file", () => {
  it("prints %f with six decimals", () => {
    expect(formatPrintfF(2460310.6000004)).toBe("2460310.600000");
    expect(formatPrintfF(2460310.6006949)).toBe("2460310.600695");
  });

  it("prints %g with six significant digits, trailing zeros stripped, C exponents", () => {
    expect(formatPrintfG(1.505149978319906)).toBe("1.50515");
    expect(formatPrintfG(0.013297055068698485)).toBe("0.0132971");
    expect(formatPrintfG(0.0000123456)).toBe("1.23456e-05");
    expect(formatPrintfG(-0.00012345)).toBe("-0.00012345");
    expect(formatPrintfG(123456.7)).toBe("123457");
    expect(formatPrintfG(1234567)).toBe("1.23457e+06");
    expect(formatPrintfG(0.1)).toBe("0.1");
    expect(formatPrintfG(100000)).toBe("100000");
    expect(formatPrintfG(999999.5)).toBe("1e+06");
    expect(formatPrintfG(0)).toBe("0");
    expect(formatPrintfG(-0)).toBe("-0");
  });
});

describe("ETD export (C02)", () => {
  it("writes Siril's three-column layout: title line, column line, %f %g %g rows sorted by time, no trailing newline", () => {
    expect(etdBlocker(source())).toBeNull();
    expect(etdText(source())).toBe(ETD_GOLDEN);
  });

  it("takes JD_UT from the UTC geometry time, not from a TT header time", () => {
    const tt = goldenFrames().map((f) => ({ ...f, jd_mid: f.geometry!.jd_tt, time_source: "DATE-OBS; time scale TT (TIMESYS), not converted to UTC" }));
    expect(etdText(source(tt))).toBe(ETD_GOLDEN);
  });

  it("names the file after the target and the reference frame", () => {
    expect(etdFileName(source())).toBe("HAT-P-32_f0_etd.txt");
  });
});

describe("ExoClock export (C02)", () => {
  it("writes the provenance block, the jd_utc header and one row per measured frame with 8-decimal times", () => {
    expect(exoClockBlocker(source(), "jd_utc")).toBeNull();
    expect(exoClockCsv(source(), "jd_utc")).toBe(EXOCLOCK_JD_UTC_GOLDEN);
  });

  it("names the BJD_TDB column and writes the barycentric times when asked", () => {
    const lines = exoClockCsv(source(), "bjd_tdb").split("\n");
    expect(lines).toContain("# time: bjd_tdb = BJD_TDB at mid-exposure");
    const header = lines.findIndex((l) => l.startsWith("bjd_tdb,"));
    expect(lines[header]).toBe("bjd_tdb,rel_flux,rel_flux_err,airmass,fwhm_px,sky,dx_px,dy_px");
    expect(lines.slice(header + 1, header + 4).map((l) => l.split(",")[0])).toEqual(["2460310.60412380", "2460310.60481830", "2460310.60620720"]);
  });

  it("names the HJD_UTC column and writes the heliocentric times when asked", () => {
    const lines = exoClockCsv(source(), "hjd_utc").split("\n");
    const header = lines.findIndex((l) => l.startsWith("hjd_utc,"));
    expect(header).toBe(EXOCLOCK_PROVENANCE.length + 1);
    expect(lines[header + 1].split(",")[0]).toBe("2460310.60332150");
  });

  it("names the file after the target, the reference frame and the time column", () => {
    expect(exoClockFileName(source(), "bjd_tdb")).toBe("HAT-P-32_f0_exoclock_bjd_tdb.csv");
  });

  it("leaves dx_px and dy_px empty for a rejected drift offset, since that frame was measured unshifted", () => {
    const frames = withFrame(1, { offset: { dx: 37.2, dy: -51.9, confidence: 1.1, registered: false } });
    const lines = exoClockCsv(source(frames), "jd_utc").trimEnd().split("\n");
    expect(lines.at(-1)).toBe("2460310.60208380,0.24,0.003021588986,1.2000,3.000,10.000,,");
  });

  it("writes zero drift on the reference row and leaves the others empty when drift was not tracked", () => {
    const frames = goldenFrames().map((f) => ({ ...f, offset: null }));
    const rows = exoClockCsv(source(frames), "jd_utc").trimEnd().split("\n").slice(-3);
    expect(rows.map((r) => r.split(",").slice(-2).join(","))).toEqual(["0.000,0.000", ",", ","]);
  });
});

describe("target and comparison names from the current Point regions (C02)", () => {
  const unnamedTargets = (): TimeSeriesTarget[] => TARGETS.map((t, i) => ({ ...t, label: defaultPointLabel(i) }));

  it("pairs the panel's default Point label with the unnamed-target test", () => {
    expect(defaultPointLabel(0)).toBe("P1");
    expect([0, 1, 9, 41].every((i) => isDefaultPointLabel(defaultPointLabel(i)))).toBe(true);
    expect(isDefaultPointLabel("HAT-P-32")).toBe(false);
    expect(isDefaultPointLabel("P1b")).toBe(false);
  });

  it("unblocks the export once the target's region is named, without measuring again", () => {
    const measured = source(goldenFrames(), unnamedTargets());
    expect(etdBlocker(measured)).toMatch(/^no target name/);
    const named = { ...measured, result: withCurrentLabels(measured.result, TARGETS) };
    expect(etdBlocker(named)).toBeNull();
    expect(etdFileName(named)).toBe("HAT-P-32_f0_etd.txt");
    expect(exoClockCsv(named, "jd_utc")).toBe(EXOCLOCK_JD_UTC_GOLDEN);
  });

  it("writes a target and a comparison renamed after the measurement", () => {
    const renamed = TARGETS.map((t, i) => ({ ...t, label: ["WASP-12", "TYC 1-2-3", "C2"][i] }));
    const measured = source();
    const relabelled = { ...measured, result: withCurrentLabels(measured.result, renamed) };
    const lines = exoClockCsv(relabelled, "bjd_tdb").split("\n");
    expect(lines).toContain("# target: WASP-12");
    expect(lines).toContain("# comparison stars: TYC 1-2-3, C2");
    expect(exoClockFileName(relabelled, "bjd_tdb")).toBe("WASP-12_f0_exoclock_bjd_tdb.csv");
  });

  it("keeps the measured name and role of a star whose region moved, was deleted or changed role", () => {
    const unnamed = unnamedTargets();
    const measured = source(goldenFrames(), unnamed);
    const moved = TARGETS.map((t) => (t.role === "target" ? { ...t, x: t.x + 0.5 } : t));
    expect(withCurrentLabels(measured.result, moved).targets[0]).toEqual(unnamed[0]);
    expect(withCurrentLabels(measured.result, []).targets).toEqual(unnamed);
    const recast = TARGETS.map((t) => ({ ...t, role: "ignore" as const }));
    expect(withCurrentLabels(measured.result, recast).targets.map((t) => t.role)).toEqual(["target", "comp", "comp"]);
  });
});

describe("magnitude and relative flux (C02)", () => {
  it("inverts 10^(-0.4 m) and the error factor within 1e-6", () => {
    for (const mag of [-3.2, -0.5, 0, 0.0123, 1.50515, 4.75, 9.9]) {
      for (const err of [1e-5, 0.0013, 0.05, 0.4]) {
        const flux = magToRelativeFlux(mag, err);
        expect(flux.flux).toBeCloseTo(10 ** (-0.4 * mag), 12);
        expect(flux.err).toBeCloseTo((flux.flux * err) / MAG_ERR_FACTOR, 12);
        expect(Math.abs(-2.5 * Math.log10(flux.flux) - mag)).toBeLessThan(1e-6);
        expect(Math.abs((MAG_ERR_FACTOR * flux.err) / flux.flux - err) / err).toBeLessThan(1e-6);
      }
    }
  });

  it("gives back the target over ensemble ratio and its relative error from the light curve's mag and err", () => {
    const src = source();
    const curve = lightCurve(src.result, 0, [1, 2], "jd");
    for (const point of curve.filter((p) => p.mag !== null)) {
      const f = src.result.frames[point.frameIndex];
      const ratio = f.targets[0]!.net_flux / 2000;
      const relErr = Math.sqrt((5 / f.targets[0]!.net_flux) ** 2 + (Math.sqrt(200) / 2000) ** 2);
      const flux = magToRelativeFlux(point.mag!, point.err!);
      expect(Math.abs(flux.flux - ratio) / ratio).toBeLessThan(1e-6);
      expect(Math.abs(flux.err / flux.flux - relErr) / relErr).toBeLessThan(1e-6);
    }
  });
});

describe("named blockers (C02)", () => {
  it("refuses without a target star", () => {
    const targets = TARGETS.map((t) => (t.role === "target" ? { ...t, role: "check" as const } : t));
    expect(etdBlocker(source(goldenFrames(), targets))).toBe(NO_TARGET_BLOCKER);
    expect(exoClockBlocker(source(goldenFrames(), targets), "jd_utc")).toBe(NO_TARGET_BLOCKER);
    expect(NO_TARGET_BLOCKER).toBe("no star has the target role");
  });

  it("refuses an unnamed target and says how to name it", () => {
    const targets = TARGETS.map((t) => (t.role === "target" ? { ...t, label: defaultPointLabel(0) } : t));
    const expected = 'no target name: label the target\'s Point region with the star\'s designation (now "P1")';
    expect(etdBlocker(source(goldenFrames(), targets))).toBe(expected);
    expect(exoClockBlocker(source(goldenFrames(), targets), "bjd_tdb")).toBe(expected);
    const blank = TARGETS.map((t) => (t.role === "target" ? { ...t, label: "  " } : t));
    expect(etdBlocker(source(goldenFrames(), blank))).toBe('no target name: label the target\'s Point region with the star\'s designation (now "  ")');
  });

  it("refuses when no frame has a differential magnitude", () => {
    const frames = goldenFrames().map((f) => ({ ...f, targets: [null, f.targets[1], f.targets[2]] }));
    expect(etdBlocker(source(frames))).toBe(NO_ROWS_BLOCKER);
    expect(NO_ROWS_BLOCKER).toBe("no frame has a differential magnitude");
  });

  it("refuses a measured frame without a UTC mid-exposure time and names DATE-OBS", () => {
    const frames = withFrame(3, { jd_mid: null, geometry: geometry(NaN, { jd_utc: null, bjd_tdb: null, hjd_utc: null }) });
    expect(etdBlocker(source(frames))).toBe("no JD_UTC for f3.fits (DATE-OBS missing or not a valid date)");
    expect(exoClockBlocker(source(frames), "jd_utc")).toBe("no JD_UTC for f3.fits (DATE-OBS missing or not a valid date)");
  });

  it("quotes the refusal note of a frame whose header time was not used, instead of blaming DATE-OBS", () => {
    const reason =
      "time reference keywords disagree (TREFPOS TOPOCENTER at the observer, TIMEREF SOLARSYSTEM at the barycentre): the header time is not used, so no light-time correction is added or assumed";
    const refused = geometry(NaN, { jd_utc: null, bjd_tdb: null, hjd_utc: null, time_scale_notes: [reason] });
    const frames = withFrame(3, { jd_mid: null, geometry: refused });
    expect(etdBlocker(source(frames))).toBe(`no JD_UTC for f3.fits (${reason})`);
    expect(exoClockBlocker(source(frames), "bjd_tdb")).toBe(`no BJD_TDB for f3.fits (${reason})`);
    expect(exoClockBlocker(source(frames), "hjd_utc")).toBe(`no HJD_UTC for f3.fits (${reason})`);
  });

  it("refuses JD_UTC for a barycentric header time and leaves BJD_TDB open", () => {
    const frames = withFrame(1, { geometry: geometry(2460310.6020838, { bjd_source: "header", hjd_utc: null }) });
    const expected = "no JD_UTC for f1.fits (the header time is barycentric: TREFPOS/TIMEREF/BJDREF)";
    expect(etdBlocker(source(frames))).toBe(expected);
    expect(exoClockBlocker(source(frames), "jd_utc")).toBe(expected);
    expect(exoClockBlocker(source(frames), "bjd_tdb")).toBeNull();
    expect(exoClockBlocker(source(frames), "hjd_utc")).toBe("no HJD_UTC for f1.fits (the header time is barycentric: TREFPOS/TIMEREF/BJDREF)");
  });

  it("writes the header BJD_TDB of a TDB barycentric series within 1e-8 d and refuses the UTC columns", () => {
    const headerBjd = (i: number) => 2400000.5 + 60310.25 + (i * 60) / 86400;
    const utcOf = (i: number) => headerBjd(i) - 69.184 / 86400;
    const frames = [0, 1, 2].map((i) =>
      frame(i, utcOf(i), 500 - 10 * i, {
        jd_mid: headerBjd(i),
        time_source: "mid-exposure from MJD-AVG; time scale TDB (TIMESYS), not converted to UTC",
        geometry: geometry(utcOf(i), { bjd_tdb: headerBjd(i), hjd_utc: null, bjd_source: "header" }),
      }),
    );
    expect(exoClockBlocker(source(frames), "bjd_tdb")).toBeNull();
    const times = exoClockCsv(source(frames), "bjd_tdb").trimEnd().split("\n").slice(-3).map((l) => Number(l.split(",")[0]));
    times.forEach((t, i) => expect(Math.abs(t - headerBjd(i))).toBeLessThanOrEqual(1e-8));
    const barycentric = "(the header time is barycentric: TREFPOS/TIMEREF/BJDREF)";
    expect(etdBlocker(source(frames))).toBe(`no JD_UTC for f0.fits ${barycentric}`);
    expect(exoClockBlocker(source(frames), "hjd_utc")).toBe(`no HJD_UTC for f0.fits ${barycentric}`);
  });

  it("refuses BJD_TDB and HJD_UTC exports without the barycentric or heliocentric time and leaves ETD open", () => {
    const frames = withFrame(1, { geometry: geometry(2460310.6020838, { bjd_tdb: null, hjd_utc: null, bjd_source: null }) });
    expect(exoClockBlocker(source(frames), "bjd_tdb")).toBe("no BJD_TDB for f1.fits (needs DATE-OBS and the target RA/Dec)");
    expect(exoClockBlocker(source(frames), "hjd_utc")).toBe("no HJD_UTC for f1.fits (needs DATE-OBS and the target RA/Dec)");
    expect(exoClockBlocker(source(frames), "jd_utc")).toBeNull();
    expect(etdBlocker(source(frames))).toBeNull();
  });

  it("refuses a measured frame whose time is the exposure start because no exposure card was found", () => {
    const frames = withFrame(3, { exptime: null, time_source: EXPOSURE_START_SOURCE });
    const expected = "no EXPTIME for f3.fits (the time is the exposure start, not mid-exposure)";
    expect(etdBlocker(source(frames))).toBe(expected);
    expect(exoClockBlocker(source(frames), "jd_utc")).toBe(expected);
    expect(exoClockBlocker(source(frames), "bjd_tdb")).toBe(expected);
  });

  it("accepts frames without EXPTIME whose mid-exposure time came from EXPOSURE, DATE-AVG or MJD-MID", () => {
    const exposure = goldenFrames().map((f) => ({ ...f, exptime: null, time_source: "DATE-OBS + EXPOSURE/2 (60 s); UTC assumed (no TIMESYS)" }));
    expect(etdBlocker(source(exposure))).toBeNull();
    expect(exoClockBlocker(source(exposure), "bjd_tdb")).toBeNull();
    expect(exoClockCsv(source(exposure), "jd_utc").split("\n")).toContain("# exposure s: 60");
    const averaged = goldenFrames().map((f, i) => ({
      ...f,
      exptime: null,
      time_source: i === 1 ? "mid-exposure from MJD-MID; time scale UTC (TIMESYS)" : "mid-exposure from DATE-AVG; UTC assumed (no TIMESYS)",
    }));
    expect(etdBlocker(source(averaged))).toBeNull();
    expect(exoClockBlocker(source(averaged), "jd_utc")).toBeNull();
    expect(exoClockCsv(source(averaged), "jd_utc").split("\n")).toContain("# exposure s: unknown");
  });

  it("refuses a measured frame whose DATE-OBS has no time of day", () => {
    const frames = withFrame(1, { time_source: "DATE-OBS (date only, 0h UT assumed) + EXPTIME/2 (60 s); UTC assumed (no TIMESYS)" });
    const expected = "no time of day for f1.fits (DATE-OBS is a date only)";
    expect(etdBlocker(source(frames))).toBe(expected);
    for (const time of ["jd_utc", "bjd_tdb", "hjd_utc"] as const) expect(exoClockBlocker(source(frames), time)).toBe(expected);
  });

  it("reads the date-only, exposure-start and half-exposure notes exactly as core/astrometry/spectral.rs writes them", () => {
    const rust = readFileSync(fileURLToPath(new URL("../../../src-tauri/src/core/astrometry/spectral.rs", import.meta.url)), "utf8");
    const dateOnly = /if date_only \{\s*"([^"]+)"/.exec(rust)?.[1];
    const startNote = /let exposure_note = match exposure \{[\s\S]*?None => "([^"]+)"\.to_string\(\)/.exec(rust)?.[1];
    const halfNote = /Some\(\(s, key\)\) => format!\("([^"]+)", key, s\)/.exec(rust)?.[1];
    expect(dateOnly, "date-only source").toBeDefined();
    expect(startNote, "exposure-start note").toBeDefined();
    expect(halfNote, "half-exposure note").toBeDefined();
    const half = halfNote!.replace("{}", "EXPTIME").replace("{}", "60");
    const dated = withFrame(1, { time_source: `${dateOnly} ${half}; UTC assumed (no TIMESYS)` });
    expect(etdBlocker(source(dated))).toBe("no time of day for f1.fits (DATE-OBS is a date only)");
    const started = withFrame(3, { exptime: null, time_source: `DATE-OBS ${startNote}; UTC assumed (no TIMESYS)` });
    expect(etdBlocker(source(started))).toBe("no EXPTIME for f3.fits (the time is the exposure start, not mid-exposure)");
    const mid = goldenFrames().map((f) => ({ ...f, exptime: null, time_source: `DATE-OBS ${half}; UTC assumed (no TIMESYS)` }));
    expect(exoClockCsv(source(mid), "jd_utc").split("\n")).toContain("# exposure s: 60");
  });

  it("refuses a measured frame without a magnitude error", () => {
    const frames = withFrame(1, { targets: [star(480, NaN), star(1000, NaN), star(1000, NaN)] });
    expect(etdBlocker(source(frames))).toBe("no errors for f1.fits (flux_err missing)");
    expect(exoClockBlocker(source(frames), "jd_utc")).toBe("no errors for f1.fits (flux_err missing)");
  });

  it("refuses a measured frame whose target or one comparison lacks flux_err even when the others have one", () => {
    const noTargetErr = withFrame(1, { targets: [star(480, NaN), star(1000, 10), star(1000, 10)] });
    expect(etdBlocker(source(noTargetErr))).toBe("no errors for f1.fits (flux_err missing)");
    const noCompErr = withFrame(3, { targets: [star(490, 5), star(1000, 10), star(1000, NaN)] });
    expect(exoClockBlocker(source(noCompErr), "jd_utc")).toBe("no errors for f3.fits (flux_err missing)");
  });

  it("refuses a series whose frames disagree on the AREA pixel-area map", () => {
    const mixed: LightCurveExportSource = { ...source(), result: { ...result(goldenFrames()), area_mixed: true } };
    const expected = "AREA pixel-area map applied on some frames only (a step of a few hundredths of a magnitude in the light curve, see the series warning)";
    expect(AREA_MIXED_BLOCKER).toBe(expected);
    expect(etdBlocker(mixed)).toBe(expected);
    for (const time of ["jd_utc", "bjd_tdb", "hjd_utc"] as const) expect(exoClockBlocker(mixed, time)).toBe(expected);
    expect(() => etdText(mixed)).toThrow(expected);
    expect(etdBlocker(source())).toBeNull();
  });

  it("ignores skipped frames when looking for blockers", () => {
    const frames = withFrame(2, { exptime: null, jd_mid: null, geometry: null });
    expect(etdBlocker(source(frames))).toBeNull();
    expect(exoClockBlocker(source(frames), "bjd_tdb")).toBeNull();
  });

  it("lists each format's blocker for the panel and merges an identical one", () => {
    expect(exportBlockerNotes(null, null)).toEqual([]);
    expect(exportBlockerNotes(NO_TARGET_BLOCKER, NO_TARGET_BLOCKER)).toEqual([
      { key: "etd-exoclock", text: "ETD and ExoClock: no star has the target role" },
    ]);
    expect(exportBlockerNotes("no EXPTIME for f3.fits", null)).toEqual([{ key: "etd", text: "ETD: no EXPTIME for f3.fits" }]);
    expect(exportBlockerNotes(null, "no BJD_TDB for f1.fits (needs DATE-OBS and the target RA/Dec)")).toEqual([
      { key: "exoclock", text: "ExoClock: no BJD_TDB for f1.fits (needs DATE-OBS and the target RA/Dec)" },
    ]);
  });

  it("throws the blocker instead of writing a blocked file", () => {
    const frames = withFrame(3, { exptime: null, time_source: EXPOSURE_START_SOURCE });
    expect(() => etdText(source(frames))).toThrow("no EXPTIME for f3.fits");
    expect(() => exoClockCsv(source(frames), "jd_utc")).toThrow("no EXPTIME for f3.fits");
  });
});
