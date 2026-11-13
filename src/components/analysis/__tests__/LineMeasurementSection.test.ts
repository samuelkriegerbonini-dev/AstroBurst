import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("../LineRedshiftSummary", async () => {
  const react = await import("react");
  return {
    default: ({ fileName, source }: { fileName: string | null; source: string | null }) =>
      react.createElement("i", { "data-redshift-summary": true, "data-file": fileName, "data-source": source }),
  };
});

import LineMeasurementSection from "../LineMeasurementSection";
import lineMeasurementSectionSource from "../LineMeasurementSection.tsx?raw";
import { SMALL_BUTTON_CLASS } from "../../ui";
import { adHocErrorBoxLines } from "../../../test/panelRules";
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

describe("LineMeasurementSection channel count", () => {
  it("prints the measured channels out of the nominal line range", () => {
    const html = render({ result: { ...RESULT, n_channels: 26, n_channels_nominal: 26 } });
    expect(html).toContain("26 of 26 channels");
    expect(html).not.toContain("data-dropped-channels");
  });

  it("marks a line range with dropped channels as a warning, since the flux is then a partial sum", () => {
    const html = render({ result: { ...RESULT, n_channels: 24, n_channels_nominal: 26 } });
    expect(html).toContain("24 of 26 channels");
    expect(html).toMatch(/<span[^>]*data-dropped-channels="2"[^>]*>24 of 26 channels<\/span>/);
    expect(html).toContain("text-amber-300/80");
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

describe("LineMeasurementSection identity, failure, measured series and display unit", () => {
  it("names the line and the requested redshift in the footer", () => {
    expect(render({ result: { ...RESULT, line_label: "Hα", line_id: "h_alpha", redshift: 0.5 } })).toContain(
      "Hα at z = 0.5, spectrum file, HDU 1, ch 5-30",
    );
    expect(render({ result: { ...RESULT, line_id: "h_alpha" } })).toContain("h_alpha, spectrum file");
    expect(render()).toContain("<span>spectrum file, HDU 1, ch 5-30");
    const atZero = render({ result: { ...RESULT, line_label: "Hα", redshift: 0 } });
    expect(atZero).toContain("Hα, spectrum file, HDU 1, ch 5-30");
    expect(atZero).not.toContain("at z =");
    const withoutRedshift = render({ result: { ...RESULT, line_label: "Hα" } });
    expect(withoutRedshift).toContain("Hα, spectrum file, HDU 1, ch 5-30");
    expect(withoutRedshift).not.toContain("at z =");
  });

  it("announces a failed measurement", () => {
    expect(render({ error: "line range 5..=4 is invalid" })).toMatch(/role="alert"[^>]*>line range 5\.\.=4 is invalid</);
  });

  it("shows which series was measured when the panel says so", () => {
    const note = "measures the sum series (MJy/sr x pix)";
    expect(render({ measuredSeriesNote: note }).split(note)).toHaveLength(2);
    expect(render()).not.toContain("measures the sum series");
  });

  it("passes the display unit to the rows", () => {
    expect(render({ axisDisplayUnit: "A" })).toMatch(/Equivalent width \(absorption &gt; 0, emission &lt; 0\)<\/div><div[^>]*>[^<]* A<\/div>/);
    expect(render()).toMatch(/Equivalent width \(absorption &gt; 0, emission &lt; 0\)<\/div><div[^>]*>[^<]* um<\/div>/);
  });
});

describe("LineMeasurementSection redshift summary and copy", () => {
  it("mounts the redshift summary only when the panel passes the file, with the source of the result", () => {
    const html = render({ filePath: "C:/d/a.fits#hdu=1" });
    expect(html).toContain("data-redshift-summary");
    expect(html).toContain('data-file="a.fits"');
    expect(html).toContain(`data-source="${JSON.stringify(RESULT.source).replace(/"/g, "&quot;")}"`);
    expect(render()).not.toContain("data-redshift-summary");
    expect(render({ filePath: null })).not.toContain("data-redshift-summary");
    expect(render({ filePath: "C:/d/a.fits#hdu=1", result: null })).not.toContain("data-redshift-summary");
  });

  it("titles the copy button with what the row carries", () => {
    expect(render()).toContain('title="Copy the measurement as one CSV row with its units and provenance"');
  });
});

describe("LineMeasurementSection sweep rules", () => {
  it("holds no hand-made failure box", () => {
    expect(adHocErrorBoxLines(lineMeasurementSectionSource)).toEqual([]);
  });

  it("announces a failed measurement through the compact alert", () => {
    expect(render({ result: null, error: "no continuum channel is left outside the line" })).toContain(
      '<div class="ab-error-alert ab-error-alert-compact" role="alert">no continuum channel is left outside the line</div>',
    );
  });

  it("copies the line through the shared button", () => {
    const html = render();
    const button = /<button([^>]*)>((?:(?!<\/button>).)*)<\/button>(<span[^>]*>[^<]*<\/span>)?/.exec(
      html.slice(html.lastIndexOf("<button", html.indexOf('title="Copy the measurement as one CSV row with its units and provenance"'))),
    );
    expect(button?.[1]).toContain('title="Copy the measurement as one CSV row with its units and provenance"');
    expect(button?.[1]).toContain(`class="ml-auto ${SMALL_BUTTON_CLASS}"`);
    expect(button?.[2]).toMatch(/Copy CSV$/);
    expect(button?.[3]).toBe('<span role="status" class="sr-only" data-copy-status=""></span>');
  });

  it("shows no copy button before a measurement", () => {
    expect(render({ result: null })).not.toContain("Copy CSV");
  });
});
