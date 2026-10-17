import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";

interface PixelMathDone {
  node: string;
  fits_path: string;
  previewUrl: string;
  dimensions: number[];
}

interface CompareBaseProps {
  inputPreviewUrl?: string | null;
  inputLabel?: string;
}

const { captured, getHistoryMock } = vi.hoisted(() => ({
  captured: {
    onProcessingDone: null as null | ((res: PixelMathDone) => void | Promise<void>),
    compareBase: null as null | CompareBaseProps,
  },
  getHistoryMock: vi.fn(),
}));

vi.mock("../../../services/ops", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/ops")>()),
  getHistory: getHistoryMock,
}));

vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  getPreviewUrl: async (path: string) => path,
}));

vi.mock("../PixelMathPanel", () => ({
  default: (props: CompareBaseProps & { onProcessingDone?: (res: PixelMathDone) => void | Promise<void> }) => {
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
import type { HistoryNode, HistoryState, OpSpec } from "../../../shared/types/ops";

const RAW = "C:/data/f444w_i2d.fits";
const HISTORY_DIR = "C:/app/history/3f9c0a1e5b7d2c44";
const BG = `${HISTORY_DIR}/n0001.fits`;
const BACKGROUND: OpSpec = { op: "background", v: 1, model: "polynomial", config: { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" } };
const PIXELMATH: OpSpec = { op: "pixelmath", v: 1, expression: "$T * 2", slots: [], truncate: false, rescale: false };

function historyNode(id: string, parent: string | null, op: OpSpec): HistoryNode {
  return {
    id,
    parent,
    op,
    mask: null,
    output: { path: `${id}.fits`, png: `${id}.png`, extras: [], dims: [8, 8], planes: 1, domain: "display_referred" },
    at: "2026-10-16T12:00:00Z",
    elapsed_ms: 1,
    app: "0.6.9",
    threads: 8,
    present: true,
    bytes: 256,
    checkpoint: false,
  };
}

function historyState(rootPath: string, basePath: string | null, nodes: HistoryNode[], head: string | null): HistoryState {
  return {
    image_id: "3f9c0a1e5b7d2c44",
    history_dir: HISTORY_DIR,
    root_path: rootPath,
    doc: {
      format: "astroburst-history",
      version: 1,
      image: { id: "3f9c0a1e5b7d2c44", ref: rootPath, size: 1, mtime: null },
      image_key: rootPath,
      base_path: basePath,
      head,
      policy: { k: 4 },
      app: "0.6.9",
      threads: 8,
      masks: {},
      nodes,
    },
    read_only: null,
    fingerprint_matches: true,
    busy: false,
  };
}

function pixelMathRun(run: number): PixelMathDone {
  const nodes = [historyNode("n0001", null, BACKGROUND)];
  for (let i = 0; i <= run; i++) nodes.push(historyNode(`n${String(i + 2).padStart(4, "0")}`, "n0001", PIXELMATH));
  const head = nodes[nodes.length - 1].id;
  getHistoryMock.mockResolvedValue(historyState(RAW, null, nodes, head));
  return { node: head, fits_path: `${HISTORY_DIR}/${head}.fits`, previewUrl: "asset://out/pm.png", dimensions: [8, 8] };
}

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

function takeOnProcessingDone(): (res: PixelMathDone) => void | Promise<void> {
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
      await takeOnProcessingDone()(pixelMathRun(run));
      recordedInputs.push(getRenderRecord(key)?.processed?.inputPath);
    }
    expect(recordedInputs).toEqual([BG, BG, BG]);
    const record = getRenderRecord(key);
    expect(record?.processed).toMatchObject({ fitsPath: `${HISTORY_DIR}/n0004.fits`, kind: "pixelmath", label: "PixelMath" });
    expect(record?.chain.steps.background?.fitsPath).toBe(BG);
    expect(record?.chain.steps.pixelMath?.fitsPath).toBe(`${HISTORY_DIR}/n0004.fits`);
  });

  it("compares every re-run with the Background output it read, never with the previous PixelMath output", async () => {
    const file: ProcessedFile = { ...FILE, id: "pm-tab-compare-base" };
    const key = fileKeyOf(file) as string;
    showBackgroundOutput(key);
    const bases: CompareBaseProps[] = [];
    for (let run = 0; run < 3; run++) {
      await renderTab(file);
      bases.push(takeCompareBase());
      await takeOnProcessingDone()(pixelMathRun(run));
    }
    await renderTab(file);
    bases.push(takeCompareBase());
    const shownBackground = { inputPreviewUrl: expect.stringMatching(/^asset:\/\/out\/bg\.png(\?v=\d+)?$/), inputLabel: "Background" };
    const projectedBackground = { inputPreviewUrl: `${HISTORY_DIR}/n0001.png?v=${Date.parse("2026-10-16T12:00:00Z")}`, inputLabel: "Background" };
    expect(bases).toEqual([shownBackground, projectedBackground, projectedBackground, projectedBackground]);
  });
});
