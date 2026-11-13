import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import LineFitSection from "../LineFitSection";
import lineFitSectionSource from "../LineFitSection.tsx?raw";
import { adHocErrorBoxLines } from "../../../test/panelRules";
import type { LineFitPlane, LineFitResult, LineFitSpaxel } from "../../../shared/types/cube";
import type { LineFitRun } from "../../../utils/lineFit";

vi.mock("../LineFitInspectPlot", async () => {
  const react = await import("react");
  return { default: (props: { redshift?: number | null }) => react.createElement("span", { "data-inspect-redshift": String(props.redshift) }) };
});

const ORDER: LineFitPlane[] = ["flux", "velocity", "sigma_obs", "flux_err", "v_err", "sigma_err", "chi2_red", "snr", "mask", "ncomp"];
const SOURCE_PLANES: LineFitPlane[] = ["source_flux", "source_velocity", "source_sigma_obs", "source_flux_err", "source_v_err", "source_sigma_err"];
const fitsOf = (plane: string) => `C:/out/a_linefit_${plane}_530-560.fits`;

function result(): LineFitResult {
  return {
    planes: Object.fromEntries(ORDER.map((p) => [p, { png_path: `C:/out/a_linefit_${p}_530-560.png`, fits_path: fitsOf(p), previewUrl: `asset://${p}.png` }])),
    plane_order: ORDER,
    units: { flux: "MJy/sr km/s", velocity: "km/s", sigma: "km/s" },
    sigma_label: "observed (instrumental width not removed)",
    weighting: "err",
    err_hdu: 2,
    dq_hdu: 3,
    z0: 530,
    z1: 560,
    n_channels: 31,
    rest_um: 1.875613,
    convention: "optical",
    components: "one",
    continuum_windows: [
      [500, 520],
      [568, 588],
    ],
    resolving_power: null,
    snr_threshold: 3,
    n_fit: 1310,
    n_masked: 64,
    n_const_continuum: 2,
    n_two_components: 0,
    median_chi2_red: 1.9,
    notes: [],
    dimensions: [53, 55],
    elapsed_ms: 1800,
  };
}

const RUN: LineFitRun = {
  key: "k1",
  filePath: "C:/d/a_s3d.fits",
  config: {
    z0: 530,
    z1: 560,
    rest_um: 1.875613,
    convention: "optical",
    continuum: [
      [500, 520],
      [568, 588],
    ],
    snr_threshold: 3,
    emission_only: true,
    use_err: true,
    use_dq: true,
    resolving_power: null,
    components: "one",
  },
  result: result(),
  lineLabel: "Paα",
};

function atZResult(): LineFitResult {
  const plain = result();
  const sourcePlanes = SOURCE_PLANES.map((p) => [p, { png_path: `C:/out/a_linefit_${p}_530-560.png`, fits_path: fitsOf(p), previewUrl: `asset://${p}.png` }]);
  return { ...plain, plane_order: [...ORDER, ...SOURCE_PLANES], planes: { ...plain.planes, ...Object.fromEntries(sourcePlanes) } };
}

const AT_Z_RUN: LineFitRun = { ...RUN, config: { ...RUN.config, redshift: 0.5 }, result: atZResult() };

const AT_Z_SPAXEL: LineFitSpaxel = {
  x: 18,
  y: 32,
  z0: 1,
  z1: 1,
  continuum_windows: [
    [0, 0],
    [2, 2],
  ],
  span: [0, 2],
  axis: [1, 2, 3],
  axis_unit: "um",
  flux: [1, 3, 1],
  err: null,
  channels: [1],
  dropped_dq: [7, 8],
  dropped_err: [],
  continuum: { intercept: 1, slope: 0, x_ref: 2, sigma: 0.05, linear: true, channels: 2 },
  weighting: "err",
  single: {
    amplitude: 2.5,
    centre: 1.876,
    sigma: 0.0006,
    amplitude_err: 0.1,
    centre_err: 0.00001,
    sigma_err: 0.00002,
    velocity_kms: 149914.68,
    v_err_kms: 1.5,
    sigma_kms: 150.0,
    sigma_err_kms: 3.0,
    sigma_corr_kms: null,
    flux: 939.99,
    flux_err: 18.75,
    snr: 50.13,
    source_velocity_kms: 12.3,
    source_v_err_kms: 1.0,
    source_sigma_kms: 100.0,
    source_sigma_err_kms: 2.0,
    source_sigma_corr_kms: null,
    source_flux: 626.66,
    source_flux_err: 12.5,
  },
  chi2: 30.5,
  dof: 28,
  chi2_red: 1.089,
  converged: true,
  iterations: 9,
  components: [],
  ncomp: 1,
  delta_bic: null,
  mask: 1,
  model: {
    channel: [0, 0.5, 1, 1.5, 2],
    continuum: [1, 1, 1, 1, 1],
    total: [1, 2, 3, 2, 1],
    components: [[0, 1, 2, 1, 0]],
  },
  notes: [],
};

