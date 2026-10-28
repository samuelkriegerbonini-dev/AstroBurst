import { beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { HeadView, HistoryDoc, HistoryNode, HistoryState } from "../../../shared/types/ops";
import type { ProcessedResult } from "../../../shared/types/preview";

const RGB = "C:\\astrokit\\docs\\notCommit\\comparison-2026-10-26\\runtime-rgb.fits";
const RGB_PNG = "asset://localhost/runtime-rgb.png";
const HISTORY_DIR = "C:\\Users\\samue\\AppData\\Roaming\\com.astroburst.desktop\\history\\344da451b072ae3a";
const CURVES = `${HISTORY_DIR}\\n0001.fits`;
const MONO = "C:\\data\\m16_f502n.fits";
const COMPOSE_PNG = "asset://localhost/rgb_composite_blend.png";
const STF_R = { shadow: 0.01, midtone: 0.2, highlight: 1 };
const STF_G = { shadow: 0.02, midtone: 0.3, highlight: 1 };
const STF_B = { shadow: 0.03, midtone: 0.4, highlight: 1 };

interface ScreenFile {
  id: string;
  path: string;
  name: string;
  status: string;
  result: { is_rgb: boolean; previewUrl: string; dimensions: [number, number] };
}

const invoke = vi.hoisted(() => vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>());
const screen = vi.hoisted(() => ({
  file: null as ScreenFile | null,
  processed: null as ProcessedResult | null,
  composite: null as string | null,
  channels: null as { r: string | null; g: string | null; b: string | null } | null,
  head: "n0001" as string | null,
}));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../infrastructure/tauri/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri/client")>()),
  isTauri: () => false,
  typedInvoke: invoke,
  getPreviewUrl: async (path: string) => `asset://localhost/${path}`,
}));
vi.mock("../../../infrastructure/tauri/output", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri/output")>()),
  getExportDir: async () => "C:/exports",
}));
vi.mock("../../../context/PreviewContext", () => ({
  fileKeyOf: (file: ScreenFile | null) => (file ? `${file.id}|${file.path}` : null),
  getRenderRecord: () => null,
  useFileContext: () => ({ file: screen.file }),
  useHistContext: () => ({ stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
  useRgbContext: () => ({ rgbChannels: screen.channels }),
  useCubeContext: () => ({ isCube: false, cubeDims: null }),
  useRenderContext: () => ({ processed: screen.processed, processedVersion: 1, chain: { steps: {}, psfKernel: null, base: null }, stfPreviewUrl: null }),
  useRenderActions: () => ({}),
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
}));
vi.mock("../../../context/CompositeContext", () => ({
  useCompositePreview: () => ({ compositePreviewUrl: screen.composite, compositeVersion: 1 }),
  useCompositeStf: () => ({ compositeStfR: STF_R, compositeStfG: STF_G, compositeStfB: STF_B, compositeStfLinked: false }),
}));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => null }));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => ({ regions: [], selectedId: null }) }));

import { hooks } from "../../../test/hookRuntime";
import ExportTab from "../ExportTab";
import ExportPanel from "../ExportPanel";
import { adoptRun, loadHistory, moveHead, type HistoryRender, type HistoryTarget } from "../../../hooks/useHistory";

const CURVES_NODE: HistoryNode = {
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
};

function historyDoc(head: string | null): HistoryDoc {
  return {
    format: "astroburst-history",
    version: 1,
    image: { ref: RGB.toLowerCase(), id: "344da451b072ae3a", size: 201600, mtime: "2026-10-26T16:31:40.899819300Z" },
    image_key: RGB,
    base_path: null,
    head,
    last_issued: 1,
    policy: { k: 4 },
    app: "0.7.5",
    threads: 24,
    masks: {},
    nodes: [CURVES_NODE],
  };
}

function historyState(): HistoryState {
  return {
    image_id: "344da451b072ae3a",
    history_dir: HISTORY_DIR,
    root_path: RGB,
    doc: historyDoc(screen.head),
    read_only: null,
    fingerprint_matches: true,
    busy: false,
  };
}

function headView(): HeadView {
  const node = screen.head === null
    ? null
    : {
        id: "n0001",
        parent: null,
        op: CURVES_NODE.op,
        fits_path: CURVES,
        png_path: `${HISTORY_DIR}\\n0001.png`,
        extras: [],
        dimensions: [128, 128] as [number, number],
        planes: 3,
        domain: "display_referred" as const,
        present: true,
      };
  return { image_id: "344da451b072ae3a", history_dir: HISTORY_DIR, head: screen.head, node, root_path: RGB, can_undo: screen.head !== null, can_redo: screen.head === null, node_count: 1 };
}

const RGB_FILE: ScreenFile = {
  id: "rgb",
  path: RGB,
  name: "runtime-rgb.fits",
  status: "done",
  result: { is_rgb: true, previewUrl: RGB_PNG, dimensions: [128, 128] },
};

const MONO_FILE: ScreenFile = {
  id: "mono",
  path: MONO,
  name: "m16_f502n.fits",
  status: "done",
  result: { is_rgb: false, previewUrl: "asset://localhost/m16_f502n.png", dimensions: [1600, 1600] },
};

const TARGET: HistoryTarget = { fileKey: `rgb|${RGB}`, scope: { imageKey: RGB, basePath: null }, base: null, rootPreviewUrl: RGB_PNG };

