import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import type { HistoryDoc } from "../../shared/types/ops";
import type { ProcessedResult } from "../../shared/types/preview";

const RGB = "C:\\astrokit\\docs\\notCommit\\comparison-2026-10-26\\runtime-rgb.fits";
const RGB_PNG = "asset://localhost/runtime-rgb.png";
const HISTORY_DIR = "C:\\Users\\samue\\AppData\\Roaming\\com.astroburst.desktop\\history\\344da451b072ae3a";
const CURVES = `${HISTORY_DIR}\\n0001.fits`;
const MONO = "C:\\data\\m16_f502n.fits";
const CUBE = "C:\\data\\jw01266_nirspec_s3d.fits";
const COMPOSE_PNG = "asset://localhost/rgb_composite_blend.png";

interface ScreenFile {
  id: string;
  path: string;
  name: string;
  result: { is_rgb: boolean; previewUrl: string; dimensions: [number, number]; header: Record<string, string>; elapsed_ms: number };
}

const screen = vi.hoisted(() => ({
  file: null as ScreenFile | null,
  processed: null as ProcessedResult | null,
  composite: null as string | null,
  isCube: false,
  gpu: true,
  compositeVersion: 3,
  processedVersion: 5,
}));
const loaders = vi.hoisted(() => ({
  loadRawPixels: vi.fn(),
  clearRawPixels: vi.fn(),
  loadRgbRawPixels: vi.fn(),
  clearRgbRawPixels: vi.fn(),
  resetComposite: vi.fn(),
}));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../infrastructure/gpu/GpuSingleton", () => ({
  probeGpu: () => Promise.resolve(null),
  isGpuAvailable: () => true,
  onGpuLost: () => () => {},
  getGpuReason: () => null,
}));
vi.mock("../../utils/gpuPreference", () => ({ loadGpuPreference: () => screen.gpu, saveGpuPreference: () => {} }));
vi.mock("../../hooks/useCpuStfPreview", () => ({ useCpuStfPreview: () => {} }));
vi.mock("../../hooks/useHistory", () => ({ useHistoryController: () => ({ revert: async () => {} }) }));
vi.mock("../../hooks/useRampStore", () => ({ useRampIntegration: () => 0 }));
vi.mock("../../hooks/useMousePixelStore", () => ({
  useMousePixelActions: () => ({ handleLeave: () => {}, reset: () => {} }),
  setMousePixel: () => {},
  emitPixelClick: () => {},
  usePixelClick: () => null,
}));
vi.mock("../../context/ComposeWizardContext", () => ({ useComposeWizardContext: () => ({ state: { compositeReady: false } }) }));
vi.mock("../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: screen.composite, compositeVersion: screen.compositeVersion, canShowParked: false }),
  useCompositeActions: () => ({
    initRgb: () => {},
    setCompositePreviewUrl: () => {},
    clearComposite: async () => {},
    resetComposite: loaders.resetComposite,
    setCompositeStf: () => {},
    setCompositeAutoStf: () => {},
    setCompositeStfLinked: () => {},
    park: () => {},
    replaceParked: () => {},
    showParkedComposite: async () => {},
  }),
}));
vi.mock("../../context/PreviewContext", () => ({
  fileKeyOf: (file: ScreenFile | null) => (file ? `${file.id}|${file.path}` : null),
  seedRgbFileView: () => {},
  useFileContext: () => ({ file: screen.file }),
  useCubeContext: () => ({ isCube: screen.isCube, isSpectralCube: screen.isCube, isSpectrum: false, ramp: null }),
  useDisplayedImage: () => {
    const processed = screen.processed;
    const file = screen.file;
    return {
      path: processed?.fitsPath ?? file?.path ?? null,
      dimensions: processed?.dimensions ?? file?.result.dimensions ?? null,
      isProcessed: processed !== null,
      label: processed?.label ?? null,
      previewOnly: processed !== null && processed.previewUrl !== null && processed.fitsPath === null,
      previewUrl: processed?.previewUrl ?? file?.result.previewUrl ?? null,
    };
  },
  useHistContext: () => ({ histData: null, histDataPath: null, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
  useRawPixelsContext: () => ({
    rawPixels: null,
    rawPixelsLoading: false,
    rawPixelsError: null,
    loadRawPixels: loaders.loadRawPixels,
    clearRawPixels: loaders.clearRawPixels,
    rgbRawPixels: null,
    rgbRawPixelsLoading: false,
    loadRgbRawPixels: loaders.loadRgbRawPixels,
    clearRgbRawPixels: loaders.clearRgbRawPixels,
  }),
  useRenderActions: () => ({ currentFileKey: () => null, setStfPreviewUrl: () => {} }),
  useRenderContext: () => ({ processed: screen.processed, processedVersion: screen.processedVersion, stfPreviewUrl: null, chain: { steps: {}, psfKernel: null, base: null } }),
  useStarOverlayContext: () => ({ starOverlayRef: { current: null } }),
}));
vi.mock("../viewer/AdvancedImageViewer", () => ({ default: function CpuViewer() { return null; } }));
vi.mock("../history/HistoryShortcuts", () => ({ default: () => null }));
vi.mock("../preview/DqControls", () => ({ default: () => null }));
vi.mock("../preview/DqOverlayCanvas", () => ({ default: () => null }));
vi.mock("../preview/ViewerStatusStrip", () => ({ default: () => null }));
vi.mock("../preview/SpectrumViewerEmpty", () => ({ default: () => null }));
vi.mock("../preview/DisplayControls", () => ({ default: () => null }));
vi.mock("../preview/PreviewTab", () => ({ default: function GpuTab() { return createElement("i"); } }));

import { hooks } from "../../test/hookRuntime";
import PreviewPanel from "../PreviewPanel";
import CpuViewer from "../viewer/AdvancedImageViewer";
import GpuTab from "../preview/PreviewTab";
import { nodeRecord } from "../../utils/historyModel";

type AnyProps = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

const CURVES_DOC: HistoryDoc = {
  format: "astroburst-history",
  version: 1,
  image: { ref: RGB.toLowerCase(), id: "344da451b072ae3a", size: 201600, mtime: "2026-10-26T16:31:40.899819300Z" },
  image_key: RGB,
  base_path: null,
  head: "n0001",
  last_issued: 1,
  policy: { k: 4 },
  app: "0.7.5",
  threads: 24,
  masks: {},
  nodes: [
    {
      id: "n0001",
      parent: null,
      op: { op: "tone", v: 1, levels: { black: 0, gamma: 1.5, white: 1 } },
      mask: null,
      output: { path: "n0001.fits", png: "n0001.png", extras: [], dims: [128, 128], planes: 3, domain: "display_referred" },
      at: "2026-10-26T16:33:46.701608500Z",
      elapsed_ms: 6,
      app: "0.7.5",
      threads: 24,
      present: true,
      bytes: 220110,
      checkpoint: false,
    },
  ],
};

const curvesRecord = (): ProcessedResult => {
  const record = nodeRecord(CURVES_DOC, "n0001", { dir: HISTORY_DIR, rootPath: RGB, rootPreviewUrl: RGB_PNG, previewOf: (png) => `asset://localhost/${png}` });
  if (!record) throw new Error("no record for n0001");
  return record;
};

const monoRecord: ProcessedResult = {
  fitsPath: `${HISTORY_DIR}\\n0002.fits`,
  previewUrl: "asset://localhost/n0002.png",
  dimensions: [1600, 1600],
  label: "Curves",
  kind: "processing",
  inputPath: MONO,
  planes: 1,
  domain: "display_referred",
};

function fileOf(id: string, path: string, isRgb: boolean, dims: [number, number]): ScreenFile {
  return { id, path, name: path.split("\\").pop() ?? path, result: { is_rgb: isRgb, previewUrl: isRgb ? RGB_PNG : `asset://localhost/${id}.png`, dimensions: dims, header: {}, elapsed_ms: 10 } };
}

const RGB_FILE = fileOf("rgb", RGB, true, [128, 128]);
const MONO_FILE = fileOf("mono", MONO, false, [1600, 1600]);
const CUBE_FILE = fileOf("cube", CUBE, false, [55, 55]);

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

interface View {
  file: ScreenFile;
  processed?: ProcessedResult | null;
  composite?: string | null;
  isCube?: boolean;
  gpu?: boolean;
}

const renderPanel = () => hooks.render(PreviewPanel as unknown as (p: object) => unknown, {}) as ReactNode;

async function rerender(): Promise<ReactNode> {
  renderPanel();
  await flush();
  const tree = renderPanel();
  await flush();
  return tree;
}

async function showOnGpu(view: View): Promise<ReactNode> {
  Object.assign(screen, { processed: null, composite: null, isCube: false, gpu: true }, view);
  hooks.mount();
  for (const fn of Object.values(loaders)) fn.mockClear();
  return rerender();
}

async function moveTo(view: Omit<View, "file">): Promise<ReactNode> {
  Object.assign(screen, view, { compositeVersion: screen.compositeVersion + 1, processedVersion: screen.processedVersion + 1 });
  return rerender();
}

const rgbLoads = () => loaders.loadRgbRawPixels.mock.calls;

beforeEach(() => {
  hooks.mount();
});

describe("PreviewPanel loads every RGB product through the RGB GPU path (AB-ERR-04)", () => {
  it("loads the original RGB file's planes while the file's RGB view is on screen", async () => {
    await showOnGpu({ file: RGB_FILE, composite: RGB_PNG });
    expect(rgbLoads().at(-1)?.slice(0, 2)).toEqual([RGB, true]);
    expect(loaders.loadRawPixels).not.toHaveBeenCalled();
  });

  it("loads the three planes of the Curves result through the RGB loader, not the 2D image loader", async () => {
    await showOnGpu({ file: RGB_FILE, processed: curvesRecord(), composite: null });
    expect(loaders.loadRawPixels, "the 2D loader fails on a 3-plane product and drops to the CPU viewer").not.toHaveBeenCalled();
    expect(rgbLoads().at(-1)).toEqual([CURVES, true, true]);
  });

  it("routes the Curves result to the RGB loader in the render before the file's RGB view is reset", async () => {
    await showOnGpu({ file: RGB_FILE, processed: curvesRecord(), composite: RGB_PNG });
    expect(loaders.resetComposite).toHaveBeenCalled();
    expect(loaders.loadRawPixels).not.toHaveBeenCalled();
    expect(rgbLoads().at(-1)).toEqual([CURVES, true, true]);
  });

  it("keeps mono files, mono results and science cubes on the 2D loader", async () => {
    await showOnGpu({ file: MONO_FILE });
    expect(loaders.loadRawPixels).toHaveBeenCalled();
    expect(loaders.loadRgbRawPixels).not.toHaveBeenCalled();

    await showOnGpu({ file: MONO_FILE, processed: monoRecord });
    expect(loaders.loadRawPixels).toHaveBeenCalled();
    expect(loaders.loadRgbRawPixels).not.toHaveBeenCalled();

    await showOnGpu({ file: CUBE_FILE, isCube: true });
    expect(loaders.loadRawPixels).toHaveBeenCalled();
    expect(loaders.loadRgbRawPixels).not.toHaveBeenCalled();
  });

  it("keeps the Compose composite on the composite slot", async () => {
    await showOnGpu({ file: MONO_FILE, composite: COMPOSE_PNG });
    expect(rgbLoads().at(-1)?.slice(0, 2)).toEqual([null, true]);
    expect(loaders.loadRawPixels).not.toHaveBeenCalled();
  });

  it("follows Curves, undo, redo and a reopened result with one RGB load each and never the 2D loader", async () => {
    const tab = (tree: ReactNode) => elements(tree).some((el) => el.type === GpuTab);
    expect(tab(await showOnGpu({ file: RGB_FILE, composite: RGB_PNG }))).toBe(true);
    expect(tab(await moveTo({ processed: curvesRecord(), composite: null }))).toBe(true);
    expect(tab(await moveTo({ processed: null, composite: RGB_PNG }))).toBe(true);
    expect(tab(await moveTo({ processed: curvesRecord(), composite: null }))).toBe(true);
    expect(rgbLoads().map((call) => [call[0], call[2] ?? false])).toEqual([
      [RGB, false],
      [CURVES, true],
      [RGB, false],
      [CURVES, true],
    ]);
    expect(tab(await showOnGpu({ file: RGB_FILE, processed: curvesRecord(), composite: null }))).toBe(true);
    expect(rgbLoads()).toEqual([[CURVES, true, true]]);
    expect(loaders.loadRawPixels).not.toHaveBeenCalled();
  });

  it("leaves a result to the CPU viewer when the GPU is off, as before", async () => {
    const tree = await showOnGpu({ file: RGB_FILE, processed: curvesRecord(), composite: null, gpu: false });
    const viewer = elements(tree).find((el) => el.type === CpuViewer);
    expect((viewer?.props.processed as { url: string } | null)?.url).toBe(curvesRecord().previewUrl);
    expect(loaders.loadRawPixels).not.toHaveBeenCalled();
    expect(loaders.loadRgbRawPixels).not.toHaveBeenCalled();
  });
});
