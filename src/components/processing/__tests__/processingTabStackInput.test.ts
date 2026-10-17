import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { renderToStaticMarkup } from "react-dom/server";

interface PanelFile {
  path: string;
  result?: { previewUrl?: string; dimensions?: [number, number] } | null;
}

interface StepDone {
  node?: string;
  corrected_fits?: string;
  fits_path?: string;
  previewUrl?: string;
  dimensions?: number[];
}

interface PanelProps {
  selectedFile: PanelFile | null;
  chainedFrom?: string;
  inputPreviewUrl?: string | null;
  inputLabel?: string;
  onProcessingDone?: (res: StepDone) => void | Promise<void>;
  onPreviewUpdate?: (url: string | null | undefined, dims: readonly number[] | null | undefined) => void;
  onPsfReady?: (kernel: number[][]) => void;
}

const { captured, getHistoryMock } = vi.hoisted(() => ({
  captured: {} as Record<string, PanelProps | undefined>,
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

function capture(name: string) {
  return {
    default: (props: PanelProps) => {
      captured[name] = props;
      return null;
    },
  };
}

vi.mock("../BackgroundPanel", () => capture("background"));
vi.mock("../WaveletPanel", () => capture("denoise"));
vi.mock("../DebayerPanel", () => capture("debayer"));
vi.mock("../PixelMathPanel", () => capture("pixelMath"));
vi.mock("../PsfPanel", () => capture("psf"));

import { CompositeProvider } from "../../../context/CompositeContext";
import { ComposeWizardProvider } from "../../../context/ComposeWizardContext";
import { PreviewProvider, fileKeyOf, getRenderRecord, useRenderActions } from "../../../context/PreviewContext";
import ProcessingTab from "../ProcessingTab";
import type { ProcessedFile } from "../../../shared/types";
import type { ProcessedResult } from "../../../shared/types/preview";
import type { HistoryNode, HistoryState, OpSpec } from "../../../shared/types/ops";

const LIGHT = "C:/data/light_001.fits";
const STACK = "C:/out/light_001_stack3_20261010-120000-000.fits";
const STACK_LABEL = "Stack - 3 frames";
const PV = "C:/out/g235h_pv.fits";
const HISTORY_DIR = "C:/app/history/5d2e7c1a9b3f0e86";
const STACK_BG = `${HISTORY_DIR}/n0001.fits`;
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

function stackBackgroundRun(): StepDone {
  getHistoryMock.mockResolvedValue(historyState(STACK, STACK, [historyNode("n0001", null, BACKGROUND)], "n0001"));
  return { node: "n0001", corrected_fits: STACK_BG, previewUrl: "asset://out/stack_bg.png", dimensions: [66, 50] };
}

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "light_001.fits",
    path: LIGHT,
    sourcePath: LIGHT,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/light_001.png", previewUrl: "asset://out/light_001.png", dimensions: [64, 48], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function publish(key: string, result: ProcessedResult): void {
  function Publisher() {
    useRenderActions().publishProcessed(key, result);
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Publisher) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function stackRecord(): ProcessedResult {
  return { fitsPath: STACK, previewUrl: "asset://out/stack.png", dimensions: [66, 50], label: STACK_LABEL, kind: "stacking", inputPath: LIGHT };
}

function frameRecord(n: number): ProcessedResult {
  return { fitsPath: `C:/out/f${n}.fits`, previewUrl: `asset://out/f${n}.png`, dimensions: [64, 48], label: `Frame ${n}`, kind: "cube", inputPath: LIGHT, frameIndex: n };
}

async function renderTab(file: ProcessedFile): Promise<string> {
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, {
      children: createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(ProcessingTab) }),
    }),
  });
  const { prelude } = await prerender(tree);
  return new Response(prelude).text();
}

function panel(name: string): PanelProps {
  const props = captured[name];
  if (!props) throw new Error(`ProcessingTab did not render the ${name} panel`);
  return props;
}

beforeEach(() => {
  for (const name of Object.keys(captured)) delete captured[name];
});