const RENDER: HistoryRender = {
  currentFileKey: () => TARGET.fileKey,
  publishProcessed: (_key, record) => {
    screen.processed = record;
  },
  setChain: () => {},
  resetProcessed: () => {
    screen.processed = null;
  },
  setHistoryId: () => {},
};

type AnyProps = Record<string, unknown> & { children?: ReactNode };

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  return isValidElement<AnyProps>(node) ? textOf(node.props.children) : "";
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function exportPanelProps(): AnyProps {
  hooks.mount();
  const inner = (ExportTab as unknown as { type: (props: object) => ReactNode }).type;
  const panel = elements(hooks.render(inner, {}) as ReactNode).find((el) => "rgbFilePath" in el.props);
  if (!panel) throw new Error("ExportTab rendered no ExportPanel");
  return panel.props;
}

function panelTree(): ReactNode {
  const props = exportPanelProps();
  hooks.mount();
  return hooks.render(ExportPanel as unknown as (p: AnyProps) => unknown, props) as ReactNode;
}

async function press(label: string): Promise<Record<string, unknown>> {
  const button = elements(panelTree()).find((el) => el.type === "button" && textOf(el.props.children).includes(label));
  if (!button) throw new Error(`no "${label}" button`);
  invoke.mockClear();
  await (button.props.onClick as () => Promise<void>)();
  await flush();
  const call = invoke.mock.calls.find(([command]) => command.startsWith("export_"));
  if (!call) throw new Error(`"${label}" sent no export command`);
  return { command: call[0], ...call[1] };
}

const sources = (args: Record<string, unknown>) => [args.rPath, args.gPath, args.bPath];

beforeEach(() => {
  invoke.mockReset().mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === "history_get_cmd") return historyState();
    if (command === "history_undo_cmd") {
      screen.head = null;
      return headView();
    }
    if (command === "history_redo_cmd") {
      screen.head = "n0001";
      return headView();
    }
    if (command.startsWith("export_")) return { output_path: args?.outputPath, elapsed_ms: 1 };
    return {};
  });
  Object.assign(screen, { file: RGB_FILE, processed: null, composite: RGB_PNG, channels: null, head: "n0001" });
});

async function applyCurves(): Promise<void> {
  screen.head = "n0001";
  await adoptRun(TARGET, RENDER, "n0001");
  screen.composite = null;
}

describe("Export RGB on an RGB FITS follows the image on screen (AB-ERR-01)", () => {
  it("exports the original RGB file's planes while the original is shown", async () => {
    const fits = await press("Export RGB as FITS cube");
    expect(fits.command).toBe("export_fits_rgb");
    expect(sources(fits)).toEqual([RGB, RGB, RGB]);
    expect(exportPanelProps().sourceLabel).toBe("original file");
  });

  it("exports the Curves result, not the original, once Curves is applied", async () => {
    await applyCurves();
    expect(exportPanelProps().sourceLabel).toBe("processed: Curves");
    const fits = await press("Export RGB as FITS cube");
    expect(sources(fits), "the RGB cube must hold the processed planes the panel names").toEqual([CURVES, CURVES, CURVES]);
    expect(String(fits.outputPath)).toMatch(/^C:\/exports\/n0001_rgb_\d{8}-\d{6}-\d{3}\.fits$/);
  });

  it("goes back to the original after undo and to the Curves result after redo", async () => {
    await applyCurves();
    await moveHead(TARGET, RENDER, { kind: "undo" });
    screen.composite = RGB_PNG;
    expect(screen.processed).toBeNull();
    expect(sources(await press("Export RGB as FITS cube"))).toEqual([RGB, RGB, RGB]);

    await moveHead(TARGET, RENDER, { kind: "redo" });
    screen.composite = null;
    expect(screen.processed?.label).toBe("Curves");
    expect(sources(await press("Export RGB as FITS cube"))).toEqual([CURVES, CURVES, CURVES]);
  });

  it("exports the Curves result of a reopened file whose history head is Curves", async () => {
    screen.head = "n0001";
    await loadHistory(TARGET, RENDER);
    screen.composite = null;
    expect(screen.processed?.fitsPath).toBe(CURVES);
    expect(sources(await press("Export RGB as FITS cube"))).toEqual([CURVES, CURVES, CURVES]);
  });

  it("writes the RGB PNG from the Curves result without re-applying the original's STF", async () => {
    await applyCurves();
    const png = await press("Export RGB as PNG");
    expect(png.command).toBe("export_rgb_png");
    expect(sources(png)).toEqual([CURVES, CURVES, CURVES]);
    expect(png.applyStfStretch, "the Curves result is display-referred; its stretch is already baked in").toBe(false);
    expect(png.shadowR).toBeUndefined();
  });

  it("writes the RGB PNG of the original with the per-channel STF on screen", async () => {
    const png = await press("Export RGB as PNG");
    expect(sources(png)).toEqual([RGB, RGB, RGB]);
    expect(png).toMatchObject({ applyStfStretch: true, linked: false, shadowR: STF_R.shadow, midtoneG: STF_G.midtone, midtoneB: STF_B.midtone });
  });

  it("keeps the Compose composite and a mono file on their own sources", async () => {
    screen.composite = COMPOSE_PNG;
    const composite = await press("Export RGB as FITS cube");
    expect(sources(composite)).toEqual([null, null, null]);

    Object.assign(screen, { file: MONO_FILE, processed: null, composite: null, channels: null });
    expect(exportPanelProps().rgbFilePath).toBeNull();
    expect(elements(panelTree()).some((el) => el.type === "button" && textOf(el.props.children).includes("Export RGB as FITS cube"))).toBe(false);
  });
});
