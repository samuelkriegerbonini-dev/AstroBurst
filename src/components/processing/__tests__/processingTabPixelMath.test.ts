import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";

interface PixelMathDone {
  fits_path: string;
  previewUrl: string;
  dimensions: number[];
}

interface CompareBaseProps {
  inputPreviewUrl?: string | null;
  inputLabel?: string;
}

const { captured } = vi.hoisted(() => ({
  captured: {
    onProcessingDone: null as null | ((res: PixelMathDone) => void),
    compareBase: null as null | CompareBaseProps,
  },
}));

vi.mock("../PixelMathPanel", () => ({
  default: (props: CompareBaseProps & { onProcessingDone?: (res: PixelMathDone) => void }) => {
    captured.onProcessingDone = props.onProcessingDone ?? null;
    captured.compareBase = { inputPreviewUrl: props.inputPreviewUrl, inputLabel: props.inputLabel };
    return null;
  },
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf, getRenderRecord, useRenderActions } from "../../../context/PreviewContext";
import { withStep } from "../../../utils/processingChain";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";

const RAW = "C:/data/f444w_i2d.fits";
const BG = "C:/out/f444w_i2d_bg.fits";
const PM = "C:/out/f444w_i2d_bg_pixelmath.fits";

const FILE: ProcessedFile = {
  id: "pm-tab-record",
  name: "f444w_i2d.fits",
  path: RAW,
  sourcePath: RAW,
  imageRef: null,
  size: 1,
  status: "done",
  result: { png_path: "C:/out/f444w_i2d.png", previewUrl: "asset://C:/out/f444w_i2d.png", dimensions: [8, 8], elapsed_ms: 1, is_rgb: false },
  error: null,
  startedAt: null,
  finishedAt: null,
};

function showBackgroundOutput(key: string): void {
  function Publisher() {
    const { publishProcessed } = useRenderActions();
    publishProcessed(
      key,
      { fitsPath: BG, previewUrl: "asset://out/bg.png", dimensions: [8, 8], label: "Background", kind: "processing", inputPath: RAW },
      (c) => withStep(c, "background", { fitsPath: BG, previewUrl: "asset://out/bg.png", dimensions: [8, 8] }),
    );
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

async function renderTab(file: ProcessedFile): Promise<void> {
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ProcessingTab) }),
    }),
  });
  await prerender(tree);
}

function takeCompareBase(): CompareBaseProps {
  const base = captured.compareBase;
  captured.compareBase = null;
  if (!base) throw new Error("ProcessingTab did not render the PixelMath panel");
  return base;
}

function takeOnProcessingDone(): (res: PixelMathDone) => void {
  const done = captured.onProcessingDone;
  captured.onProcessingDone = null;
  if (!done) throw new Error("ProcessingTab did not render the PixelMath panel");
  return done;
}

describe("ProcessingTab PixelMath record", () => {
  it("records the image PixelMath read on every re-run, so the third run does not feed on the second", async () => {
    const key = fileKeyOf(FILE) as string;
    showBackgroundOutput(key);
    const recordedInputs: (string | undefined)[] = [];
    for (let run = 0; run < 3; run++) {
      await renderTab(FILE);
      takeOnProcessingDone()({ fits_path: PM, previewUrl: "asset://out/pm.png", dimensions: [8, 8] });
      recordedInputs.push(getRenderRecord(key)?.processed?.inputPath);
    }
    expect(recordedInputs).toEqual([BG, BG, BG]);
    const record = getRenderRecord(key);
    expect(record?.processed).toMatchObject({ fitsPath: PM, kind: "pixelmath", label: "PixelMath" });
    expect(record?.chain.steps.background?.fitsPath).toBe(BG);
    expect(record?.chain.steps.pixelMath?.fitsPath).toBe(PM);
  });

  it("compares every re-run with the Background output it read, never with the previous PixelMath output", async () => {
    const file: ProcessedFile = { ...FILE, id: "pm-tab-compare-base" };
    const key = fileKeyOf(file) as string;
    showBackgroundOutput(key);
    const backgroundBase = { inputPreviewUrl: expect.stringMatching(/^asset:\/\/out\/bg\.png(\?v=\d+)?$/), inputLabel: "Background" };
    const bases: CompareBaseProps[] = [];
    for (let run = 0; run < 3; run++) {
      await renderTab(file);
      bases.push(takeCompareBase());
      takeOnProcessingDone()({ fits_path: PM, previewUrl: "asset://out/pm.png", dimensions: [8, 8] });
    }
    await renderTab(file);
    bases.push(takeCompareBase());
    expect(bases).toEqual([backgroundBase, backgroundBase, backgroundBase, backgroundBase]);
  });
});