describe("ProcessingTab with a stack on screen", () => {
  it("hands Background the stack, compared with the stack preview, under the stack label", async () => {
    const file = fileOf("stack-bg");
    publish(fileKeyOf(file) as string, stackRecord());
    await renderTab(file);
    const bg = panel("background");
    expect(bg.selectedFile?.path).toBe(STACK);
    expect(bg.selectedFile?.result?.previewUrl).toMatch(/^asset:\/\/out\/stack\.png\?v=\d+$/);
    expect(bg.selectedFile?.result?.dimensions).toEqual([66, 50]);
    expect(bg.chainedFrom).toBe(STACK_LABEL);
  });

  it("records a Background run as read from the stack and keeps the stack as the base", async () => {
    const file = fileOf("stack-bg-done");
    const key = fileKeyOf(file) as string;
    publish(key, stackRecord());
    await renderTab(file);
    await panel("background").onProcessingDone?.(stackBackgroundRun());
    expect(getHistoryMock).toHaveBeenLastCalledWith({ imageKey: LIGHT, basePath: STACK });
    const record = getRenderRecord(key);
    expect(record?.processed).toMatchObject({ fitsPath: STACK_BG, inputPath: STACK, kind: "processing" });
    expect(record?.chain.base?.path).toBe(STACK);
    expect(record?.chain.steps.background?.fitsPath).toBe(STACK_BG);
  });

  it("hands Denoise the stack with its preview and label when no step ran", async () => {
    const file = fileOf("stack-denoise");
    publish(fileKeyOf(file) as string, stackRecord());
    await renderTab(file);
    const denoise = panel("denoise");
    expect(denoise.selectedFile?.path).toBe(STACK);
    expect(denoise.inputLabel).toBe(STACK_LABEL);
    expect(denoise.inputPreviewUrl).toMatch(/^asset:\/\/out\/stack\.png\?v=\d+$/);
  });

  it("names the stack as the first crumb once a step ran on it", async () => {
    const file = fileOf("stack-crumb");
    const key = fileKeyOf(file) as string;
    publish(key, stackRecord());
    await renderTab(file);
    await panel("background").onProcessingDone?.(stackBackgroundRun());
    const html = await renderTab(file);
    expect(html).toContain(STACK_LABEL);
    expect(html).not.toMatch(/>light_001<\/span>/);
  });

  it("shows no PNG-only notice for a stack with a FITS", async () => {
    const file = fileOf("stack-no-notice");
    publish(fileKeyOf(file) as string, stackRecord());
    const html = await renderTab(file);
    expect(html).not.toContain("not the image on screen");
  });
});

describe("ProcessingTab with a PNG-only result on screen", () => {
  it("tells the user the steps read the loaded file", async () => {
    const file = fileOf("drizzle-rgb");
    publish(fileKeyOf(file) as string, {
      fitsPath: null,
      previewUrl: "asset://out/drizzle_rgb.png",
      dimensions: null,
      label: "Drizzle RGB - 6 frames",
      kind: "stacking",
      inputPath: LIGHT,
    });
    const html = await renderTab(file);
    expect(html).toContain("Input: light_001.fits, not the image on screen (Drizzle RGB - 6 frames is a PNG-only result).");
    expect(panel("background").selectedFile?.path).toBe(LIGHT);
  });
});

describe("ProcessingTab Debayer publish", () => {
  it("publishes the Debayer output dimensions", async () => {
    const file = fileOf("debayer-dims");
    const key = fileKeyOf(file) as string;
    await renderTab(file);
    panel("debayer").onPreviewUpdate?.("asset://out/cfa_debayer.png", [512, 384]);
    expect(getRenderRecord(key)?.processed).toMatchObject({ kind: "debayer", fitsPath: null, dimensions: [512, 384], inputPath: LIGHT });
  });
});

