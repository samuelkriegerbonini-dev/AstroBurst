import { describe, it, expect } from "vitest";
import {
  CSV_COLUMNS,
  continuumModel,
  convergedFit,
  formatAxisQuantity,
  formatQuantity,
  gaussianModel,
  lineMeasurementAxis,
  lineMeasurementCsv,
  lineMeasurementRows,
  lineOverlayPolylines,
  measurementAxisValues,
  measurementSpan,
  momentVelocityFrameNote,
  POOR_FIT_CHI2_RED,
  runForSource,
  spectrumSourceKey,
  spectrumSourceLabel,
  type PlotFrame,
} from "../lineMeasure";
import { formatWithUncertainty as fmt } from "../formatUncertainty";
import { airToVacuumUm, formatAxis } from "../spectralAxis";
import { axisValueToPixel, channelAxisValue, type PlotMapping } from "../spectrumRange";
import type { LineMeasurement, SeriesSpectrumSource, SpectralAxisInfo } from "../../shared/types/spectral";

const AXIS = Array.from({ length: 40 }, (_, i) => 1.0 + i * 0.001);

function measurement(overrides: Partial<LineMeasurement> = {}): LineMeasurement {
  return {
    z0: 8,
    z1: 32,
    n_channels: 25,
    n_channels_nominal: 25,
    axis_unit: "um",
    flux_unit: "Jy/beam x um",
    continuum_level: 1.0,
    continuum_slope: 0.0,
    continuum_sigma: 0.01,
    continuum_chi2_red: null,
    continuum_reference: 1.02,
    continuum_linear: true,
    continuum_channels: 12,
    continuum_windows: [
      [0, 5],
      [34, 39],
    ],
    flux: 7.52e-3,
    flux_err: 5e-5,
    equivalent_width: -7.52e-3,
    centroid: 1.02,
    sigma: 0.003,
    fwhm: 0.00706,
    peak: 1.0,
    peak_channel: 20,
    snr: 150.4,
    velocity: {
      centroid_kms: 0.2,
      sigma_kms: 881.7,
      fwhm_kms: 2076.3,
      rest_um: 1.02,
      convention: "optical",
      shift_applied_kms: 0,
      axis_frame: null,
      redshift: 0,
      source_centroid_kms: 0.2,
      source_sigma_kms: 881.7,
      source_fwhm_kms: 2076.3,
    },
    fit: {
      amplitude: 1.0,
      centre: 1.02,
      sigma: 0.003,
      amplitude_err: 0.01,
      centre_err: 1e-5,
      sigma_err: 1e-5,
      chi2: 20.5,
      dof: 22,
      iterations: 6,
      converged: true,
    },
    err_used: false,
    flux_err_terms: null,
    n_dq_excluded: 0,
    n_nonfinite: 0,
    n_bad_err: 0,
    err_source: "none",
    notes: ["continuum: first-order line"],
    elapsed_ms: 3,
    source: { kind: "pixel", x: 16, y: 16 },
    ...overrides,
  };
}

function mapping(xValues: number[] | null, n = 40): PlotMapping {
  const values = xValues ?? Array.from({ length: n }, (_, i) => i);
  return { width: 400, padLeft: 50, padRight: 12, xMin: Math.min(...values), xMax: Math.max(...values), xValues, n };
}

const FRAME: PlotFrame = { top: 10, height: 146, yMin: 0.9, yMax: 2.1 };

describe("gaussianModel", () => {
  const continuum = { level: 1.0, slope: 0.0, reference: 1.02 };
  const fit = { amplitude: 1.0, centre: 1.02, sigma: 0.003 };

  it("peaks at level plus amplitude at the centre and halves at 1.1774 sigma", () => {
    const [centre, right, left] = gaussianModel(fit, continuum, [1.02, 1.02 + 1.1774 * 0.003, 1.02 - 1.1774 * 0.003]);
    expect(centre).toBeCloseTo(2.0, 12);
    expect(right).toBeCloseTo(1.5, 4);
    expect(left).toBeCloseTo(1.5, 4);
  });

  it("adds the sloped continuum around its reference", () => {
    const sloped = { level: 1.0, slope: 10.0, reference: 1.02 };
    const [far] = gaussianModel(fit, sloped, [1.12]);
    expect(far).toBeCloseTo(2.0, 9);
  });
});

describe("continuumModel", () => {
  it("is linear in x around the reference", () => {
    const m = measurement({ continuum_level: 2.0, continuum_slope: 3.0, continuum_reference: 1.0 });
    expect(continuumModel(m, [1.0, 1.5, 2.0])).toEqual([2.0, 3.5, 5.0]);
  });

  it("returns NaN when the continuum is unavailable", () => {
    const m = measurement({ continuum_level: null });
    expect(continuumModel(m, [1.0, 2.0]).every((v) => Number.isNaN(v))).toBe(true);
  });
});

describe("lineMeasurementCsv", () => {
  it("writes one header and one row in the fixed column order", () => {
    const csv = lineMeasurementCsv(measurement());
    const [header, row, extra] = csv.split("\n");
    expect(extra).toBeUndefined();
    expect(header).toBe(CSV_COLUMNS.join(","));
    const cells = row.split(",");
    expect(cells).toHaveLength(CSV_COLUMNS.length);
    expect(cells[CSV_COLUMNS.indexOf("flux")]).toBe("0.00752");
    expect(cells[CSV_COLUMNS.indexOf("velocity_convention")]).toBe("optical");
    expect(cells[CSV_COLUMNS.indexOf("fit_converged")]).toBe("true");
    expect(cells[CSV_COLUMNS.indexOf("continuum_linear")]).toBe("true");
    expect(cells[CSV_COLUMNS.indexOf("flux_unit")]).toBe("Jy/beam x um");
  });

  it("leaves nulls, missing velocity and missing fit blank", () => {
    const csv = lineMeasurementCsv(measurement({ velocity: null, fit: null, sigma: null, fwhm: null }));
    const cells = csv.split("\n")[1].split(",");
    expect(cells).toHaveLength(CSV_COLUMNS.length);
    expect(cells[CSV_COLUMNS.indexOf("sigma")]).toBe("");
    expect(cells[CSV_COLUMNS.indexOf("velocity_centroid_kms")]).toBe("");
    expect(cells[CSV_COLUMNS.indexOf("fit_amplitude")]).toBe("");
    expect(cells[CSV_COLUMNS.indexOf("fit_converged")]).toBe("");
  });

  it("quotes text cells that contain commas", () => {
    const csv = lineMeasurementCsv(measurement({ flux_unit: "MJy/sr, summed x um" }));
    expect(csv.split("\n")[1]).toContain('"MJy/sr, summed x um"');
  });

  it("writes the axis frame of the velocities", () => {
    const base = measurement();
    const cells = lineMeasurementCsv(measurement({ velocity: { ...base.velocity!, axis_frame: "LSRK" } }))
      .split("\n")[1]
      .split(",");
    expect(cells[CSV_COLUMNS.indexOf("velocity_axis_frame")]).toBe("LSRK");
  });
});

