import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ProcessedFile } from "../../shared/types";

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
import { PreviewProvider } from "../PreviewContext";

function frame(name: string, header: Record<string, string>): ProcessedFile {
  const path = `C:/ngc7000/${name}`;
  return {
    id: name,
    name,
    path,
    sourcePath: path,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: `C:/out/${name}.png`, previewUrl: `asset://${name}.png`, dimensions: [8, 8], elapsed_ms: 1, header },
    error: null,
    startedAt: 1,
    finishedAt: 2,
  };
}

const HA = ["ha_1.fits", "ha_2.fits"].map((name) => frame(name, { IMAGETYP: "LIGHT", FILTER: "Ha" }));
const OIII = ["oiii_1.fits", "oiii_2.fits"].map((name) => frame(name, { IMAGETYP: "LIGHT", FILTER: "OIII" }));
const FLATS = [frame("flat_ha_1.fits", { IMAGETYP: "FLAT", FILTER: "Ha" }), frame("flat_oiii_1.fits", { IMAGETYP: "FLAT", FILTER: "OIII" })];
const DARK = frame("dark_300s_1.fits", { IMAGETYP: "DARK" });

function renderProvider(doneFiles: ProcessedFile[]): void {
  hooks.run.active = true;
  try {
    for (let pass = 0; pass < 2; pass++) {
      hooks.run.cursor = 0;
      hooks.run.effects = [];
      PreviewProvider({ file: doneFiles[0] ?? null, doneFiles, children: null });
      for (const effect of hooks.run.effects) effect();
    }
  } finally {
    hooks.run.active = false;
  }
}

const detections = () => invokeMock.mock.calls.filter(([command]) => command === "detect_narrowband_filters").map(([, args]) => args);

beforeEach(() => {
  vi.useFakeTimers();
  hooks.run.cells = [];
  invokeMock.mockReset().mockImplementation(() => new Promise(() => {}));
});

afterEach(() => {
  vi.useRealTimers();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("PreviewProvider narrowband detection", () => {
  it("reads the filters of the lights only, never of the flats and darks loaded with them", () => {
    renderProvider([HA[0], FLATS[0], OIII[0], DARK, HA[1], FLATS[1], OIII[1]]);
    vi.advanceTimersByTime(300);
    expect(detections()).toEqual([{ paths: [HA[0].path, OIII[0].path, HA[1].path, OIII[1].path], palette: "SHO" }]);
  });

  it("waits for two lights, however many calibration frames are loaded", () => {
    renderProvider([HA[0], ...FLATS, DARK]);
    vi.advanceTimersByTime(1000);
    expect(detections()).toEqual([]);
  });
});
