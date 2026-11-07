import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { DqOverlaySettings, PlaneInfo, ProcessedFile, ProcessResult } from "../../shared/types";
import type { DisplayLook } from "../../shared/types/display";

vi.hoisted(() => {
  vi.stubGlobal("window", {
    setTimeout: (fn: () => void, ms?: number) => globalThis.setTimeout(fn, ms),
    clearTimeout: (handle: ReturnType<typeof setTimeout>) => globalThis.clearTimeout(handle),
    addEventListener: () => {},
    removeEventListener: () => {},
    devicePixelRatio: 1,
    innerWidth: 1600,
    innerHeight: 900,
    localStorage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
  });
});


const invokeMock = vi.hoisted(() => vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>());
const composite = vi.hoisted(() => ({
  park: vi.fn(),
  resetComposite: vi.fn(),
  initRgb: vi.fn(),
  setCompositePreviewUrl: vi.fn(),
  setCompositeStf: vi.fn(),
  setCompositeStfLinked: vi.fn(),
}));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../infrastructure/tauri/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../infrastructure/tauri/client")>()),
  typedInvoke: invokeMock,
}));
const compositePreview = vi.hoisted(() => ({ compositePreviewUrl: null as string | null }));
vi.mock("../CompositeContext", () => ({
  useCompositeActions: () => composite,
  useCompositePreview: () => compositePreview,
}));
vi.mock("../ComposeWizardContext", () => ({ useComposeWizardContext: () => ({ state: { compositeReady: false } }) }));

import { hooks } from "../../test/hookRuntime";
import { PreviewProvider, fileKeyOf, getRenderRecord, useCubeContext, useDisplayedImage, useRenderActions } from "../PreviewContext";
import { scientificInputRevisionOf } from "../../utils/pixelSource";
import type { ProcessedResult } from "../../shared/types/preview";
import { useAnalysisTarget } from "../../hooks/useAnalysisTarget";
import { displayMemory } from "../../utils/displayMemoryStore";
import { emptyDisplayMemory, setDefaults } from "../../utils/displayMemory";

const COLORMAP_LUT_COMMAND = "get_colormap_lut_cmd";
const LIMITS_WAIT_MS = 1000;

