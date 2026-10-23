import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import LineFitSection from "../LineFitSection";
import type { LineFitPlane, LineFitResult } from "../../../shared/types/cube";
import type { LineFitRun } from "../../../utils/lineFit";

const ORDER: LineFitPlane[] = ["flux", "velocity", "sigma_obs", "flux_err", "v_err", "sigma_err", "chi2_red", "snr", "mask", "ncomp"];
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

function render(run: LineFitRun | null, shownPlane: LineFitPlane | null): string {
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
    }),
  );
}

describe("LineFitSection output note", () => {
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