describe("line measurement provenance", () => {
  it("writes the measured source into the CSV row", () => {
    const cells = lineMeasurementCsv(measurement()).split("\n")[1].split(",");
    expect(cells[CSV_COLUMNS.indexOf("source_kind")]).toBe("pixel");
    expect(cells[CSV_COLUMNS.indexOf("source_x")]).toBe("16");
    expect(cells[CSV_COLUMNS.indexOf("source_y")]).toBe("16");
    expect(cells[CSV_COLUMNS.indexOf("source_region")]).toBe("");
    const disk = { shape: "circle", x: 16, y: 16, r: 4 } as const;
    const row = lineMeasurementCsv(measurement({ source: { kind: "region", shape: disk, background: null } })).split("\n")[1];
    expect(row.startsWith('region,,,"{""shape"":{""shape"":""circle""')).toBe(true);
  });

  it("drops a run measured on another pixel or region and labels the source", () => {
    const run = { key: spectrumSourceKey({ kind: "pixel", x: 16, y: 16 }, "sum") };
    expect(runForSource(run, { kind: "pixel", x: 2, y: 2 }, "sum")).toBeNull();
    expect(runForSource(run, { kind: "pixel", x: 16, y: 16 }, "sum")).toBe(run);
    expect(runForSource(run, { kind: "pixel", x: 16, y: 16 }, "mean")).toBeNull();
    expect(runForSource(run, null, "sum")).toBeNull();
    expect(runForSource(null, { kind: "pixel", x: 16, y: 16 }, "sum")).toBeNull();
    const small = { shape: "circle", x: 20, y: 20, r: 3 } as const;
    const large = { shape: "circle", x: 20, y: 20, r: 6 } as const;
    const regionRun = { key: spectrumSourceKey({ kind: "region", shape: small, background: null }, "sum") };
    expect(runForSource(regionRun, { kind: "region", shape: large, background: null }, "sum")).toBeNull();
    expect(spectrumSourceLabel({ kind: "pixel", x: 16, y: 16 })).toBe("pixel (16, 16)");
    expect(spectrumSourceLabel({ kind: "region", shape: small, background: null })).toBe("circle region");
    const annulus = { shape: "annulus", x: 20, y: 20, r_inner: 5, r_outer: 8 } as const;
    expect(spectrumSourceLabel({ kind: "region", shape: small, background: annulus })).toBe("circle region with background");
  });

  it("labels a spectrum file source with its HDU when the backend resolved one", () => {
    expect(spectrumSourceLabel({ kind: "file", hdu: 1 })).toBe("spectrum file, HDU 1");
    expect(spectrumSourceLabel({ kind: "file", hdu: 0 })).toBe("spectrum file, HDU 0");
    expect(spectrumSourceLabel({ kind: "file", hdu: null })).toBe("spectrum file");
  });
});

describe("momentVelocityFrameNote", () => {
  it("names the header frame of M1", () => {
    expect(momentVelocityFrameNote("BARYCENT", "none", null)).toBe("M1 in the header frame BARYCENT");
    expect(momentVelocityFrameNote(null, "barycentric", 0)).toBe("M1 in the header frame (no SPECSYS)");
  });

  it("says M1 lacks the frame shift that the line velocity includes", () => {
    expect(momentVelocityFrameNote("TOPOCENT", "barycentric", 25.3)).toBe(
      "M1 in the header frame TOPOCENT, without the barycentric shift of 25.3 km/s that the line velocity includes",
    );
  });
});

describe("lineMeasurementRows", () => {
  it("lists the metrics with their units and velocities", () => {
    const rows = lineMeasurementRows(measurement());
    const labels = rows.map((r) => r.label);
    expect(labels).toEqual([
      "Flux",
      "Equivalent width (absorption > 0, emission < 0)",
      "Centroid",
      "Sigma (2nd moment)",
      "FWHM (2.355 x sigma)",
      "Peak",
      "SNR",
      "Continuum",
      "Velocity frame",
      "Gaussian fit",
      "Error source",
      "Excluded channels",
    ]);
    expect(rows[0].value).toBe(fmt(7.52e-3, 5e-5, { unit: "Jy/beam x um" }));
    expect(rows[2].value).toBe("1.02000 um / 0.2 km/s");
    expect(rows[5].value).toBe("1.000 Jy/beam at ch 20");
    expect(rows[6].value).toBe("150.4 (emission)");
    expect(rows[8].value).toBe("optical, rest 1.02000 um, z = 0, axis frame not declared (no SPECSYS)");
  });

  it("states the equivalent-width sign of specutils and IRAF splot next to a negative emission width", () => {
    const row = lineMeasurementRows(measurement()).find((r) => r.label.startsWith("Equivalent width"));
    expect(row?.label).toBe("Equivalent width (absorption > 0, emission < 0)");
    expect(row?.value).toBe("-0.007520 um");
  });

  it("names the axis frame instead of assuming topocentric, and adds a frame shift when one was applied", () => {
    const base = measurement();
    const velocity = base.velocity!;
    const frameRow = (m: LineMeasurement) => lineMeasurementRows(m).find((r) => r.label === "Velocity frame")?.value;
    expect(frameRow(measurement({ velocity: { ...velocity, axis_frame: "BARYCENT", shift_applied_kms: 0 } }))).toBe(
      "optical, rest 1.02000 um, z = 0, axis frame BARYCENT",
    );
    expect(frameRow(measurement({ velocity: { ...velocity, axis_frame: null, shift_applied_kms: 0 } }))).not.toContain("topocentric");
    expect(frameRow(measurement({ velocity: { ...velocity, axis_frame: "TOPOCENT", shift_applied_kms: 25.3 } }))).toBe(
      "optical, rest 1.02000 um, z = 0, axis frame TOPOCENT, shift 25.3 km/s",
    );
  });

  it("keeps significant digits of narrow fit widths and their uncertainties", () => {
    const rows = lineMeasurementRows(
      measurement({
        sigma: 1.1e-4,
        fwhm: 2.59e-4,
        velocity: null,
        fit: {
          amplitude: 1,
          centre: 0.6550412,
          sigma: 1.1e-4,
          amplitude_err: 0.025,
          centre_err: 3.09e-6,
          sigma_err: 3.25e-6,
          chi2: 20.5,
          dof: 22,
          iterations: 6,
          converged: true,
        },
      }),
    );
    const fit = rows.find((r) => r.label === "Gaussian fit")?.value ?? "";
    expect(fit).toContain("c 0.6550412 ± 3.090e-6");
    expect(fit).toContain("s 1.100e-4 ± 3.250e-6 um");
    expect(fit).not.toContain("0.00000");
    expect(rows.find((r) => r.label === "Sigma (2nd moment)")?.value).toBe("1.100e-4 um");
    expect(rows.find((r) => r.label === "FWHM (2.355 x sigma)")?.value).toBe("2.590e-4 um");
  });

  it("chooses axis decimals from the resolution and keeps five decimals otherwise", () => {
    expect(formatAxisQuantity(1.02)).toBe("1.02000");
    expect(formatAxisQuantity(1.02, 0.01)).toBe("1.02000");
    expect(formatAxisQuantity(0.6550412, 3.09e-6)).toBe("0.6550412");
    expect(formatAxisQuantity(0.5, 1e-15)).toBe("0.5000000000");
    expect(formatAxisQuantity(1.02, 0)).toBe("1.02000");
    expect(formatAxisQuantity(1.02, NaN)).toBe("1.02000");
    expect(formatAxisQuantity(null, 1e-6)).toBe("n/a");
  });

  it("marks unavailable values and drops the velocity and fit rows when absent", () => {
    const rows = lineMeasurementRows(measurement({ velocity: null, fit: null, sigma: null, snr: null }));
    expect(rows.map((r) => r.label)).not.toContain("Gaussian fit");
    expect(rows.find((r) => r.label === "Sigma (2nd moment)")?.value).toBe("n/a um");
    expect(rows.find((r) => r.label === "SNR")?.value).toBe("n/a");
  });

  it("formats tiny and huge numbers in exponent form and others with four digits", () => {
    expect(formatQuantity(7.52e-3)).toBe("0.007520");
    expect(formatQuantity(5e-5)).toBe("5.000e-5");
    expect(formatQuantity(1234567)).toBe("1.235e+6");
    expect(formatQuantity(1.5)).toBe("1.500");
    expect(formatQuantity(0)).toBe("0.000");
    expect(formatQuantity(null)).toBe("n/a");
    expect(formatQuantity(NaN)).toBe("n/a");
  });
});