const ASINH_LOOK: DisplayLook = {
  stretch: "asinh",
  limits: "zscale",
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

const SPECTRUM_PATH = "C:/data/jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";
const IMAGE_PATH = "C:/data/jw02739001001_02105_00001_nrca1_cal.fits";

const SPECTRUM_ROW: ProcessedFile = {
  id: "spec",
  name: "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits",
  path: SPECTRUM_PATH,
  sourcePath: SPECTRUM_PATH,
  imageRef: null,
  size: 1,
  status: "spectrum",
  spectrum: { kind: "x1d", hdu: 1 },
  result: null,
  error: null,
  startedAt: 1,
  finishedAt: 2,
};

function planeOf(path: string): PlaneInfo {
  return {
    kind: "hdu",
    index: 1,
    key: null,
    extname: "SCI",
    extver: 1,
    is_dq: false,
    is_err: false,
    dq_ref: `${path}#hdu=3`,
    err_ref: `${path}#hdu=2`,
    source_path: path,
    dq_table: "jwst",
  };
}

function resultOf(path: string): ProcessResult {
  return {
    png_path: `${path}.png`,
    previewUrl: `asset://${path}.png`,
    dimensions: [8, 8],
    elapsed_ms: 1,
    header: { NAXIS: "3", NAXIS3: "4" },
    plane: planeOf(path),
  };
}

const IMAGE_ROW: ProcessedFile = {
  ...SPECTRUM_ROW,
  id: "img",
  name: "jw02739001001_02105_00001_nrca1_cal.fits",
  path: IMAGE_PATH,
  sourcePath: IMAGE_PATH,
  status: "done",
  spectrum: undefined,
  result: resultOf(IMAGE_PATH),
};

const SPECTRUM_ROW_WITH_RESULT: ProcessedFile = { ...SPECTRUM_ROW, id: "spec-with-result", result: resultOf(SPECTRUM_PATH) };

const RGB_IMAGE_ROW: ProcessedFile = {
  ...IMAGE_ROW,
  id: "rgb",
  result: { ...resultOf(IMAGE_PATH), header: { NAXIS: "3", NAXIS3: "3" }, is_rgb: true },
};

const LATE_CUBE_INFO = {
  width: 8,
  height: 8,
  frames: 4,
  spectral_classification: { is_spectral: true, reason: "WAVE axis" },
  ramp: { kind: "jwst_uncal", ngroups: 4, nints: 1 },
};

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

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

function renderPasses(file: ProcessedFile): ReactNode {
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

function mount(file: ProcessedFile): ReactNode {
  hooks.run.cells = [];
  return renderPasses(file);
}

function exercise(file: ProcessedFile): ReactNode {
  const first = mount(file);
  contextWith<{ loadRawPixels: (force?: boolean) => void }>(first, "loadRawPixels").loadRawPixels(true);
  contextWith<{ setOverlay: (patch: Partial<DqOverlaySettings>) => void }>(first, "setOverlay").setOverlay({ enabled: true, mask: 1 });
  const tree = renderPasses(file);
  vi.advanceTimersByTime(LIMITS_WAIT_MS);
  return tree;
}

function fileRequests(): string[] {
  return invokeMock.mock.calls.map(([command]) => command).filter((command) => command !== COLORMAP_LUT_COMMAND);
}

function requestedPaths(): unknown[] {
  return invokeMock.mock.calls.filter(([command]) => command !== COLORMAP_LUT_COMMAND).map(([, args]) => args?.path);
}

function publishScientificProduct(file: ProcessedFile, patch: Partial<ProcessedResult> = {}): void {
  const tree = mount(file);
  const actions = contextWith<ReturnType<typeof useRenderActions>>(tree, "publishProcessed");
  actions.publishProcessed(fileKeyOf(file) as string, {
    fitsPath: "C:/out/moment.fits",
    previewUrl: null,
    dimensions: [8, 8],
    label: "Moment 0",
    kind: "cube",
    inputPath: file.path,
    inputRevision: scientificInputRevisionOf(file),
    ...patch,
  });
  renderPasses(file);
}

function shownProduct(tree: ReactNode): ProcessedResult | null {
  return contextWith<{ processed: ProcessedResult | null }>(tree, "processed").processed;
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeMock.mockReset();
  invokeMock.mockImplementation(() => new Promise(() => {}));
  for (const action of Object.values(composite)) action.mockClear();
  displayMemory.update(() => setDefaults(emptyDisplayMemory(null), ASINH_LOOK));
});

afterEach(() => {
  compositePreview.compositePreviewUrl = null;
  vi.useRealTimers();
  displayMemory.update(() => emptyDisplayMemory(null));
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("DQ belongs to the displayed product", () => {
  const table = (path: string) => ({ table: "jwst", label: "JWST", flags: [], default_mask: 1, exclusion_mask: 1, dq_ref: `${path}#hdu=3`, err_ref: null, bits_present: 1 });

  it("discovers the derived product companions and requests its mask", async () => {
    invokeMock.mockImplementation((command, args) => command === "get_dq_flag_table_cmd"
      ? Promise.resolve(table(String(args?.path))) : new Promise(() => {}));
    const file = { ...IMAGE_ROW, id: "dq-derived" };
    publishScientificProduct(file, { dimensions: [4, 6] });
    await flushMicrotasks();
    const tree = renderPasses(file);
    const dq = contextWith<{ flagTable: { dq_ref: string }; setOverlay: (patch: Partial<DqOverlaySettings>) => void }>(tree, "flagTable");
    expect(dq.flagTable.dq_ref).toBe("C:/out/moment.fits#hdu=3");
    dq.setOverlay({ enabled: true });
    renderPasses(file);
    expect(invokeMock).toHaveBeenCalledWith("get_dq_mask_preview", expect.objectContaining({ path: "C:/out/moment.fits" }));
  });

  it("rejects a delayed source table after changing to a product without DQ", async () => {
    const answers: { path: string; resolve: (value: unknown) => void }[] = [];
    invokeMock.mockImplementation((command, args) => command === "get_dq_flag_table_cmd"
      ? new Promise((resolve) => { answers.push({ path: String(args?.path), resolve }); }) : new Promise(() => {}));
    const file = { ...IMAGE_ROW, id: "dq-late" };
    publishScientificProduct(file);
    expect(answers.map((a) => a.path)).toEqual([IMAGE_PATH, "C:/out/moment.fits"]);
    answers[1].resolve({ ...table("C:/out/moment.fits"), dq_ref: null });
    answers[0].resolve(table(IMAGE_PATH));
    await flushMicrotasks();
    const dq = contextWith<{ flagTable: { dq_ref: string | null }; dqMask: unknown }>(renderPasses(file), "flagTable");
    expect(dq.flagTable.dq_ref).toBeNull();
    expect(dq.dqMask).toBeNull();
  });

  it.each(["png", "composite"])("does not inherit DQ for a %s preview", async (kind) => {
    invokeMock.mockImplementation((command, args) => command === "get_dq_flag_table_cmd"
      ? Promise.resolve(table(String(args?.path))) : new Promise(() => {}));
    const file = { ...IMAGE_ROW, id: `dq-${kind}` };
    publishScientificProduct(file, kind === "png" ? { fitsPath: null, previewUrl: "asset://result.png" } : {});
    if (kind === "composite") compositePreview.compositePreviewUrl = "asset://composite.png";
    await flushMicrotasks();
    const dq = contextWith<{ flagTable: unknown; dqMask: unknown }>(renderPasses(file), "flagTable");
    expect(dq.flagTable).toBeNull();
    expect(dq.dqMask).toBeNull();
  });

  it("discards a late mask on product change and reloads the source after revert", async () => {
    const pending: { path: string; resolve: (value: unknown) => void }[] = [];
    invokeMock.mockImplementation((command, args) => command === "get_dq_flag_table_cmd"
      ? Promise.resolve(table(String(args?.path))) : command === "get_dq_mask_preview"
        ? new Promise((resolve) => { pending.push({ path: String(args?.path), resolve }); }) : new Promise(() => {}));
    const file = { ...IMAGE_ROW, id: "dq-mask-late" };
    let tree = mount(file);
    await flushMicrotasks();
    tree = renderPasses(file);
    contextWith<{ setOverlay: (patch: Partial<DqOverlaySettings>) => void }>(tree, "setOverlay").setOverlay({ enabled: true });
    tree = renderPasses(file);
    const actions = contextWith<ReturnType<typeof useRenderActions>>(tree, "publishProcessed");
    actions.publishProcessed(fileKeyOf(file) as string, { fitsPath: "C:/out/crop.fits", previewUrl: null, dimensions: [2, 3], label: "crop", kind: "cube", inputPath: file.path, inputRevision: scientificInputRevisionOf(file) });
    renderPasses(file);
    const bytes = new ArrayBuffer(20);
    const view = new DataView(bytes);
    view.setUint32(0, 2, true);
    view.setUint32(4, 2, true);
    view.setUint32(8, 1, true);
    pending[0].resolve(bytes);
    await flushMicrotasks();
    tree = renderPasses(file);
    expect(contextWith<{ dqMask: unknown }>(tree, "dqMask").dqMask).toBeNull();
    actions.resetProcessed();
    renderPasses(file);
    await flushMicrotasks();
    tree = renderPasses(file);
    expect(contextWith<{ flagTable: { dq_ref: string } }>(tree, "flagTable").flagTable.dq_ref).toBe(`${IMAGE_PATH}#hdu=3`);
  });

  it("clears the pending indicator when the overlay is disabled", () => {
    const file = { ...IMAGE_ROW, id: "dq-disabled-pending" };
    const tree = exercise(file);
    const dq = contextWith<{ dqMaskLoading: boolean; setOverlay: (patch: Partial<DqOverlaySettings>) => void }>(tree, "dqMaskLoading");
    expect(dq.dqMaskLoading).toBe(true);
    dq.setOverlay({ enabled: false });
    expect(contextWith<{ dqMaskLoading: boolean }>(renderPasses(file), "dqMaskLoading").dqMaskLoading).toBe(false);
  });

  interface DqState {
    dqRef: string | null;
    dqTarget: unknown;
    flagTable: { dq_ref: string | null } | null;
    flagTableLoading: boolean;
    flagTableError: string | null;
    retryFlagTable: () => void;
    overlay: DqOverlaySettings;
    setOverlay: (patch: Partial<DqOverlaySettings>) => void;
    dqMask: unknown;
    dqMaskLoading: boolean;
  }

  const dqState = (tree: ReactNode) => contextWith<DqState>(tree, "flagTable");
  const tableRequests = (path: string) => invokeMock.mock.calls.filter(([command, args]) => command === "get_dq_flag_table_cmd" && args?.path === path);
  const maskRequests = (path: string) => invokeMock.mock.calls.filter(([command, args]) => command === "get_dq_mask_preview" && args?.path === path);

  function publishNext(file: ProcessedFile, fitsPath: string, label: string): void {
    const actions = contextWith<ReturnType<typeof useRenderActions>>(renderPasses(file), "publishProcessed");
    actions.publishProcessed(fileKeyOf(file) as string, { fitsPath, previewUrl: null, dimensions: [8, 8], label, kind: "cube", inputPath: file.path, inputRevision: scientificInputRevisionOf(file) });
    renderPasses(file);
  }

  it("exposes a rejected product DQ table instead of reporting no DQ plane, and reads it again on retry", async () => {
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    let locked = true;
    invokeMock.mockImplementation((command, args) => command === "get_dq_flag_table_cmd"
      ? locked && args?.path === "C:/out/moment.fits" ? Promise.reject(new Error("output is locked")) : Promise.resolve(table(String(args?.path)))
      : new Promise(() => {}));
    const file = { ...IMAGE_ROW, id: "dq-table-error" };
    publishScientificProduct(file);
    await flushMicrotasks();
    const failed = dqState(renderPasses(file));
    expect(failed.flagTable).toBeNull();
    expect(failed.flagTableError).toBe("output is locked");
    expect(failed.flagTableLoading).toBe(false);
    expect(quiet).toHaveBeenCalled();
    locked = false;
    failed.retryFlagTable();
    renderPasses(file);
    await flushMicrotasks();
    const recovered = dqState(renderPasses(file));
    expect(recovered.flagTableError).toBeNull();
    expect(recovered.flagTable?.dq_ref).toBe("C:/out/moment.fits#hdu=3");
    expect(tableRequests("C:/out/moment.fits")).toHaveLength(2);
    quiet.mockRestore();
  });

  function maskBuffer(): ArrayBuffer {
    const bytes = new ArrayBuffer(20);
    const view = new DataView(bytes);
    view.setUint32(0, 2, true);
    view.setUint32(4, 2, true);
    view.setUint32(8, 1, true);
    return bytes;
  }

  it("keeps the painted overlay of the source when its failed DQ table is read again", async () => {
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    let sourceTableReads = 0;
    invokeMock.mockImplementation((command, args) => {
      if (command === "get_dq_mask_preview") return Promise.resolve(maskBuffer());
      if (command !== "get_dq_flag_table_cmd") return new Promise(() => {});
      if (args?.path === IMAGE_PATH && ++sourceTableReads === 2) return Promise.reject(new Error("source is locked"));
      return Promise.resolve(table(String(args?.path)));
    });
    const file = { ...IMAGE_ROW, id: "dq-source-retry" };
    mount(file);
    await flushMicrotasks();
    dqState(renderPasses(file)).setOverlay({ enabled: true });
    renderPasses(file);
    await flushMicrotasks();
    publishNext(file, "C:/out/moment.fits", "Moment 0");
    await flushMicrotasks();
    renderPasses(file);
    await flushMicrotasks();
    contextWith<ReturnType<typeof useRenderActions>>(renderPasses(file), "resetProcessed").resetProcessed();
    renderPasses(file);
    await flushMicrotasks();
    const failed = dqState(renderPasses(file));
    expect(failed.flagTableError).toBe("source is locked");
    expect(failed.overlay.enabled).toBe(true);
    expect(failed.dqMask).not.toBeNull();
    failed.retryFlagTable();
    renderPasses(file);
    await flushMicrotasks();
    const recovered = dqState(renderPasses(file));
    expect(recovered.flagTableError).toBeNull();
    expect(recovered.flagTable?.dq_ref).toBe(`${IMAGE_PATH}#hdu=3`);
    expect(recovered.overlay.enabled).toBe(true);
    expect(recovered.dqMask).toBe(failed.dqMask);
    expect(recovered.dqMaskLoading).toBe(false);
    expect(tableRequests(IMAGE_PATH)).toHaveLength(3);
    quiet.mockRestore();
  });

  it("reports a product table that is still being read and names the displayed product", () => {
    const file = { ...IMAGE_ROW, id: "dq-table-pending" };
    publishScientificProduct(file);
    const dq = dqState(renderPasses(file));
    expect(dq.dqRef).toBeNull();
    expect(dq.flagTableLoading).toBe(true);
    expect(dq.flagTableError).toBeNull();
    expect(dq.dqTarget).toEqual({ kind: "product", label: "Moment 0" });
  });

  it.each([
    ["the selected file", null, { kind: "file" }],
    ["a colour composite", "asset://composite.png", { kind: "composite" }],
  ])("names %s as the DQ target", (_name, compositeUrl, target) => {
    compositePreview.compositePreviewUrl = compositeUrl;
    const dq = dqState(mount({ ...IMAGE_ROW, id: `dq-target-${target.kind}` }));
    expect(dq.dqTarget).toEqual(target);
  });

  it("keeps the overlay on when another product is published and requests that product's mask", async () => {
    invokeMock.mockImplementation((command, args) => command === "get_dq_flag_table_cmd"
      ? Promise.resolve(table(String(args?.path))) : new Promise(() => {}));
    const file = { ...IMAGE_ROW, id: "dq-overlay-kept" };
    publishScientificProduct(file);
    await flushMicrotasks();
    dqState(renderPasses(file)).setOverlay({ enabled: true });
    renderPasses(file);
    expect(maskRequests("C:/out/moment.fits")).toHaveLength(1);
    publishNext(file, "C:/out/moment1.fits", "Moment 1");
    await flushMicrotasks();
    const dq = dqState(renderPasses(file));
    expect(dq.overlay.enabled).toBe(true);
    expect(maskRequests("C:/out/moment1.fits")).toHaveLength(1);
  });

  it("keeps the chosen flags while products share a flag table and restores the default for another table", async () => {
    invokeMock.mockImplementation((command, args) => {
      if (command !== "get_dq_flag_table_cmd") return new Promise(() => {});
      const path = String(args?.path);
      return Promise.resolve(path === "C:/out/raw.fits" ? { ...table(path), table: "unknown", label: "unknown (raw bits)", default_mask: 0 } : table(path));
    });
    const file = { ...IMAGE_ROW, id: "dq-overlay-mask" };
    publishScientificProduct(file);
    await flushMicrotasks();
    expect(dqState(renderPasses(file)).overlay.mask).toBe(1);
    dqState(renderPasses(file)).setOverlay({ enabled: true, mask: 6 });
    publishNext(file, "C:/out/moment1.fits", "Moment 1");
    await flushMicrotasks();
    expect(dqState(renderPasses(file)).overlay).toEqual({ enabled: true, mask: 6 });
    expect(invokeMock).toHaveBeenCalledWith("get_dq_mask_preview", expect.objectContaining({ path: "C:/out/moment1.fits", mask: 6 }));
    publishNext(file, "C:/out/raw.fits", "Raw ratio");
    await flushMicrotasks();
    expect(dqState(renderPasses(file)).overlay).toEqual({ enabled: true, mask: 0 });
  });
});

describe("PreviewProvider scientific cache validity without the Cube tool", () => {
  it("reloads cube metadata and rejects the previous probe after a same-path reload", async () => {
    const answers: ((value: unknown) => void)[] = [];
    invokeMock.mockImplementation((command) => command === "get_cube_info"
      ? new Promise((resolve) => { answers.push(resolve); })
      : new Promise(() => {}));
    const file = { ...IMAGE_ROW, id: "science-metadata" };
    mount(file);
    const reloaded = { ...file, reloads: 1 };
    renderPasses(reloaded);
    expect(answers).toHaveLength(2);
    answers[0](LATE_CUBE_INFO);
    await flushMicrotasks();
    expect(contextWith<{ cubeDims: unknown }>(renderPasses(reloaded), "cubeDims").cubeDims).toBeNull();
    answers[1]({ ...LATE_CUBE_INFO, width: 12 });
    await flushMicrotasks();
    expect(contextWith<{ cubeDims: { width: number } }>(renderPasses(reloaded), "cubeDims").cubeDims.width).toBe(12);
  });

  it("removes the stale product before rendering a reloaded input", () => {
    const file = { ...IMAGE_ROW, id: "science-reload" };
    publishScientificProduct(file);
    expect(shownProduct(renderPasses(file))?.fitsPath).toBe("C:/out/moment.fits");
    const tree = hooks.render(PreviewProvider, { file: { ...file, reloads: 1 }, doneFiles: [], children: null }) as ReactNode;
    expect(shownProduct(tree)).toBeNull();
    expect(getRenderRecord(fileKeyOf(file) as string)?.processed ?? null).toBeNull();
  });

  it("rejects a cached product after its input changed while another file was shown", () => {
    const file = { ...IMAGE_ROW, id: "science-away" };
    publishScientificProduct(file);
    renderPasses({ ...IMAGE_ROW, id: "other-file" });
    const tree = renderPasses({ ...file, reloads: 1 });
    expect(shownProduct(tree)).toBeNull();
    expect(getRenderRecord(fileKeyOf(file) as string)?.processed ?? null).toBeNull();
  });

  it("rejects an outdated cached product when the viewer mounts again", () => {
    const file = { ...IMAGE_ROW, id: "science-reopen" };
    publishScientificProduct(file);
    const tree = mount({ ...file, finishedAt: 100 });
    expect(shownProduct(tree)).toBeNull();
  });

  it("preserves a cached product for the unchanged scientific input", () => {
    const file = { ...IMAGE_ROW, id: "science-current" };
    publishScientificProduct(file);
    expect(shownProduct(mount(file))?.fitsPath).toBe("C:/out/moment.fits");
  });

  it("preserves processing products without a scientific revision", () => {
    const file = { ...IMAGE_ROW, id: "science-processing" };
    publishScientificProduct(file, { kind: "processing", inputRevision: undefined });
    expect(shownProduct(renderPasses({ ...file, reloads: 1 }))?.fitsPath).toBe("C:/out/moment.fits");
  });

  it("rejects an unversioned scientific snapshot", () => {
    const file = { ...IMAGE_ROW, id: "science-unversioned" };
    publishScientificProduct(file, { inputRevision: undefined });
    expect(shownProduct(renderPasses(file))).toBeNull();
  });

  it("keeps the history binding when invalidating a scientific product", () => {
    const file = { ...IMAGE_ROW, id: "science-history" };
    publishScientificProduct(file);
    const actions = contextWith<ReturnType<typeof useRenderActions>>(renderPasses(file), "publishProcessed");
    actions.setHistoryId(fileKeyOf(file) as string, "history-science");
    const tree = renderPasses({ ...file, reloads: 1 });
    expect(shownProduct(tree)).toBeNull();
    expect(getRenderRecord(fileKeyOf(file) as string)?.historyId).toBe("history-science");
    expect(contextWith<{ chain: { base: unknown } }>(tree, "chain").chain.base).toBeNull();
  });
});

describe("PreviewProvider with an image file (control)", () => {
  it("requests the histogram, scale limits, cube info, DQ table, DQ mask and raw pixels of the selected image", () => {
    exercise(IMAGE_ROW);
    expect(new Set(fileRequests())).toEqual(
      new Set(["compute_histogram", "get_cube_info", "get_dq_flag_table_cmd", "get_raw_pixels_preview", "get_dq_mask_preview", "compute_scale_limits_cmd"]),
    );
    expect(new Set(requestedPaths())).toEqual(new Set([IMAGE_PATH]));
  });

  it("does not expose the image as a spectrum", () => {
    const cube = contextWith<{ isSpectrum: boolean; isCube: boolean }>(mount(IMAGE_ROW), "isSpectralCube");
    expect(cube.isSpectrum).toBe(false);
    expect(cube.isCube).toBe(true);
  });
});

describe("PreviewProvider with a 1D spectrum file", () => {
  it("issues no image, histogram, cube, DQ, scale-limit or raw-pixel request for a spectrum row", () => {
    exercise(SPECTRUM_ROW);
    expect(fileRequests()).toEqual([]);
  });

  it("issues none either when the spectrum row carries an image-like result with NAXIS3 and a DQ plane", () => {
    exercise(SPECTRUM_ROW_WITH_RESULT);
    expect(fileRequests()).toEqual([]);
  });

  it("exposes isSpectrum, never isCube, and marks the cube flags as describing the spectrum file", () => {
    const cube = contextWith<{ isSpectrum: boolean; isCube: boolean; ramp: unknown; cubeFileKey: string | null }>(mount(SPECTRUM_ROW), "isSpectralCube");
    expect(cube.isSpectrum).toBe(true);
    expect(cube.isCube).toBe(false);
    expect(cube.ramp).toBeNull();
    expect(cube.cubeFileKey).toBe(`spec|${SPECTRUM_PATH}`);
  });

  it("shows no image path for a spectrum, so the measuring tools and the status strip have nothing to request", () => {
    let shown: ReturnType<typeof useDisplayedImage> | null = null;
    let cubeFlags: ReturnType<typeof useCubeContext> | null = null;
    function Probe() {
      shown = useDisplayedImage();
      cubeFlags = useCubeContext();
      return null;
    }
    const render = (file: ProcessedFile) => {
      renderToStaticMarkup(createElement(PreviewProvider, { file, doneFiles: [], children: createElement(Probe) }));
      return { shown: shown as ReturnType<typeof useDisplayedImage> | null, cube: cubeFlags as ReturnType<typeof useCubeContext> | null };
    };
    const spectrum = render(SPECTRUM_ROW);
    expect(spectrum.shown?.path).toBeNull();
    expect(spectrum.shown?.dimensions).toBeNull();
    expect(spectrum.shown?.previewUrl).toBeNull();
    expect(spectrum.cube?.isSpectrum).toBe(true);
    const image = render(IMAGE_ROW);
    expect(image.shown?.path).toBe(IMAGE_PATH);
    expect(image.cube?.isSpectrum).toBe(false);
  });
});

describe("PreviewProvider switching from an image to a 1D spectrum", () => {
  it("resets the previous RGB file's colour view, so that image does not stay on screen for the spectrum", () => {
    mount(RGB_IMAGE_ROW);
    expect(composite.setCompositePreviewUrl).toHaveBeenCalledWith(RGB_IMAGE_ROW.result?.previewUrl);
    for (const action of Object.values(composite)) action.mockClear();
    renderPasses(SPECTRUM_ROW);
    expect(composite.resetComposite).toHaveBeenCalledTimes(1);
    expect(composite.park).not.toHaveBeenCalled();
    expect(composite.setCompositePreviewUrl).not.toHaveBeenCalled();
    expect(composite.initRgb).not.toHaveBeenCalled();
  });

  it("drops a late cube probe of the previous file instead of writing its cube flags onto the spectrum", async () => {
    let answerCubeInfo: (info: unknown) => void = () => {};
    invokeMock.mockImplementation((command) =>
      command === "get_cube_info"
        ? new Promise((resolve) => {
            answerCubeInfo = resolve;
          })
        : new Promise(() => {}),
    );
    mount(IMAGE_ROW);
    expect(fileRequests()).toContain("get_cube_info");
    renderPasses(SPECTRUM_ROW);
    answerCubeInfo(LATE_CUBE_INFO);
    await flushMicrotasks();
    const cube = contextWith<{ isSpectrum: boolean; isCube: boolean; isSpectralCube: boolean; cubeDims: unknown; ramp: unknown }>(
      renderPasses(SPECTRUM_ROW),
      "isSpectralCube",
    );
    expect(cube.isSpectrum).toBe(true);
    expect(cube.isCube).toBe(false);
    expect(cube.isSpectralCube).toBe(false);
    expect(cube.cubeDims).toBeNull();
    expect(cube.ramp).toBeNull();
  });
});

describe("useAnalysisTarget with a colour composite on screen over a selected file", () => {
  const COMPOSITE_URL = "asset://composite.png";

  function targetFor(file: ProcessedFile): ReturnType<typeof useAnalysisTarget> | null {
    let target: ReturnType<typeof useAnalysisTarget> | null = null;
    function Probe() {
      target = useAnalysisTarget();
      return null;
    }
    compositePreview.compositePreviewUrl = COMPOSITE_URL;
    renderToStaticMarkup(createElement(PreviewProvider, { file, doneFiles: [], children: createElement(Probe) }));
    return target;
  }

  it("gives the spectrum file's path to nothing, so the status strip sends no WCS request for it", () => {
    for (const row of [SPECTRUM_ROW, SPECTRUM_ROW_WITH_RESULT]) {
      const target = targetFor(row);
      expect(target?.composite).toBe(true);
      expect(target?.path).toBeNull();
      expect(target?.dimensions).toBeNull();
      expect(target?.rgbPath).toBeNull();
    }
  });

  it("still measures the composite on the selected image's grid (control)", () => {
    const target = targetFor(IMAGE_ROW);
    expect(target?.composite).toBe(true);
    expect(target?.path).toBe(IMAGE_PATH);
    expect(target?.dimensions).toEqual([8, 8]);
  });
});
