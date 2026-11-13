import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { ProcessedFile, StfParams } from "../../shared/types";
import type { DisplaySettings } from "../../shared/types/display";

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

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../infrastructure/tauri/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../infrastructure/tauri/client")>()),
  typedInvoke: () => new Promise(() => {}),
}));
vi.mock("../CompositeContext", () => ({
  useCompositeActions: () => ({ park: () => {}, resetComposite: () => {}, initRgb: () => {}, setCompositePreviewUrl: () => {}, setCompositeStf: () => {}, setCompositeStfLinked: () => {} }),
  useCompositePreview: () => ({ compositePreviewUrl: null }),
}));
vi.mock("../ComposeWizardContext", () => ({ useComposeWizardContext: () => ({ state: { compositeReady: false } }) }));

import { hooks } from "../../test/hookRuntime";
import { PreviewProvider } from "../PreviewContext";
import { displayMemory } from "../../utils/displayMemoryStore";
import { emptyDisplayMemory, fileEntry } from "../../utils/displayMemory";

const PATH = "C:/data/656nmos.fits";
const AUTO_STF: StfParams = { shadow: 0.012, midtone: 0.046, highlight: 1 };

const N656: ProcessedFile = {
  id: "656",
  name: "656nmos.fits",
  path: PATH,
  sourcePath: PATH,
  imageRef: null,
  size: 1,
  status: "done",
  result: {
    png_path: "C:/out/656nmos.png",
    previewUrl: "asset://656nmos.png",
    dimensions: [4, 4],
    elapsed_ms: 1,
    header: { INSTRUME: "WFPC2", FILTNAM1: "F656N" },
    histogram: { bins: [3, 2, 1], bin_count: 3, data_min: -12, data_max: 3194.12, median: 1, mean: 2, sigma: 3, mad: 1, total_pixels: 16, auto_stf: AUTO_STF },
  },
  error: null,
  startedAt: 1,
  finishedAt: 2,
};

interface DisplayValue {
  display: DisplaySettings;
  setDisplay: (patch: Partial<DisplaySettings>) => void;
}

function contextValue<T>(tree: ReactNode, key: string): T {
  let current: ReactNode = tree;
  while (isValidElement<{ value?: unknown; children?: ReactNode }>(current)) {
    const value = current.props.value as Record<string, unknown> | undefined;
    if (value && key in value) return value as T;
    current = current.props.children;
  }
  throw new Error(`no context value with ${key}`);
}

function renderProvider(): ReactNode {
  let tree: ReactNode = null;
  hooks.run.active = true;
  try {
    for (let pass = 0; pass < 2; pass++) {
      hooks.run.cursor = 0;
      hooks.run.effects = [];
      tree = PreviewProvider({ file: N656, doneFiles: [], children: null });
      for (const effect of hooks.run.effects) effect();
    }
  } finally {
    hooks.run.active = false;
  }
  return tree;
}

function edit(patch: (display: DisplaySettings) => Partial<DisplaySettings>): ReactNode {
  const { display, setDisplay } = contextValue<DisplayValue>(renderProvider(), "setDisplay");
  setDisplay(patch(display));
  return renderProvider();
}

const clickSymmetric = (display: DisplaySettings) => ({ symmetric: !display.symmetric });

beforeEach(() => {
  vi.useFakeTimers();
  hooks.run.cells = [];
  displayMemory.update(() => emptyDisplayMemory(null));
});

afterEach(() => {
  vi.useRealTimers();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("PreviewProvider Symmetric toggle on an mtf file", () => {
  it("switching Symmetric off gives back mtf, the auto STF and every other setting, and stores mtf for the file", () => {
    edit(() => ({ grid: true }));
    const opened = contextValue<DisplayValue>(edit(() => ({ compass: true })), "setDisplay").display;
    expect(opened).toMatchObject({ stretch: "mtf", limits: "minmax", symmetric: false, grid: true, compass: true });
    expect(contextValue<{ stfParams: StfParams }>(renderProvider(), "stfParams").stfParams).toEqual(AUTO_STF);

    const on = edit(clickSymmetric);
    expect(contextValue<DisplayValue>(on, "setDisplay").display).toEqual({ ...opened, symmetric: true, stretch: "linear" });

    const off = edit(clickSymmetric);
    expect(contextValue<DisplayValue>(off, "setDisplay").display).toEqual(opened);
    expect(contextValue<{ stfParams: StfParams }>(off, "stfParams").stfParams).toEqual(AUTO_STF);
    expect(fileEntry(displayMemory.get(), PATH)?.look).toMatchObject({ stretch: "mtf", limits: "minmax", symmetric: false });
  });
});