const REDSHIFTED: Partial<LineMeasurement> = {
  centroid: 0.98469,
  sigma: 3.2846e-4,
  fwhm: 7.7346e-4,
  velocity: {
    centroid_kms: 149896.229,
    sigma_kms: 150.0,
    fwhm_kms: 353.223,
    rest_um: 0.65646,
    convention: "optical",
    shift_applied_kms: 0,
    axis_frame: null,
    redshift: 0.5,
    source_centroid_kms: 0,
    source_sigma_kms: 100.0,
    source_fwhm_kms: 235.482,
  },
};

describe("lineMeasurementRows at a redshift", () => {
  const row = (m: LineMeasurement, label: string) => lineMeasurementRows(m).find((r) => r.label === label)?.value;
  const OBSERVER = "Observer frame (against the laboratory rest wavelength)";

  it("puts the source-frame velocity and widths beside the centroid, sigma and FWHM", () => {
    const m = measurement(REDSHIFTED);
    expect(row(m, "Centroid")).toBe("0.98469 um / 0.0 km/s");
    expect(row(m, "Sigma (2nd moment)")).toBe("3.285e-4 um / 100.0 km/s");
    expect(row(m, "FWHM (2.355 x sigma)")).toBe("7.735e-4 um / 235.5 km/s");
  });

  it("moves the observer-frame values against the laboratory rest wavelength to their own row and states z", () => {
    const m = measurement(REDSHIFTED);
    expect(row(m, OBSERVER)).toBe("centroid 149896.2 km/s, sigma 150.0 km/s, FWHM 353.2 km/s");
    expect(row(m, "Velocity frame")).toBe("optical, rest 0.65646 um, z = 0.5, axis frame not declared (no SPECSYS)");
    const labels = lineMeasurementRows(m).map((r) => r.label);
    expect(labels.indexOf(OBSERVER)).toBe(labels.indexOf("Velocity frame") + 1);
  });

  it("shows no observer-frame row at z = 0, where it would repeat the source frame", () => {
    expect(row(measurement(), OBSERVER)).toBeUndefined();
    expect(row(measurement({ velocity: null }), OBSERVER)).toBeUndefined();
  });
});

describe("lineMeasurementRows for absorption and undefined widths", () => {
  const rowOf = (m: LineMeasurement, label: string) => lineMeasurementRows(m).find((r) => r.label === label);

  it("labels the extremum of a line with negative flux as its trough", () => {
    const absorption = measurement({ flux: -3.759806e-3, equivalent_width: 3.76e-3, peak: -0.49999882, peak_channel: 20 });
    expect(rowOf(absorption, "Peak")).toBeUndefined();
    expect(rowOf(absorption, "Trough")?.value).toBe("-0.5000 Jy/beam at ch 20");
    expect(rowOf(measurement(), "Peak")?.value).toBe("1.000 Jy/beam at ch 20");
    expect(rowOf(measurement({ flux: 0 }), "Peak")).toBeDefined();
  });

  it("prints an undefined equivalent width as unavailable", () => {
    expect(rowOf(measurement({ equivalent_width: null }), "Equivalent width (absorption > 0, emission < 0)")?.value).toBe("n/a um");
  });

  it("names the continuum scatter as the robust sigma it is", () => {
    expect(rowOf(measurement(), "Continuum")?.value).toBe("level 1.000, slope 0.000, σ(MAD) 0.01000, 12 ch linear");
  });
});

describe("line overlay on a faint F_lambda spectrum", () => {
  it("places a 1e-17 continuum inside a 0.9e-17 to 2.1e-17 frame instead of on a 1e-10 floor", () => {
    const xs = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);
    const m: PlotMapping = { width: 400, padLeft: 50, padRight: 12, xMin: xs[0], xMax: xs[39], xValues: xs, n: 40 };
    const run = measurement({
      z0: 15,
      z1: 25,
      continuum_level: 1e-17,
      continuum_slope: 0,
      continuum_reference: xs[20],
      continuum_windows: [
        [0, 4],
        [35, 39],
      ],
      fit: null,
    });
    const { continuum } = lineOverlayPolylines(run, xs, m, { top: 10, height: 146, yMin: 0.9e-17, yMax: 2.1e-17 });
    expect(continuum.length).toBeGreaterThan(0);
    expect(continuum[0].y).toBeCloseTo(143.83, 1);
  });
});

describe("lineOverlayPolylines", () => {
  it("maps the continuum over the window span and the model over the line through axisValueToPixel", () => {
    const m = mapping(AXIS);
    const { continuum, model } = lineOverlayPolylines(measurement(), AXIS, m, FRAME);
    expect(continuum[0].x).toBeCloseTo(axisValueToPixel(AXIS[0], m), 9);
    expect(continuum[continuum.length - 1].x).toBeCloseTo(axisValueToPixel(AXIS[39], m), 9);
    const continuumY = FRAME.top + FRAME.height - ((1.0 - FRAME.yMin) / (FRAME.yMax - FRAME.yMin)) * FRAME.height;
    expect(continuum.every((p) => Math.abs(p.y - continuumY) < 1e-9)).toBe(true);
    expect(model[0].x).toBeCloseTo(axisValueToPixel(AXIS[8], m), 9);
    expect(model[model.length - 1].x).toBeCloseTo(axisValueToPixel(AXIS[32], m), 9);
    const peak = model.reduce((best, p) => (p.y < best.y ? p : best), model[0]);
    expect(peak.x).toBeCloseTo(axisValueToPixel(1.02, m), 9);
    expect(peak.y).toBeCloseTo(FRAME.top + FRAME.height - ((2.0 - FRAME.yMin) / (FRAME.yMax - FRAME.yMin)) * FRAME.height, 9);
    expect(model.length).toBe(24 * 4 + 1);
  });

  it("follows the display axis when it differs from the measurement axis", () => {
    const velocityDisplay = AXIS.map((w) => ((w - 1.02) / 1.02) * 299792.458);
    const m = mapping(velocityDisplay);
    const { model } = lineOverlayPolylines(measurement(), AXIS, m, FRAME);
    const peak = model.reduce((best, p) => (p.y < best.y ? p : best), model[0]);
    expect(peak.x).toBeCloseTo(axisValueToPixel(0, m), 9);
  });

  it("clips to the plot frame and skips a fit that did not converge", () => {
    const narrow = { ...mapping(AXIS), xMin: 1.01, xMax: 1.03 };
    const converged = lineOverlayPolylines(measurement(), AXIS, narrow, FRAME);
    expect(converged.continuum.every((p) => p.x >= narrow.padLeft && p.x <= narrow.width - narrow.padRight)).toBe(true);
    expect(converged.continuum.length).toBeLessThan(40 * 4);
    const tall = { ...FRAME, yMin: 1.2, yMax: 1.4 };
    const clipped = lineOverlayPolylines(measurement(), AXIS, mapping(AXIS), tall);
    expect(clipped.model.every((p) => p.y >= tall.top && p.y <= tall.top + tall.height)).toBe(true);
    const unconverged = measurement({ fit: { ...measurement().fit!, converged: false } });
    expect(lineOverlayPolylines(unconverged, AXIS, mapping(AXIS), FRAME).model).toEqual([]);
    expect(lineOverlayPolylines(measurement({ fit: null }), AXIS, mapping(AXIS), FRAME).model).toEqual([]);
  });

  it("returns nothing when the axis length does not match the plot or the continuum is missing", () => {
    expect(lineOverlayPolylines(measurement(), AXIS.slice(0, 10), mapping(AXIS), FRAME)).toEqual({ continuum: [], model: [] });
    expect(lineOverlayPolylines(measurement({ continuum_level: null }), AXIS, mapping(AXIS), FRAME)).toEqual({
      continuum: [],
      model: [],
    });
    expect(lineOverlayPolylines(measurement(), [], mapping(null, 0), FRAME)).toEqual({ continuum: [], model: [] });
  });
});

