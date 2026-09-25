import { describe, it, expect } from "vitest";
import {
  CSV_COLUMNS,
  continuumModel,
  convergedFit,
  formatQuantity,
  gaussianModel,
  lineMeasurementCsv,
  lineMeasurementRows,
  lineOverlayPolylines,
  measurementAxisValues,
  measurementSpan,
  spectrumSourceKey,
  type PlotFrame,
} from "../lineMeasure";
import { airToVacuumUm } from "../spectralAxis";
import { axisValueToPixel, type PlotMapping } from "../spectrumRange";
import type { LineMeasurement, SpectralAxisInfo } from "../../shared/types/spectral";

const AXIS = Array.from({ length: 40 }, (_, i) => 1.0 + i * 0.001);

function measurement(overrides: Partial<LineMeasurement> = {}): LineMeasurement {
  return {
    z0: 8,
    z1: 32,
    n_channels: 25,
    axis_unit: "um",
    flux_unit: "Jy/beam x um",
    continuum_level: 1.0,
    continuum_slope: 0.0,
    continuum_sigma: 0.01,
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
});

describe("lineMeasurementRows", () => {
  it("lists the metrics with their units and velocities", () => {
    const rows = lineMeasurementRows(measurement());
    const labels = rows.map((r) => r.label);
    expect(labels).toEqual([
      "Flux",
      "Equivalent width",
      "Centroid",
      "Sigma",
      "FWHM",
      "Peak",
      "SNR",
      "Continuum",
      "Velocity frame",
      "Gaussian fit",
    ]);
    expect(rows[0].value).toBe("0.007520 ± 5.000e-5 Jy/beam x um");
    expect(rows[2].value).toBe("1.02000 um / 0.2 km/s");
    expect(rows[5].value).toBe("1.000 at ch 20");
    expect(rows[6].value).toBe("150.4");
    expect(rows[8].value).toBe("optical, rest 1.02000 um, topocentric");
  });

  it("marks unavailable values and drops the velocity and fit rows when absent", () => {
    const rows = lineMeasurementRows(measurement({ velocity: null, fit: null, sigma: null, snr: null }));
    expect(rows.map((r) => r.label)).not.toContain("Gaussian fit");
    expect(rows.find((r) => r.label === "Sigma")?.value).toBe("n/a um");
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
