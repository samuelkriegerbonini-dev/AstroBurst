import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import LineMeasurementSection from "../LineMeasurementSection";
import { LINE_MODEL_LABELS, LINE_MODELS, type LineMeasurement } from "../../../shared/types/spectral";

const RESULT: LineMeasurement = {
  z0: 5,
  z1: 30,
  n_channels: 26,
  axis_unit: "um",
  flux_unit: "Jy x um",
  continuum_level: 1,
  continuum_slope: 0,
  continuum_sigma: 0.01,
  continuum_reference: 4.05,
  continuum_linear: true,
  continuum_channels: 10,
  continuum_windows: [
    [0, 4],
    [31, 35],
  ],
  flux: 1e-4,
  flux_err: 1e-6,
  equivalent_width: -1e-4,
  centroid: 4.052,
  sigma: 2e-4,
  fwhm: 4.7e-4,
  peak: 1,
  peak_channel: 18,
  snr: 100,
  velocity: null,
  fit: null,
  err_used: true,
  flux_err_terms: { pixel: 1e-6, level: 2e-7, slope: 1e-8 },
  n_dq_excluded: 3,
  n_nonfinite: 2,
  n_bad_err: 58,
  err_source: "FLUX_ERROR column (HDU 1)",
  notes: [],
  elapsed_ms: 1,
  source: { kind: "file", hdu: 1 },
};

function render(extra: Record<string, unknown> = {}): string {
  return renderToStaticMarkup(
    createElement(LineMeasurementSection, {
      result: RESULT,
      loading: false,
      error: null,
      model: "gaussian",
      onModelChange: () => {},
      onMeasure: () => {},
      disabled: false,
      disabledHint: null,
      ...extra,
    }),
  );
}

const CONTROLS = { useErr: true, onUseErr: () => {}, useDq: true, onUseDq: () => {}, errAvailable: true, dqAvailable: true };

describe("LineMeasurementSection error rows and switches", () => {
  it("shows the ERR source, the three error terms and the excluded channel counts", () => {
    const html = render();
    expect(html).toContain("Error source");
    expect(html).toContain("FLUX_ERROR column (HDU 1), weights 1/ERR^2");
    expect(html).toContain("Error terms");
    expect(html).toContain("3 DQ-flagged, 2 non-finite flux, 58 ERR &lt;= 0 or non-finite");
    expect(html).toContain("spectrum file, HDU 1, ch 5-30");
  });

  it("offers the ERR weighting and DQ masking switches only when the panel passes them", () => {
    expect(render()).not.toContain("Weight by ERR");
    const html = render({ errControls: CONTROLS });
    expect(html).toContain('aria-label="Weight by ERR"');
    expect(html).toContain('aria-label="Mask DQ channels"');
    expect(html).not.toContain("no ERR in this file");
  });

  it("says which companion the file lacks", () => {
    const html = render({ errControls: { ...CONTROLS, errAvailable: false, dqAvailable: false } });
    expect(html).toContain("no ERR in this file");
    expect(html).toContain("no DQ in this file");
  });
});

describe("LineMeasurementSection model names and equivalent-width sign", () => {
  it("names the models by what they compute and keeps the logged values", () => {
    expect(LINE_MODEL_LABELS).toEqual({ none: "direct integration", gaussian: "direct + Gaussian fit" });
    expect(LINE_MODELS).toEqual(["none", "gaussian"]);
  });

  it("shows the model names in the select with the values none and gaussian", () => {
    const html = render({ model: "none" });
    expect(html).toMatch(/<option value="none"[^>]*>direct integration<\/option>/);
    expect(html).toMatch(/<option value="gaussian"[^>]*>direct \+ Gaussian fit<\/option>/);
  });

  it("labels the equivalent-width card with its sign convention", () => {
    expect(render()).toContain("Equivalent width (absorption &gt; 0, emission &lt; 0)");
  });
});