describe("measurement helpers", () => {
  function axisOf(kind: SpectralAxisInfo["kind"], values: number[]): SpectralAxisInfo {
    return {
      kind,
      ctype: kind.toUpperCase(),
      unit: "um",
      header_unit: "um",
      header_scale: 1,
      values,
      crval: values[0],
      cdelt: 0.001,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
    };
  }

  it("converts air wavelengths to vacuum and keeps other axes as stored", () => {
    expect(measurementAxisValues(null)).toBeNull();
    expect(measurementAxisValues(axisOf("unknown", AXIS))).toBeNull();
    expect(measurementAxisValues(axisOf("wave", AXIS))).toBe(AXIS);
    expect(measurementAxisValues(axisOf("freq", AXIS))).toBe(AXIS);
    expect(measurementAxisValues(axisOf("vrad", AXIS))).toBe(AXIS);
    const air = measurementAxisValues(axisOf("awav", AXIS))!;
    expect(air[20]).toBeCloseTo(airToVacuumUm(AXIS[20]), 12);
    expect(air[20]).toBeGreaterThan(AXIS[20]);
  });

  it("keys a source by kind, view and geometry", () => {
    const pixel = spectrumSourceKey({ kind: "pixel", x: 1, y: 2 }, "sum");
    expect(pixel).toBe('pixel:sum:{"kind":"pixel","x":1,"y":2}');
    const shape = { shape: "circle", x: 1, y: 2, r: 3 } as const;
    expect(spectrumSourceKey({ kind: "region", shape, background: null }, "sum")).not.toBe(
      spectrumSourceKey({ kind: "region", shape, background: null }, "mean"),
    );
  });

  it("spans the windows and the line and recognises a converged fit", () => {
    expect(measurementSpan(measurement())).toEqual({ first: 0, last: 39 });
    expect(
      measurementSpan(
        measurement({
          continuum_windows: [
            [10, 12],
            [30, 31],
          ],
        }),
      ),
    ).toEqual({ first: 8, last: 32 });
    expect(convergedFit(null)).toBeNull();
    expect(convergedFit({ ...measurement().fit!, sigma: null })).toBeNull();
    expect(convergedFit(measurement().fit)).toEqual({ amplitude: 1.0, centre: 1.02, sigma: 0.003 });
  });
});

const FILE_WEIGHTED: Partial<LineMeasurement> = {
  source: { kind: "file", hdu: 1 },
  flux_unit: "Jy x um",
  err_used: true,
  err_source: "FLUX_ERROR column (HDU 1)",
  flux_err_terms: { pixel: 4e-5, level: 2.5e-5, slope: 1.2e-6 },
  n_dq_excluded: 3,
  n_nonfinite: 2,
  n_bad_err: 58,
};

describe("lineMeasurementRows error rows", () => {
  const row = (m: LineMeasurement, label: string) => lineMeasurementRows(m).find((r) => r.label === label)?.value;

  it("names the ERR source used as weights", () => {
    expect(row(measurement(FILE_WEIGHTED), "Error source")).toBe("FLUX_ERROR column (HDU 1), weights 1/ERR^2");
    expect(row(measurement({ err_used: true, err_source: "ERR HDU 2" }), "Error source")).toBe("ERR HDU 2, weights 1/ERR^2");
  });

  it("says the flux error comes from the continuum scatter when no ERR was used", () => {
    expect(row(measurement(), "Error source")).toBe("none: flux error from the continuum scatter");
  });

  it("lists the pixel, level and slope terms of the flux error in the flux unit, and no row when the backend sent no terms", () => {
    expect(row(measurement(FILE_WEIGHTED), "Error terms")).toBe("pixel 4.000e-5, level 2.500e-5, slope 1.200e-6 Jy x um");
    expect(row(measurement(), "Error terms")).toBeUndefined();
    const order = lineMeasurementRows(measurement(FILE_WEIGHTED)).map((r) => r.label);
    expect(order.slice(-3)).toEqual(["Error source", "Error terms", "Excluded channels"]);
  });

  it("lists the three terms of the continuum-scatter error when no ERR was used", () => {
    const scatter = measurement({ err_used: false, flux_err_terms: { pixel: 2.4e-4, level: 9.1e-5, slope: 3e-6 } });
    expect(row(scatter, "Error source")).toBe("none: flux error from the continuum scatter");
    expect(row(scatter, "Error terms")).toBe("pixel 2.400e-4, level 9.100e-5, slope 3.000e-6 Jy/beam x um");
  });

  it("says how much the continuum scatter scaled an ERR-propagated flux error", () => {
    expect(row(measurement({ ...FILE_WEIGHTED, continuum_chi2_red: 3065.964 }), "Error source")).toBe(
      "FLUX_ERROR column (HDU 1), weights 1/ERR^2, flux error x 55.37 from the continuum scatter (chi2_red 3066)",
    );
    expect(row(measurement({ ...FILE_WEIGHTED, continuum_chi2_red: 0.93 }), "Error source")).toBe("FLUX_ERROR column (HDU 1), weights 1/ERR^2");
    expect(row(measurement({ ...FILE_WEIGHTED, continuum_chi2_red: null }), "Error source")).toBe("FLUX_ERROR column (HDU 1), weights 1/ERR^2");
  });

  it("counts the channels left out by DQ, non-finite flux and unusable ERR", () => {
    expect(row(measurement({ ...FILE_WEIGHTED, n_channels_nominal: undefined }), "Excluded channels")).toBe(
      "3 DQ-flagged, 2 non-finite flux, 58 ERR <= 0 or non-finite",
    );
    expect(row(measurement(), "Excluded channels")).toBe(
      "in line 0 of 25, in continuum 0 of 12: 0 DQ-flagged, 0 non-finite flux, 0 ERR <= 0 or non-finite",
    );
  });

  it("splits the excluded channels into the line and the continuum", () => {
    const split = measurement({ ...FILE_WEIGHTED, n_channels: 24, n_channels_nominal: 25, continuum_channels: 10 });
    expect(row(split, "Excluded channels")).toBe(
      "in line 1 of 25, in continuum 2 of 12: 3 DQ-flagged, 2 non-finite flux, 58 ERR <= 0 or non-finite",
    );
    const coinciding = measurement({
      ...FILE_WEIGHTED,
      n_channels: 24,
      n_channels_nominal: 25,
      continuum_channels: 5,
      continuum_windows: [
        [0, 5],
        [0, 5],
      ],
    });
    expect(row(coinciding, "Excluded channels")).toBe(
      "in line 1 of 25, in continuum 1 of 6: 3 DQ-flagged, 2 non-finite flux, 58 ERR <= 0 or non-finite",
    );
  });
});

const WITH_ERRORS: Partial<LineMeasurement> = {
  equivalent_width_err: 5.3e-4,
  centroid_err: 2.8e-4,
  sigma_err: 4.9e-4,
  fwhm_err: 1.16e-3,
  errors_reliable: true,
  width_errors_reliable: true,
  velocity: {
    ...measurement().velocity!,
    centroid_err_kms: 82.5,
    sigma_err_kms: 144.4,
    fwhm_err_kms: 340.1,
    source_centroid_err_kms: 82.5,
    source_sigma_err_kms: 144.4,
    source_fwhm_err_kms: 340.1,
    z_line: -1.3e-8,
    z_line_err: 2.75e-4,
  },
};

