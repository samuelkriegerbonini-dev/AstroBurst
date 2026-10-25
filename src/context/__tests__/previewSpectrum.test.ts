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
import { PreviewProvider, useCubeContext, useDisplayedImage } from "../PreviewContext";
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
