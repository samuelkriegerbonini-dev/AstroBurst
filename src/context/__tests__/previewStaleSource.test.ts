import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { ProcessedFile, RawPixelData, StfParams } from "../../shared/types";
import type { DisplayLook, ScaleLimits } from "../../shared/types/display";

vi.hoisted(() => {
  vi.stubGlobal("window", {
    setTimeout: (fn: () => void, ms?: number) => globalThis.setTimeout(fn, ms),
    clearTimeout: (handle: ReturnType<typeof setTimeout>) => globalThis.clearTimeout(handle),
    addEventListener: () => {},
    removeEventListener: () => {},
    devicePixelRatio: 1,
    innerWidth: 1000,
    innerHeight: 600,
    localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
  });
});

const invokeMock = vi.hoisted(() => vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>());

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../infrastructure/tauri/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../infrastructure/tauri/client")>()),
  typedInvoke: invokeMock,
}));
vi.mock("../CompositeContext", () => ({
  useCompositeActions: () => ({ park: () => {}, resetComposite: () => {}, initRgb: () => {}, setCompositePreviewUrl: () => {}, setCompositeStf: () => {}, setCompositeStfLinked: () => {} }),
  useCompositePreview: () => ({ compositePreviewUrl: null }),
}));
vi.mock("../ComposeWizardContext", () => ({ useComposeWizardContext: () => ({ state: { compositeReady: false } }) }));

import { hooks } from "../../test/hookRuntime";
import { PreviewProvider, fileKeyOf, type useRenderActions } from "../PreviewContext";
import { displayMemory } from "../../utils/displayMemoryStore";
import { emptyDisplayMemory, setDefaults } from "../../utils/displayMemory";

const INPUT = "C:/data/jw02739001001_02105_00001_nrca1_cal.fits";
const PRODUCT = "C:/out/jw02739_arcsinh.fits";
const CROPPED = "C:/out/jw02739_crop.fits";
const LIMITS_WAIT_MS = 1000;
const DEFAULT_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };
const INPUT_AUTO_STF: StfParams = { shadow: 0.1, midtone: 0.3, highlight: 1 };

const LINEAR_MINMAX: DisplayLook = {
  stretch: "linear",
  limits: "minmax",
  percentileLow: 1,
  percentileHigh: 99.5,
  userLo: null,
  userHi: null,
  zscaleContrast: 0.25,
  asinhA: 0.1,
  power: 2,
  colormap: "gray",
  invert: false,
  symmetric: false,
  centre: 0,
};

const MTF: DisplayLook = { ...LINEAR_MINMAX, stretch: "mtf" };

let rowSeq = 0;

function inputRow(): ProcessedFile {
  rowSeq += 1;
  return {
    id: `stale-${rowSeq}`,
    name: "jw02739001001_02105_00001_nrca1_cal.fits",
    path: INPUT,
    sourcePath: INPUT,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: `${INPUT}.png`, previewUrl: `asset://${INPUT}.png`, dimensions: [4, 2], elapsed_ms: 1, header: { NAXIS: "2" } },
    error: null,
    startedAt: 1,
    finishedAt: 2,
  };
}

function pixelBuffer(width: number, height: number): ArrayBuffer {
  const buf = new ArrayBuffer(16 + width * height * 4);
  const view = new DataView(buf);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  view.setFloat32(8, 0, true);
  view.setFloat32(12, 1, true);
  return buf;
}

const limitsOf = (vmin: number, vmax: number): ScaleLimits => ({ vmin, vmax, algorithm: "minmax", symmetric: false, centre: null, notes: [] });

type ContextValue = Record<string, unknown>;

function providerValues(node: ReactNode): ContextValue[] {
  const values: ContextValue[] = [];
  let current: ReactNode = node;
  while (isValidElement<{ value?: unknown; children?: ReactNode }>(current)) {
    const value = current.props.value;
    if (value !== null && typeof value === "object") values.push(value as ContextValue);
    current = current.props.children;
  }
  return values;
}

function contextWith<T>(tree: ReactNode, key: string): T {
  const value = providerValues(tree).find((v) => key in v);
  expect(value, `context value carrying ${key}`).toBeDefined();
  return value as T;
}

function render(file: ProcessedFile): ReactNode {
  let tree: ReactNode = null;
  hooks.run.active = true;
  try {
    for (let pass = 0; pass < 3; pass++) {
      hooks.run.cursor = 0;
      hooks.run.effects = [];
      tree = PreviewProvider({ file, doneFiles: [], children: null });
      for (const effect of hooks.run.effects) effect();
    }
  } finally {
    hooks.run.active = false;
  }
  return tree;
}

async function settle(): Promise<void> {
  vi.advanceTimersByTime(LIMITS_WAIT_MS);
  for (let i = 0; i < 8; i++) await Promise.resolve();
}

function publish(tree: ReactNode, file: ProcessedFile, fitsPath: string, dimensions: [number, number]): void {
  contextWith<ReturnType<typeof useRenderActions>>(tree, "publishProcessed").publishProcessed(fileKeyOf(file) as string, {
    fitsPath,
    previewUrl: `asset://${fitsPath}.png`,
    dimensions,
    label: "Arcsinh",
    kind: "processing",
    inputPath: file.path,
  });
}

interface RawPixelsValue {
  rawPixels: RawPixelData | null;
  rawPixelsError: string | null;
  rawPixelsSource: { key: string; dims: [number, number] | null } | null;
  loadRawPixels: (force?: boolean) => void;
}

interface DisplayValue {
  limits: ScaleLimits | null;
  limitsError: string | null;
  histError: string | null;
}