const PHYSICAL: Partial<LineMeasurement> = {
  flux_si: 7.84e-17,
  flux_si_err: 3.8e-18,
  flux_cgs: 7.84e-14,
  flux_cgs_err: 3.8e-15,
  flux_physical_basis: "F_nu in Jy integrated per channel with c / lambda^2",
};

const EW = "Equivalent width (absorption > 0, emission < 0)";
const SIGMA = "Sigma (2nd moment)";
const FWHM = "FWHM (2.355 x sigma)";
const OBSERVER_FRAME = "Observer frame (against the laboratory rest wavelength)";

describe("lineMeasurementRows with errors, physical flux, redshift and display units", () => {
  const row = (m: LineMeasurement, label: string) => lineMeasurementRows(m).find((r) => r.label === label)?.value;
  const labelsOf = (m: LineMeasurement) => lineMeasurementRows(m).map((r) => r.label);

  it("writes the flux with its error through the shared formatter", () => {
    expect(lineMeasurementRows(measurement())[0].value).toBe(fmt(7.52e-3, 5e-5, { unit: "Jy/beam x um" }));
    expect(row(measurement({ flux_err: null }), "Flux")).toBe("0.007520 ± n/a Jy/beam x um");
  });

  it("puts the errors beside equivalent width, centroid, sigma and FWHM, and beside their velocities", () => {
    const m = measurement(WITH_ERRORS);
    expect(row(m, EW)).toBe(fmt(-7.52e-3, 5.3e-4, { unit: "um" }));
    expect(row(m, "Centroid")).toBe(`${fmt(1.02, 2.8e-4, { unit: "um" })} / 0.2 ± 82.5 km/s`);
    expect(row(m, SIGMA)).toBe(`${fmt(0.003, 4.9e-4, { unit: "um" })} / 881.7 ± 144.4 km/s`);
    expect(row(m, FWHM)).toBe(`${fmt(0.00706, 1.16e-3, { unit: "um" })} / 2076.3 ± 340.1 km/s`);
  });

  it("keeps today's text for a result without the new errors", () => {
    const m = measurement();
    expect(row(m, EW)).toBe("-0.007520 um");
    expect(row(m, "Centroid")).toBe("1.02000 um / 0.2 km/s");
    expect(row(m, SIGMA)).toBe("0.003000 um / 881.7 km/s");
  });

  it("names sigma as the second moment and FWHM as derived from it", () => {
    const labels = labelsOf(measurement(WITH_ERRORS));
    expect(labels).toContain(SIGMA);
    expect(labels).toContain(FWHM);
    expect(labels).not.toContain("Sigma");
    expect(labels).not.toContain("FWHM");
  });

  it("shows the physical flux in both systems with its basis, or the reason it is absent", () => {
    const m = measurement(PHYSICAL);
    expect(row(m, "Physical flux")).toBe(
      `${fmt(7.84e-17, 3.8e-18, { unit: "W m^-2" })} = ${fmt(7.84e-14, 3.8e-15, { unit: "erg s^-1 cm^-2" })} (F_nu in Jy integrated per channel with c / lambda^2)`,
    );
    const labels = labelsOf(m);
    expect(labels.indexOf("Physical flux")).toBe(labels.indexOf("Flux") + 1);
    const reason = "no physical flux: Jy/beam is a brightness per beam and this version does not read the beam area";
    expect(
      row(measurement({ flux_si: null, flux_si_err: null, flux_cgs: null, flux_cgs_err: null, flux_physical_basis: reason }), "Physical flux"),
    ).toBe(reason);
    expect(row(measurement(), "Physical flux")).toBeUndefined();
    expect(row(measurement({ ...PHYSICAL, flux_physical_basis: "" }), "Physical flux")).toBeUndefined();
  });

  it("gives the peak its unit and its wavelength", () => {
    expect(row(measurement({ peak_axis: 1.02 }), "Peak")).toBe("1.000 Jy/beam at 1.02000 um (ch 20)");
    expect(row(measurement(), "Peak")).toBe("1.000 Jy/beam at ch 20");
    expect(row(measurement({ flux_unit: "native" }), "Peak")).toBe("1.000 at ch 20");
  });

  it("labels the centroid and the peak wavelength with the frame", () => {
    const m = measurement({ ...WITH_ERRORS, wavelength_frame: "air", peak_axis: 1.02 });
    expect(row(m, "Centroid")?.startsWith(fmt(1.02, 2.8e-4, { unit: "um air" }))).toBe(true);
    expect(row(m, "Peak")).toContain("at 1.02000 um air (ch 20)");
    expect(row(m, EW)).toBe(fmt(-7.52e-3, 5.3e-4, { unit: "um" }));
    expect(row(m, SIGMA)?.startsWith(fmt(0.003, 4.9e-4, { unit: "um" }))).toBe(true);
  });

  it("prints the signal-to-noise as a magnitude with the sense of the line", () => {
    expect(row(measurement(), "SNR")).toBe("150.4 (emission)");
    expect(row(measurement({ flux: -3.76e-3, snr: -21.4 }), "SNR")).toBe("21.4 (absorption)");
    expect(row(measurement({ snr: null }), "SNR")).toBe("n/a");
    expect(row(measurement({ flux: 0, snr: 0 }), "SNR")).toBe("0.0");
  });

  it("shows the redshift the line gives", () => {
    const velocity = WITH_ERRORS.velocity!;
    const m = measurement({ ...WITH_ERRORS, velocity: { ...velocity, z_line: 0.5000891, z_line_err: 4.57e-5 } });
    expect(row(m, "Line redshift")).toBe(`z = ${fmt(0.5000891, 4.57e-5)} (centroid over rest wavelength, minus 1)`);
    const labels = labelsOf(m);
    expect(labels.indexOf("Line redshift")).toBe(labels.indexOf("Velocity frame") + 1);
    const atRedshift = measurement({ ...WITH_ERRORS, velocity: { ...REDSHIFTED.velocity!, z_line: 0.5000891, z_line_err: 4.57e-5 } });
    const redshiftedLabels = labelsOf(atRedshift);
    expect(redshiftedLabels.indexOf("Line redshift")).toBe(redshiftedLabels.indexOf(OBSERVER_FRAME) + 1);
    expect(row(measurement({ ...WITH_ERRORS, velocity: null }), "Line redshift")).toBeUndefined();
    expect(row(measurement({ ...WITH_ERRORS, velocity: { ...velocity, z_line: null } }), "Line redshift")).toBeUndefined();
  });

  it("adds the fitted FWHM and the reduced chi2 to the Gaussian row and flags a poor fit", () => {
    const fit = measurement().fit!;
    const good = row(measurement(), "Gaussian fit") ?? "";
    expect(good).toContain("FWHM 0.007064 ± 2.355e-5 um");
    expect(good).toContain("chi2/dof 20.50/22 = 0.9318");
    expect(good).not.toContain("poor fit");
    expect(row(measurement({ fit: { ...fit, chi2: 456.16, dof: 15 } }), "Gaussian fit")).toContain("chi2/dof 456.2/15 = 30.41 (poor fit)");
    expect(456.16 / 15).toBeGreaterThan(POOR_FIT_CHI2_RED);
    const noDof = row(measurement({ fit: { ...fit, dof: 0 } }), "Gaussian fit") ?? "";
    expect(noDof).toContain("chi2/dof 20.50/0,");
    expect(noDof).not.toContain(" = ");
  });

  it("says when the errors are only indicative", () => {
    const reliability = (m: LineMeasurement) => row(m, "Error reliability");
    expect(reliability(measurement({ ...WITH_ERRORS, errors_reliable: false, snr: 5.135 }))).toBe(
      "line S/N 5.1 is below 10: the errors of equivalent width, centroid and widths are indicative",
    );
    expect(reliability(measurement({ ...WITH_ERRORS, errors_reliable: true, width_errors_reliable: false, sigma: 0.003, sigma_err: 4.9e-4 }))).toBe(
      "sigma is known to 16 percent: the errors of sigma and FWHM are indicative above 15 percent",
    );
    expect(reliability(measurement(WITH_ERRORS))).toBeUndefined();
    expect(reliability(measurement({ ...WITH_ERRORS, errors_reliable: undefined, width_errors_reliable: undefined }))).toBeUndefined();
    const noErrors = { equivalent_width_err: null, centroid_err: null, sigma_err: null, fwhm_err: null };
    expect(reliability(measurement({ ...WITH_ERRORS, ...noErrors, errors_reliable: false, snr: null }))).toBeUndefined();
    expect(reliability(measurement({ errors_reliable: false, snr: null }))).toBeUndefined();
    const withTerms = labelsOf(measurement({ ...WITH_ERRORS, ...FILE_WEIGHTED, errors_reliable: false, snr: 5.135 }));
    expect(withTerms.slice(-3)).toEqual(["Error terms", "Error reliability", "Excluded channels"]);
    const withoutTerms = labelsOf(measurement({ ...WITH_ERRORS, errors_reliable: false, snr: 5.135 }));
    expect(withoutTerms.slice(-3)).toEqual(["Error source", "Error reliability", "Excluded channels"]);
  });

  it("converts positions and widths to the display unit and leaves other axes alone", () => {
    const angstrom = measurement(WITH_ERRORS);
    const inA = (label: string) => lineMeasurementRows(angstrom, { axisDisplayUnit: "A" }).find((r) => r.label === label)?.value ?? "";
    expect(inA(EW)).toBe(fmt(-75.2, 5.3, { unit: "A" }));
    expect(inA("Centroid").startsWith(fmt(10200, 2.8, { unit: "A" }))).toBe(true);
    expect(inA(SIGMA).startsWith(fmt(30, 4.9, { unit: "A" }))).toBe(true);
    expect(inA("Gaussian fit")).toContain("s 30.00 ± 0.1000 A");
    expect(inA("Velocity frame")).toContain("rest 1.02000 um");
    const inNm = lineMeasurementRows(angstrom, { axisDisplayUnit: "nm" }).find((r) => r.label === "Centroid")?.value ?? "";
    expect(inNm.startsWith(fmt(1020, 0.28, { unit: "nm" }))).toBe(true);
    const ghz = measurement({ ...WITH_ERRORS, axis_unit: "GHz", flux_unit: "Jy x GHz" });
    const ghzRow = (label: string) => lineMeasurementRows(ghz, { axisDisplayUnit: "A" }).find((r) => r.label === label)?.value ?? "";
    expect(ghzRow(EW)).toBe(fmt(-7.52e-3, 5.3e-4, { unit: "GHz" }));
    expect(ghzRow("Centroid").startsWith(fmt(1.02, 2.8e-4, { unit: "GHz" }))).toBe(true);
  });

  it("keeps the order of the rows", () => {
    const full = measurement({
      ...WITH_ERRORS,
      ...FILE_WEIGHTED,
      ...PHYSICAL,
      errors_reliable: false,
      width_errors_reliable: false,
      velocity: { ...WITH_ERRORS.velocity!, ...REDSHIFTED.velocity!, z_line: 0.5000891, z_line_err: 4.57e-5 },
    });
    expect(labelsOf(full)).toEqual([
      "Flux",
      "Physical flux",
      EW,
      "Centroid",
      SIGMA,
      FWHM,
      "Peak",
      "SNR",
      "Continuum",
      "Velocity frame",
      OBSERVER_FRAME,
      "Line redshift",
      "Gaussian fit",
      "Error source",
      "Error terms",
      "Error reliability",
      "Excluded channels",
    ]);
  });
});

