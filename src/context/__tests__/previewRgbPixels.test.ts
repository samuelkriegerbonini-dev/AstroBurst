import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactNode } from "react";
import type { ProcessedFile, RawRgbPixelData } from "../../shared/types";

const viewport = vi.hoisted(() => ({ innerWidth: 1000, resize: null as null | (() => void) }));

vi.hoisted(() => {
  vi.stubGlobal("window", {
    setTimeout: (fn: () => void, ms?: number) => globalThis.setTimeout(fn, ms),
    clearTimeout: (handle: ReturnType<typeof setTimeout>) => globalThis.clearTimeout(handle),
    addEventListener: (type: string, listener: () => void) => {
      if (type === "resize") viewport.resize = listener;
    },
    removeEventListener: () => {},
    devicePixelRatio: 1,
    get innerWidth() {
      return viewport.innerWidth;
    },
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
import { PreviewProvider } from "../PreviewContext";

const RGB = "C:/data/runtime-rgb.fits";
const CURVES = "C:/history/344da451b072ae3a/n0001.fits";

const RGB_ROW: ProcessedFile = {
  id: "rgb",
  name: "runtime-rgb.fits",
  path: RGB,
  sourcePath: RGB,
  imageRef: null,
  size: 1,
  status: "done",
  result: { png_path: "C:/out/runtime-rgb.png", previewUrl: "asset://runtime-rgb.png", dimensions: [2, 1], elapsed_ms: 1, header: { NAXIS: "3", NAXIS3: "3" }, is_rgb: true },
  error: null,
  startedAt: 1,
  finishedAt: 2,
};

function rgbBuffer(): ArrayBuffer {
  const buf = new ArrayBuffer(32 + 2 * 4 * 3);
  const view = new DataView(buf);
  view.setUint32(0, 2, true);
  view.setUint32(4, 1, true);
  [0.1, 0.9, 0.2, 0.8, 0.3, 0.7].forEach((v, i) => view.setFloat32(8 + i * 4, v, true));
  [0.1, 0.9, 0.2, 0.8, 0.3, 0.7].forEach((v, i) => view.setFloat32(32 + i * 4, v, true));
  return buf;
}

interface RgbPixelsValue {
  rgbRawPixels: RawRgbPixelData | null;
  loadRgbRawPixels: (source: string | null, force?: boolean, displayReferred?: boolean) => void;
}

function rgbPixelsValue(tree: ReactNode): RgbPixelsValue {
  let current: ReactNode = tree;
  while (isValidElement<{ value?: unknown; children?: ReactNode }>(current)) {
    const value = current.props.value as Record<string, unknown> | undefined;
    if (value && "loadRgbRawPixels" in value) return value as unknown as RgbPixelsValue;
    current = current.props.children;
  }
  throw new Error("no raw pixels context value");
}

function renderProvider(): ReactNode {
  let tree: ReactNode = null;
  hooks.run.active = true;
  try {
    for (let pass = 0; pass < 2; pass++) {
      hooks.run.cursor = 0;
      hooks.run.effects = [];
      tree = PreviewProvider({ file: RGB_ROW, doneFiles: [], children: null });
      for (const effect of hooks.run.effects) effect();
    }
  } finally {
    hooks.run.active = false;
  }
  return tree;
}

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

const rgbRequests = () => invokeMock.mock.calls.filter(([command]) => command === "get_raw_rgb_pixels_preview").map(([, args]) => args);

beforeEach(() => {
  vi.useFakeTimers();
  viewport.innerWidth = 1000;
  viewport.resize = null;
  hooks.run.cells = [];
  invokeMock.mockReset().mockImplementation((command) =>
    command === "get_raw_rgb_pixels_preview" ? Promise.resolve(rgbBuffer()) : new Promise(() => {}),
  );
});

afterEach(() => {
  vi.useRealTimers();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("PreviewProvider loads RGB pixels with the domain of the product it shows", () => {
  it("reads a display-referred RGB result on 0..1 and keeps that reading when a resize reloads it", async () => {
    rgbPixelsValue(renderProvider()).loadRgbRawPixels(CURVES, true, true);
    await flushMicrotasks();
    const loaded = rgbPixelsValue(renderProvider()).rgbRawPixels;
    expect(rgbRequests()).toEqual([{ path: CURVES, maxDim: 1000 }]);
    expect(loaded?.displayReferred).toBe(true);
    expect([loaded?.r.min, loaded?.r.max, loaded?.b.min, loaded?.b.max]).toEqual([0, 1, 0, 1]);

    viewport.innerWidth = 1600;
    viewport.resize?.();
    vi.advanceTimersByTime(300);
    await flushMicrotasks();
    expect(rgbRequests().at(-1)).toEqual({ path: CURVES, maxDim: 1600 });
    expect(rgbPixelsValue(renderProvider()).rgbRawPixels?.displayReferred).toBe(true);
  });

  it("keeps the RGB file's own channel ranges when it is not display-referred", async () => {
    rgbPixelsValue(renderProvider()).loadRgbRawPixels(RGB, true);
    await flushMicrotasks();
    const loaded = rgbPixelsValue(renderProvider()).rgbRawPixels;
    expect(rgbRequests()).toEqual([{ path: RGB, maxDim: 1000 }]);
    expect(loaded?.displayReferred).toBe(false);
    expect([loaded?.r.min, loaded?.r.max]).toEqual([Math.fround(0.1), Math.fround(0.9)]);
  });
});