const SOURCE_FRAME_CAPTION =
  "source frame at z = 0.5: the planes marked (source frame) give V relative to z, and σ and flux in the rest frame of the source; the other planes are observer-frame values (see the redshift note)";

interface SectionOverrides {
  error?: string | null;
  inspectError?: string | null;
  redshift?: number | null;
  inspect?: LineFitSpaxel | null;
}

function render(run: LineFitRun | null, shownPlane: LineFitPlane | null, stale = false, overrides: SectionOverrides = {}): string {
  const noop = () => {};
  return renderToStaticMarkup(
    createElement(LineFitSection, {
      ready: true,
      readyHint: null,
      loading: false,
      error: null,
      errAvailable: true,
      dqAvailable: true,
      useErr: true,
      onUseErr: noop,
      useDq: true,
      onUseDq: noop,
      emissionOnly: true,
      onEmissionOnly: noop,
      snrText: "3",
      onSnrText: noop,
      resolvingPowerText: "",
      onResolvingPowerText: noop,
      components: "one",
      onComponents: noop,
      onFit: noop,
      run,
      shownPlane,
      onShowPlane: noop,
      inspect: null,
      inspectLoading: false,
      inspectError: null,
      frameNote: "velocities in the file's frame",
      stale,
      ...overrides,
    }),
  );
}

describe("LineFitSection output note", () => {
  it("exposes the completed map manifest and its source hash without requiring a selected plane", () => {
    const run = { ...RUN, result: { ...result(), provenance_path: "C:/out/fit/provenance.json", provenance: {
      source: { input_ref: "C:/data/cube.fits", sha256: "abcdef0123456789".repeat(4) },
      integrity: "sha256_before_open_and_before_publication",
    } } };
    const html = render(run, null);
    expect(html).toContain('data-output-path="C:/out/fit/provenance.json"');
    expect(html).toContain("does not copy this manifest");
    expect(html).toContain("Input SHA-256: abcdef012345…");
    expect(render(null, null)).not.toContain("does not copy this manifest");
  });

  it("names the map on screen as an output-folder file that is cleared at launch, with Show in folder", () => {
    const html = render(RUN, "flux");
    expect(html).toContain(`data-output-path="${fitsOf("flux")}"`);
    expect(html).toContain(">a_linefit_flux_530-560.fits<");
    expect(html).toContain("cleared when AstroBurst starts");
    expect(html).toContain("Show in folder");
  });

  it("names the velocity map, which a fit shows first, while no plane of the run is on screen", () => {
    expect(render(RUN, null)).toContain(`data-output-path="${fitsOf("velocity")}"`);
  });

  it("shows no note before a fit", () => {
    expect(render(null, null)).not.toContain("data-output-note");
  });
});

describe("LineFitSection stale note", () => {
  it("shows the stale note once, at the top of the fit result above its DQ policy, numbers and manifest", () => {
    const html = render(RUN, null, true);
    const note = html.indexOf("data-stale-result");
    expect(html.split("data-stale-result")).toHaveLength(2);
    expect(note).toBeGreaterThan(html.indexOf('aria-label="Fit lines"'));
    expect(note).toBeLessThan(html.indexOf('data-dq-policy="fit"'));
  });

  it("shows no stale note for a current fit or before a fit", () => {
    expect(render(RUN, null)).not.toContain("data-stale-result");
    expect(render(null, null, true)).not.toContain("data-stale-result");
  });
});