const rawPixelsOf = (tree: ReactNode) => contextWith<RawPixelsValue>(tree, "loadRawPixels");
const displayOf = (tree: ReactNode) => contextWith<DisplayValue>(tree, "limits");
const stfOf = (tree: ReactNode) => contextWith<{ stfParams: StfParams }>(tree, "stfParams").stfParams;

function failing(paths: Record<string, string>) {
  return (command: string, args?: Record<string, unknown>): Promise<unknown> => {
    const path = args?.path as string | undefined;
    const failure = path !== undefined ? paths[`${command}|${path}`] : undefined;
    if (failure !== undefined) return Promise.reject(new Error(failure));
    if (command === "get_raw_pixels_preview") return Promise.resolve(pixelBuffer(4, 2));
    if (command === "compute_scale_limits_cmd") return Promise.resolve(limitsOf(0, 10));
    if (command === "compute_histogram") return Promise.resolve({ bins: [1, 2], auto_stf: INPUT_AUTO_STF });
    return new Promise(() => {});
  };
}

beforeEach(() => {
  vi.useFakeTimers();
  hooks.run.cells = [];
  invokeMock.mockReset();
  displayMemory.update(() => setDefaults(emptyDisplayMemory(null), LINEAR_MINMAX));
});

afterEach(() => {
  vi.useRealTimers();
  displayMemory.update(() => emptyDisplayMemory(null));
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("PreviewProvider never shows data of a previous source under a new one", () => {
  it("drops the input pixels when the pixels of the product shown in their place fail to load", async () => {
    const file = inputRow();
    invokeMock.mockImplementation(failing({ [`get_raw_pixels_preview|${PRODUCT}`]: "product file is locked" }));
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
    rawPixelsOf(render(file)).loadRawPixels(true);
    await settle();
    let tree = render(file);
    expect(rawPixelsOf(tree).rawPixels?.width).toBe(4);

    publish(tree, file, PRODUCT, [4, 2]);
    rawPixelsOf(render(file)).loadRawPixels(true);
    await settle();
    tree = render(file);

    expect(rawPixelsOf(tree).rawPixelsError).toBe("product file is locked");
    expect(rawPixelsOf(tree).rawPixels).toBeNull();
    expect(rawPixelsOf(tree).rawPixelsSource).toBeNull();
    errorLog.mockRestore();
  });

  it("keeps pixels of the shown source when only their reload fails", async () => {
    const file = inputRow();
    invokeMock.mockImplementation(failing({}));
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
    rawPixelsOf(render(file)).loadRawPixels(true);
    await settle();
    invokeMock.mockImplementation(failing({ [`get_raw_pixels_preview|${INPUT}`]: "busy" }));
    rawPixelsOf(render(file)).loadRawPixels(true);
    await settle();
    const tree = render(file);

    expect(rawPixelsOf(tree).rawPixelsError).toBe("busy");
    expect(rawPixelsOf(tree).rawPixels?.width).toBe(4);
    expect(rawPixelsOf(tree).rawPixelsSource?.key).toBe(`${INPUT}|0`);
    errorLog.mockRestore();
  });

  it("records the grid of the image the pixels were loaded for, so another grid never borrows them", async () => {
    const file = inputRow();
    invokeMock.mockImplementation((command, args) =>
      command === "get_raw_pixels_preview" && args?.path === CROPPED ? new Promise(() => {}) : failing({})(command, args),
    );
    rawPixelsOf(render(file)).loadRawPixels(true);
    await settle();
    const tree = render(file);
    publish(tree, file, CROPPED, [2, 2]);
    const after = render(file);
    expect(rawPixelsOf(after).rawPixelsSource).toEqual({ key: `${INPUT}|0`, dims: [4, 2] });
  });

  it("does not stretch a product with the scale limits of the input while its own limits load or after they fail", async () => {
    const file = inputRow();
    invokeMock.mockImplementation(failing({ [`compute_scale_limits_cmd|${PRODUCT}`]: "all pixels are NaN" }));
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
    render(file);
    await settle();
    let tree = render(file);
    expect(displayOf(tree).limits).toEqual(limitsOf(0, 10));

    publish(tree, file, PRODUCT, [4, 2]);
    tree = render(file);
    expect(displayOf(tree).limits).toBeNull();

    await settle();
    tree = render(file);
    expect(displayOf(tree).limitsError).toBe("all pixels are NaN");
    expect(displayOf(tree).limits).toBeNull();
    errorLog.mockRestore();
  });

  it("asks for scale limits without the DQ-flagged pixels when Exclude DQ is on", async () => {
    const file = inputRow();
    invokeMock.mockImplementation(failing({}));
    const tree = render(file);
    contextWith<{ setExcludeDq: (v: boolean) => void }>(tree, "setExcludeDq").setExcludeDq(true);
    render(file);
    await settle();
    const requests = invokeMock.mock.calls.filter(([command]) => command === "compute_scale_limits_cmd").map(([, args]) => args?.excludeDq);
    expect(requests.at(-1)).toBe(true);
  });

  it("falls back to the default STF and says why when the histogram of a product fails", async () => {
    displayMemory.update(() => setDefaults(emptyDisplayMemory(null), MTF));
    const file = inputRow();
    invokeMock.mockImplementation(failing({ [`compute_histogram|${PRODUCT}`]: "histogram failed: empty plane" }));
    const errorLog = vi.spyOn(console, "error").mockImplementation(() => {});
    render(file);
    await settle();
    let tree = render(file);
    expect(stfOf(tree)).toEqual(INPUT_AUTO_STF);

    publish(tree, file, PRODUCT, [4, 2]);
    render(file);
    await settle();
    tree = render(file);

    expect(stfOf(tree)).toEqual(DEFAULT_STF);
    expect(displayOf(tree).histError).toBe("histogram failed: empty plane");
    errorLog.mockRestore();
  });
});