describe("line measurement CSV for a spectrum file", () => {
  const cellOf = (m: LineMeasurement, column: (typeof CSV_COLUMNS)[number]) =>
    lineMeasurementCsv(m).split("\n")[1].split(",")[CSV_COLUMNS.indexOf(column)];

  it("keeps the existing columns in place and appends the HDU and the error columns", () => {
    expect(CSV_COLUMNS.indexOf("fit_converged")).toBe(40);
    expect(CSV_COLUMNS.slice(41, 50)).toEqual([
      "source_hdu",
      "err_used",
      "err_source",
      "flux_err_pixel",
      "flux_err_level",
      "flux_err_slope",
      "n_dq_excluded",
      "n_nonfinite",
      "n_bad_err",
    ]);
  });

  it("writes the file source with its HDU and the ERR and exclusion figures", () => {
    const m = measurement(FILE_WEIGHTED);
    expect(lineMeasurementCsv(m).split("\n")[1].split(",")).toHaveLength(CSV_COLUMNS.length);
    expect(cellOf(m, "source_kind")).toBe("file");
    expect(cellOf(m, "source_x")).toBe("");
    expect(cellOf(m, "source_region")).toBe("");
    expect(cellOf(m, "source_hdu")).toBe("1");
    expect(cellOf(m, "err_used")).toBe("true");
    expect(cellOf(m, "err_source")).toBe("FLUX_ERROR column (HDU 1)");
    expect(cellOf(m, "flux_err_pixel")).toBe("0.00004");
    expect(cellOf(m, "flux_err_level")).toBe("0.000025");
    expect(cellOf(m, "flux_err_slope")).toBe("0.0000012");
    expect(cellOf(m, "n_dq_excluded")).toBe("3");
    expect(cellOf(m, "n_nonfinite")).toBe("2");
    expect(cellOf(m, "n_bad_err")).toBe("58");
  });

  it("leaves the HDU and the terms blank for a cube pixel without ERR", () => {
    const m = measurement();
    expect(cellOf(m, "source_hdu")).toBe("");
    expect(cellOf(m, "err_used")).toBe("false");
    expect(cellOf(m, "err_source")).toBe("none");
    expect(cellOf(m, "flux_err_pixel")).toBe("");
  });

  it("appends the redshift, the source-frame velocities, the continuum chi2 and the nominal channel count after the error columns", () => {
    expect(CSV_COLUMNS.slice(50, 56)).toEqual([
      "velocity_redshift",
      "velocity_source_centroid_kms",
      "velocity_source_sigma_kms",
      "velocity_source_fwhm_kms",
      "continuum_chi2_red",
      "n_channels_nominal",
    ]);
  });

  it("writes the redshift, the source-frame values, chi2_red and the nominal count, blank when absent", () => {
    const m = measurement({ ...REDSHIFTED, ...FILE_WEIGHTED, continuum_chi2_red: 3065.964, n_channels: 24, n_channels_nominal: 25 });
    expect(lineMeasurementCsv(m).split("\n")[1].split(",")).toHaveLength(CSV_COLUMNS.length);
    expect(cellOf(m, "velocity_redshift")).toBe("0.5");
    expect(cellOf(m, "velocity_source_centroid_kms")).toBe("0");
    expect(cellOf(m, "velocity_source_sigma_kms")).toBe("100");
    expect(cellOf(m, "velocity_source_fwhm_kms")).toBe("235.482");
    expect(cellOf(m, "velocity_centroid_kms")).toBe("149896.229");
    expect(cellOf(m, "continuum_chi2_red")).toBe("3065.964");
    expect(cellOf(m, "n_channels")).toBe("24");
    expect(cellOf(m, "n_channels_nominal")).toBe("25");
    const bare = measurement({ velocity: null });
    expect(cellOf(bare, "velocity_redshift")).toBe("");
    expect(cellOf(bare, "velocity_source_sigma_kms")).toBe("");
    expect(cellOf(bare, "continuum_chi2_red")).toBe("");
  });

  it("fills the flux error terms of the continuum-scatter path", () => {
    const m = measurement({ err_used: false, flux_err_terms: { pixel: 2.4e-4, level: 9.1e-5, slope: 3e-6 } });
    expect(cellOf(m, "flux_err_pixel")).toBe("0.00024");
    expect(cellOf(m, "flux_err_level")).toBe("0.000091");
    expect(cellOf(m, "flux_err_slope")).toBe("0.000003");
  });

  const IDENTIFIED: Partial<LineMeasurement> = {
    equivalent_width_err: 5.3e-4,
    centroid_err: 2.8e-4,
    errors_reliable: true,
    width_errors_reliable: false,
    peak_axis: 1.02,
    wavelength_frame: "vacuum",
    line_id: "h_alpha",
    line_label: "Hα",
    redshift: 0.5,
    flux_si: 7.84e-17,
    flux_si_err: 3.8e-18,
    flux_cgs: 7.84e-14,
    flux_cgs_err: 3.8e-15,
    flux_physical_basis: "F_nu in Jy integrated per channel with c / lambda^2",
  };
  const NEW_COLUMNS = [
    "equivalent_width_err",
    "centroid_err",
    "sigma_err",
    "fwhm_err",
    "errors_reliable",
    "width_errors_reliable",
    "velocity_centroid_err_kms",
    "velocity_sigma_err_kms",
    "velocity_fwhm_err_kms",
    "velocity_source_centroid_err_kms",
    "velocity_source_sigma_err_kms",
    "velocity_source_fwhm_err_kms",
    "velocity_z_line",
    "velocity_z_line_err",
    "peak_axis",
    "wavelength_frame",
    "line_id",
    "line_label",
    "redshift",
    "flux_si",
    "flux_si_err",
    "flux_cgs",
    "flux_cgs_err",
    "flux_physical_basis",
    "file",
    "dq",
  ] as const;

  it("appends the errors, the identity, the physical flux, the file and the DQ handling after the wave 0 columns", () => {
    expect(CSV_COLUMNS.slice(56, 82)).toEqual([...NEW_COLUMNS]);
    expect(CSV_COLUMNS.slice(50, 56)).toEqual([
      "velocity_redshift",
      "velocity_source_centroid_kms",
      "velocity_source_sigma_kms",
      "velocity_source_fwhm_kms",
      "continuum_chi2_red",
      "n_channels_nominal",
    ]);
    expect(CSV_COLUMNS.indexOf("fit_converged")).toBe(40);
  });

  it("writes the new cells and leaves them blank when absent", () => {
    const base = measurement();
    const m = measurement({
      ...IDENTIFIED,
      velocity: { ...base.velocity!, z_line: 0.5000891, z_line_err: 4.57e-5, source_sigma_err_kms: 144.4 },
    });
    expect(lineMeasurementCsv(m).split("\n")[1].split(",")).toHaveLength(CSV_COLUMNS.length);
    expect(cellOf(m, "equivalent_width_err")).toBe("0.00053");
    expect(cellOf(m, "centroid_err")).toBe("0.00028");
    expect(cellOf(m, "errors_reliable")).toBe("true");
    expect(cellOf(m, "width_errors_reliable")).toBe("false");
    expect(cellOf(m, "peak_axis")).toBe("1.02");
    expect(cellOf(m, "wavelength_frame")).toBe("vacuum");
    expect(cellOf(m, "line_id")).toBe("h_alpha");
    expect(cellOf(m, "line_label")).toBe("Hα");
    expect(cellOf(m, "redshift")).toBe("0.5");
    expect(cellOf(m, "flux_si")).toBe("7.84e-17");
    expect(cellOf(m, "flux_si_err")).toBe("3.8e-18");
    expect(cellOf(m, "flux_cgs")).toBe("7.84e-14");
    expect(cellOf(m, "flux_cgs_err")).toBe("3.8e-15");
    expect(cellOf(m, "flux_physical_basis")).toBe("F_nu in Jy integrated per channel with c / lambda^2");
    expect(cellOf(m, "velocity_z_line")).toBe("0.5000891");
    expect(cellOf(m, "velocity_z_line_err")).toBe("0.0000457");
    expect(cellOf(m, "velocity_source_sigma_err_kms")).toBe("144.4");
    const plain = measurement();
    expect(lineMeasurementCsv(plain).split("\n")[1].split(",")).toHaveLength(CSV_COLUMNS.length);
    expect(NEW_COLUMNS.map((column) => cellOf(plain, column))).toEqual(NEW_COLUMNS.map(() => ""));
  });

  it("writes the file and the DQ handling when the caller gives them", () => {
    const [header, row] = lineMeasurementCsv(measurement(), { fileName: "jw01_x1d.fits", dq: "excluded" }).split("\n");
    expect(header.endsWith(",file,dq,source_series")).toBe(true);
    expect(row.endsWith(",jw01_x1d.fits,excluded,")).toBe(true);
    const quoted = lineMeasurementCsv(measurement(), { fileName: "a,b.fits", dq: "off" }).split("\n")[1];
    expect(quoted.endsWith(',"a,b.fits",off,')).toBe(true);
  });

  it("starts with provenance and unit lines when asked", () => {
    const text = lineMeasurementCsv(measurement(), {
      fileName: "a.fits",
      dq: "off",
      provenance: { software: "AstroBurst v0.8.0", exportedUtc: "2026-11-10T03:00:00.000Z" },
    });
    const lines = text.split("\n");
    expect(lines.slice(0, 3)).toEqual(["# software: AstroBurst v0.8.0", "# exported_utc: 2026-11-10T03:00:00.000Z", "# file: a.fits"]);
    expect(lines).toContain("# unit flux: Jy/beam x um");
    expect(lines).toContain("# unit centroid: um");
    expect(lines).toContain("# unit velocity_centroid_kms: km/s");
    expect(lines).toContain("# unit flux_si: W m^-2");
    const headerIndex = lines.findIndex((line) => !line.startsWith("#"));
    expect(headerIndex).toBe(lines.length - 2);
    expect(lines[headerIndex]).toBe(CSV_COLUMNS.join(","));
    expect(lines[lines.length - 1].endsWith(",a.fits,off,")).toBe(true);
    expect(lineMeasurementCsv(measurement(), { fileName: "a.fits", dq: "off" }).split("\n")).toHaveLength(2);
  });

  it("omits the spectrum-unit lines when the flux unit has no axis factor", () => {
    const provenance = { software: "AstroBurst v0.8.0", exportedUtc: "2026-11-10T03:00:00.000Z" };
    const lines = lineMeasurementCsv(measurement({ flux_unit: "native" }), { provenance }).split("\n");
    expect(lines).toContain("# unit flux: native");
    expect(lines.some((line) => line.startsWith("# unit peak:"))).toBe(false);
    expect(lines.some((line) => line.startsWith("# unit continuum_slope:"))).toBe(false);
    expect(lineMeasurementCsv(measurement(), { provenance }).split("\n")).toContain("# unit peak: Jy/beam");
  });
});

