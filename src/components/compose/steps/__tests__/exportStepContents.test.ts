import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("../../../../context/CompositeContext", () => {
  const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
  return { useCompositeStf: () => ({ wizardStfR: stf, wizardStfG: stf, wizardStfB: stf, wizardStfLinked: true }) };
});

import ExportStep from "../ExportStep";
import { applyCompositeOp, EMPTY_COMPOSITE_HISTORY, INITIAL_STATE, type WizardState } from "../../../../utils/wizard";

const DECONV = "Deconvolution RL: 20 iterations, PSF gaussian sigma 2 px size 15";

function composite(): WizardState {
  const history = [
    { kind: "blend", preset: "sho" },
    { kind: "starRemoval", sigma: 4, growth: 3 },
    { kind: "chain", lines: [DECONV], restarted: false },
  ] as const;
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: ["ha", "oiii", "sii"].includes(b.id) ? [`/${b.id}.fits`] : [] })),
    compositeReady: true,
    compositeHistory: history.reduce(applyCompositeOp, EMPTY_COMPOSITE_HISTORY),
  };
}

describe("Export banner on the composite", () => {
  it("lists what the FITS really holds instead of a fixed sentence", () => {
    const html = renderToStaticMarkup(createElement(ExportStep, { state: composite() }));
    expect(html).not.toContain("WB + SCNR only");
    expect(html).toContain(
      `FITS holds the linear composite, written to HISTORY as: Blend: sho; Star removal: 4.0 sigma, growth 3.00x FWHM; ${DECONV}.`,
    );
  });
});
