import { afterEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import type { ChainStep } from "../../../shared/types/preview";
import type { CompositeStepResult, DisplayStf } from "../../../shared/types/compositeChain";

type CompositeDone = (step: ChainStep, label: string, result: CompositeStepResult, callStf: DisplayStf, history?: readonly string[]) => void;

const { captured } = vi.hoisted(() => ({ captured: { onCompositeDone: null as CompositeDone | null } }));

vi.mock("../../../hooks/useCompositeMode", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useCompositeMode")>()),
  useCompositeMode: () => true,
}));

vi.mock("../DeconvolutionPanel", () => ({
  default: (props: { onCompositeDone: CompositeDone }) => {
    captured.onCompositeDone = props.onCompositeDone;
    return null;
  },
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider, useComposeWizardContext } from "../../../context/ComposeWizardContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import { compositeChainStore } from "../../../utils/compositeChainStore";
import { compositeHistoryLines, INITIAL_STATE } from "../../../utils/wizard";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";

const FILE: ProcessedFile = {
  id: "composite-history",
  name: "673nmos.fits",
  path: "C:/data/673nmos.fits",
  sourcePath: "C:/data/673nmos.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: { png_path: "C:/out/673nmos.png", previewUrl: "asset://C:/out/673nmos.png", dimensions: [64, 64], elapsed_ms: 1, is_rgb: false },
  error: null,
  startedAt: null,
  finishedAt: null,
};

const STF = { shadow: 0, midtone: 0.5, highlight: 1 };
const DISPLAY: DisplayStf = { r: STF, g: STF, b: STF, linked: true };

type WizardContextValue = ReturnType<typeof useComposeWizardContext>;

async function renderTab(): Promise<WizardContextValue> {
  const seen: WizardContextValue[] = [];
  function Capture(): null {
    seen.push(useComposeWizardContext());
    return null;
  }
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: [
        createElement(Capture, { key: "capture" }),
        createElement(PreviewProvider, { key: "preview", file: FILE, doneFiles: [FILE], children: createElement(ProcessingTab) }),
      ],
    }),
  });
  await prerender(tree);
  return seen[0];
}

function deconvResult(restarted: boolean): CompositeStepResult {
  return {
    png_path: "C:/out/composite_deconv.png",
    base_png_path: restarted ? "C:/out/composite_base.png" : null,
    previewUrl: "asset://C:/out/composite_deconv.png",
    basePreviewUrl: restarted ? "asset://C:/out/composite_base.png" : undefined,
    dimensions: [64, 64],
    elapsed_ms: 12,
    chain_generation: 7,
    chain_restarted: restarted,
    chain_input: "base",
    displayed: "linear",
    stf: DISPLAY,
  };
}

afterEach(() => {
  compositeChainStore.reset();
  captured.onCompositeDone = null;
});

describe("ProcessingTab steps on the wizard composite", () => {
  it("writes a deconvolution of the linear composite, with its parameters, into the HISTORY the Export step writes", async () => {
    const ctx = await renderTab();
    ctx.dispatch({ type: "SET_BINS", bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: b.id === "ha" ? ["/h.fits"] : b.id === "oiii" ? ["/o.fits"] : [] })) });
    ctx.dispatch({ type: "RECORD_COMPOSITE_OP", op: { kind: "blend", preset: "hoo" } });
    ctx.dispatch({ type: "SET_COMPOSITE_READY", ready: true });
    expect(captured.onCompositeDone).not.toBeNull();
    const lines = ["Deconvolution RL: 20 iterations, PSF gaussian sigma 2 px size 15"];
    captured.onCompositeDone!("deconv", "Deconvolved", deconvResult(true), DISPLAY, lines);
    expect(compositeHistoryLines(ctx.getState().compositeHistory)).toEqual(["Blend: hoo", ...lines]);
  });
});