describe("spectrum file runs", () => {
  it("keys a file source by its HDU so the overlay stays on the document it was measured on", () => {
    const run = { key: spectrumSourceKey({ kind: "file", hdu: 1 }, "sum") };
    expect(run.key).toBe('file:sum:{"kind":"file","hdu":1}');
    expect(runForSource(run, { kind: "file", hdu: 1 }, "sum")).toBe(run);
    expect(runForSource(run, { kind: "file", hdu: 2 }, "sum")).toBeNull();
    expect(runForSource(run, { kind: "pixel", x: 1, y: 1 }, "sum")).toBeNull();
  });
});

function csvCells(row: string): string[] {
  const cells: string[] = [];
  let cell = "";
  let quoted = false;
  for (let i = 0; i < row.length; i++) {
    const character = row[i];
    if (quoted && character === '"' && row[i + 1] === '"') {
      cell += '"';
      i++;
    } else if (character === '"') {
      quoted = !quoted;
    } else if (character === "," && !quoted) {
      cells.push(cell);
      cell = "";
    } else {
      cell += character;
    }
  }
  cells.push(cell);
  return cells;
}

describe("a spectrum picked from a time series", () => {
  const seriesOf = (detector: string, sporder: number | null, pick: SeriesSpectrumSource["pick"]): SeriesSpectrumSource => ({
    kind: "series",
    paths: ["a.fits"],
    detector,
    sporder,
    pick,
  });
  const MEAN = seriesOf("NRS1", 2, { kind: "mean", first: 3, last: 9 });

  it("labels a series pick with its detector and pick", () => {
    expect(spectrumSourceLabel(seriesOf("NRS2", -1, { kind: "median" }))).toBe("time series NRS2, median");
    expect(spectrumSourceLabel(MEAN)).toBe("time series NRS1, order 2, mean 4 to 10");
    expect(spectrumSourceLabel(seriesOf("NRS1", 0, { kind: "integration", index: 2 }))).toBe("time series NRS1, integration 3");
    expect(spectrumSourceLabel(seriesOf("MIRIMAGE", 1, { kind: "median" }))).toBe("time series MIRIMAGE, median");
    expect(spectrumSourceLabel(seriesOf("NIS", 1, { kind: "median" }))).toBe("time series NIS, order 1, median");
    expect(spectrumSourceLabel(seriesOf("NIS", null, { kind: "median" }))).toBe("time series NIS, median");
    expect(spectrumSourceLabel(seriesOf("NIS", 3, { kind: "integration", index: 0 }))).toBe("time series NIS, order 3, integration 1");
  });

  it("keys a series pick by its files, series and 0-based pick", () => {
    const key = spectrumSourceKey(MEAN, "sum");
    expect(key).toContain('"first":3,"last":9');
    expect(key).toBe('series:sum:{"kind":"series","paths":["a.fits"],"detector":"NRS1","sporder":2,"pick":{"kind":"mean","first":3,"last":9}}');
    const run = { key };
    expect(runForSource(run, MEAN, "sum")).toBe(run);
    expect(runForSource(run, seriesOf("NRS1", 2, { kind: "mean", first: 3, last: 8 }), "sum")).toBeNull();
    expect(runForSource(run, seriesOf("NRS1", 2, { kind: "median" }), "sum")).toBeNull();
    expect(runForSource(run, { kind: "file", hdu: null }, "sum")).toBeNull();
  });

  it("appends the series column after the columns of the file and the DQ handling", () => {
    expect(CSV_COLUMNS[82]).toBe("source_series");
    expect(CSV_COLUMNS).toHaveLength(83);
  });

  it("writes the series pick into the CSV row as sent, counted from 0", () => {
    const m = measurement({ source: MEAN, err_used: true, err_source: "FLUX_ERROR of the series pick" });
    const [header, row] = lineMeasurementCsv(m, { fileName: "a.fits", dq: "excluded" }).split("\n");
    const cells = csvCells(row);
    expect(cells).toHaveLength(header.split(",").length);
    expect(cells).toHaveLength(CSV_COLUMNS.length);
    expect(cells[CSV_COLUMNS.indexOf("source_kind")]).toBe("series");
    expect(cells[CSV_COLUMNS.indexOf("source_x")]).toBe("");
    expect(cells[CSV_COLUMNS.indexOf("source_region")]).toBe("");
    expect(cells[CSV_COLUMNS.indexOf("source_hdu")]).toBe("");
    expect(cells[CSV_COLUMNS.indexOf("err_source")]).toBe("FLUX_ERROR of the series pick");
    expect(cells[CSV_COLUMNS.indexOf("dq")]).toBe("excluded");
    expect(cells[cells.length - 1]).toBe('{"paths":["a.fits"],"detector":"NRS1","sporder":2,"pick":{"kind":"mean","first":3,"last":9}}');
    expect(header.endsWith(",file,dq,source_series")).toBe(true);
    expect(JSON.parse(cells[CSV_COLUMNS.indexOf("source_series")])).toEqual({
      paths: ["a.fits"],
      detector: "NRS1",
      sporder: 2,
      pick: { kind: "mean", first: 3, last: 9 },
    });
  });

  it("leaves the series column empty for the other sources", () => {
    const pixel = csvCells(lineMeasurementCsv(measurement()).split("\n")[1]);
    expect(pixel).toHaveLength(CSV_COLUMNS.length);
    expect(pixel[CSV_COLUMNS.indexOf("source_series")]).toBe("");
    const file = csvCells(lineMeasurementCsv(measurement({ source: { kind: "file", hdu: 1 } })).split("\n")[1]);
    expect(file[CSV_COLUMNS.indexOf("source_series")]).toBe("");
  });

  it("names the series pick in the provenance lines of the copied row", () => {
    const text = lineMeasurementCsv(measurement({ source: MEAN }), {
      fileName: "a.fits",
      dq: "excluded",
      provenance: { software: "AstroBurst v0.8.1", exportedUtc: "2026-11-13T03:00:00.000Z" },
    });
    expect(text.split("\n")).toContain("# source: time series NRS1, order 2, mean 4 to 10");
  });
});