describe("LineFitSection errors", () => {
  const elementWith = (html: string, attribute: string) => new RegExp(`<p[^>]*${attribute}[^>]*>([^<]*)</p>`).exec(html);

  it("announces a failed fit and a failed spaxel inspection", () => {
    const fitError = "line-fit window 3..=5 has 3 channels: at least 5 are needed";
    const failed = render(null, null, false, { error: fitError });
    const fit = elementWith(failed, "data-linefit-error");
    expect(failed.split("data-linefit-error")).toHaveLength(2);
    expect(fit?.[0]).toContain('role="alert"');
    expect(fit?.[1]).toBe(fitError);
    const inspectError = "pixel (60, 2) is outside the 53x55 cube";
    const inspect = elementWith(render(RUN, null, false, { inspectError }), "data-linefit-inspect-error");
    expect(inspect?.[0]).toContain('role="alert"');
    expect(inspect?.[1]).toBe(inspectError);
    expect(render(null, null)).not.toContain('role="alert"');
  });
});

describe("LineFitSection at a redshift", () => {
  it("offers the source-frame planes and names the frame of the maps", () => {
    const html = render(AT_Z_RUN, null);
    expect(html).toMatch(/<button data-linefit-plane="source_velocity"[^>]*title="source_velocity \(km\/s\)"[^>]*>V \(source frame\)<\/button>/);
    expect(html.split("data-linefit-source-frame")).toHaveLength(2);
    expect(html).toContain(`>${SOURCE_FRAME_CAPTION}</span>`);
    expect(html.indexOf("data-linefit-source-frame")).toBeGreaterThan(html.indexOf("data-linefit-frame"));
    expect(html).toContain(`data-output-path="${fitsOf("source_velocity")}"`);
    const plain = render(RUN, null);
    expect(plain).not.toContain("data-linefit-source-frame");
    expect(plain).toContain(`data-output-path="${fitsOf("velocity")}"`);
    const ignored = render({ ...RUN, config: { ...RUN.config, redshift: 0.5 } }, null);
    expect(ignored).toContain(">redshift 0.5 was not applied: this fit has no source-frame planes (see the notes)</span>");
  });

  it("says before a fit that it will run at the redshift of the line list", () => {
    const hinted = render(null, null, false, { redshift: 0.5 });
    expect(hinted.split("data-linefit-redshift")).toHaveLength(2);
    expect(hinted).toContain(">fits at z = 0.5 (Redshift z of the line list) and adds source-frame maps<");
    expect(hinted.indexOf("data-linefit-redshift")).toBeGreaterThan(hinted.indexOf('aria-label="Fit lines"'));
    for (const redshift of [0, null]) expect(render(null, null, false, { redshift })).not.toContain("data-linefit-redshift");
    expect(render(null, null)).not.toContain("data-linefit-redshift");
  });

  it("passes the redshift of the run to the spaxel rows and to the inspect plot", () => {
    const html = render(AT_Z_RUN, null, false, { inspect: AT_Z_SPAXEL });
    expect(html).toContain('data-inspect-redshift="0.5"');
    expect(html).toContain(">source frame at z = 0.5</div>");
    expect(html).toContain("Observer frame");
    const plain = render(RUN, null, false, { inspect: AT_Z_SPAXEL });
    expect(plain).toContain('data-inspect-redshift="null"');
    expect(plain).not.toContain("Observer frame");
  });
});

describe("LineFitSection sweep rules", () => {
  it("holds no hand-made failure box", () => {
    expect(adHocErrorBoxLines(lineFitSectionSource)).toEqual([]);
  });

  it("announces a failed fit and a failed inspection through the compact alert, on the elements tests can find", () => {
    expect(render(null, null, false, { error: "cube_line_fit_cmd: no channel inside the range" })).toContain(
      '<p data-linefit-error="true" role="alert" class="ab-error-alert ab-error-alert-compact">cube_line_fit_cmd: no channel inside the range</p>',
    );
    expect(render(RUN, null, false, { inspectError: "spaxel (3, 4) lies outside the fitted region" })).toContain(
      '<p data-linefit-inspect-error="true" role="alert" class="ab-error-alert ab-error-alert-compact">spaxel (3, 4) lies outside the fitted region</p>',
    );
  });
});