describe("ProcessingTab PixelMath compare on a non-file input", () => {
  it("keeps the PV slice as the before image on every PixelMath run", async () => {
    const file = fileOf("pv-pixelmath");
    const key = fileKeyOf(file) as string;
    publish(key, { fitsPath: PV, previewUrl: "asset://out/pv.png", dimensions: [40, 900], label: "PV slice", kind: "cube", inputPath: LIGHT });
    const bases: { inputPreviewUrl?: string | null; inputLabel?: string }[] = [];
    for (let run = 0; run < 2; run++) {
      await renderTab(file);
      const pm = panel("pixelMath");
      bases.push({ inputPreviewUrl: pm.inputPreviewUrl, inputLabel: pm.inputLabel });
      const node = `n000${run + 1}`;
      const nodes = Array.from({ length: run + 1 }, (_, i) => historyNode(`n000${i + 1}`, null, PIXELMATH));
      getHistoryMock.mockResolvedValue(historyState(PV, PV, nodes, node));
      await pm.onProcessingDone?.({ node, fits_path: `${HISTORY_DIR}/${node}.fits`, previewUrl: "asset://out/pm.png", dimensions: [40, 900] });
      expect(getRenderRecord(key)?.processed?.inputPath).toBe(PV);
    }
    await renderTab(file);
    const pm = panel("pixelMath");
    bases.push({ inputPreviewUrl: pm.inputPreviewUrl, inputLabel: pm.inputLabel });
    const pvBase = { inputPreviewUrl: expect.stringMatching(/^asset:\/\/out\/pv\.png\?v=\d+$/), inputLabel: "PV slice" };
    expect(bases).toEqual([pvBase, pvBase, pvBase]);
  });

  it("compares with the image on screen when it is outside the chain", async () => {
    const file = fileOf("wcs-pixelmath");
    const key = fileKeyOf(file) as string;
    publish(key, { fitsPath: "C:/out/light_001_wcs.fits", previewUrl: "asset://out/wcs.png", dimensions: [64, 48], label: "Solved WCS", kind: "processing", inputPath: LIGHT });
    await renderTab(file);
    expect(panel("pixelMath").inputPreviewUrl).toMatch(/^asset:\/\/out\/wcs\.png\?v=\d+$/);
    expect(panel("pixelMath").inputLabel).toBe("Solved WCS");
  });
});

describe("ProcessingTab when the frame on screen changes during a run", () => {
  it("drops a Background result computed from the previous frame", async () => {
    const file = fileOf("frame-bg-race");
    const key = fileKeyOf(file) as string;
    publish(key, frameRecord(120));
    await renderTab(file);
    const bg = panel("background");
    expect(bg.selectedFile?.path).toBe("C:/out/f120.fits");
    publish(key, frameRecord(121));
    getHistoryMock.mockResolvedValue(historyState(LIGHT, "C:/out/f120.fits", [historyNode("n0001", null, BACKGROUND)], "n0001"));
    await bg.onProcessingDone?.({ node: "n0001", corrected_fits: `${HISTORY_DIR}/n0001.fits`, previewUrl: "asset://out/f120_bg.png", dimensions: [64, 48] });
    expect(getHistoryMock).toHaveBeenLastCalledWith({ imageKey: LIGHT, basePath: "C:/out/f120.fits" });
    const record = getRenderRecord(key);
    expect(record?.chain.base?.path).toBe("C:/out/f121.fits");
    expect(record?.chain.steps.background).toBeUndefined();
    expect(record?.processed).toMatchObject({ fitsPath: "C:/out/f121.fits", label: "Frame 121" });
  });

  it("drops a PSF measured on the previous frame", async () => {
    const file = fileOf("frame-psf-race");
    const key = fileKeyOf(file) as string;
    publish(key, frameRecord(120));
    await renderTab(file);
    const psf = panel("psf");
    expect(psf.selectedFile?.path).toBe("C:/out/f120.fits");
    publish(key, frameRecord(121));
    psf.onPsfReady?.([[0, 1, 0], [1, 4, 1], [0, 1, 0]]);
    expect(getRenderRecord(key)?.chain.psfKernel).toBeNull();
  });

  it("keeps a PSF measured on the frame still on screen", async () => {
    const file = fileOf("frame-psf-kept");
    const key = fileKeyOf(file) as string;
    publish(key, frameRecord(120));
    await renderTab(file);
    panel("psf").onPsfReady?.([[0, 1, 0], [1, 4, 1], [0, 1, 0]]);
    expect(getRenderRecord(key)?.chain.psfKernel).toEqual([[0, 1, 0], [1, 4, 1], [0, 1, 0]]);
    expect(getRenderRecord(key)?.chain.base?.path).toBe("C:/out/f120.fits");
  });
});