describe("lineMeasurementAxis", () => {
  const AIR_VALUES = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);
  const airAxis: SpectralAxisInfo = {
    kind: "awav",
    ctype: "LINEAR",
    unit: "um",
    header_unit: "angstroms",
    header_scale: 1e-4,
    values: AIR_VALUES,
    crval: AIR_VALUES[0],
    cdelt: 1e-4,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
  };
  const airRun = measurement({
    source: { kind: "file", hdu: 0 },
    z0: 10,
    z1: 30,
    continuum_reference: AIR_VALUES[20],
    continuum_slope: 0,
    fit: { ...measurement().fit!, centre: AIR_VALUES[20], sigma: 2e-4 },
  });

  it("keeps the native axis of a spectrum file, where the backend measured, and converts a cube's air axis", () => {
    expect(lineMeasurementAxis(airAxis, true)).toBe(AIR_VALUES);
    expect(lineMeasurementAxis(null, true)).toBeNull();
    expect(lineMeasurementAxis(airAxis, false)![20]).toBeCloseTo(airToVacuumUm(AIR_VALUES[20]), 12);
  });

  it("draws the model peak of an air document at the native channel of fit.centre on the vacuum display", () => {
    const display = formatAxis(airAxis, "wavelength_vac", null, "optical")!.values;
    const m = mapping(display);
    const peakOf = (axis: number[]) => {
      const { model } = lineOverlayPolylines(airRun, axis, m, FRAME);
      return model.reduce((best, p) => (p.y < best.y ? p : best), model[0]);
    };
    const expected = axisValueToPixel(channelAxisValue(20, m), m);
    expect(Math.abs(peakOf(lineMeasurementAxis(airAxis, true)!).x - expected)).toBeLessThan(0.5);
    expect(Math.abs(peakOf(measurementAxisValues(airAxis)!).x - expected)).toBeGreaterThan(5);
  });
});
