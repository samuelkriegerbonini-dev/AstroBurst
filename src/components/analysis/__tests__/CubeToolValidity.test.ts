import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isValidElement, type ReactElement, type ReactNode } from "react";
import type { ProcessedResult } from "../../../shared/types/preview";
import type { CubeResult } from "../SpectroscopyPanel";

const state = vi.hoisted(() => ({
  file: { id: "first", path: "cube.fits" } as { id: string; path: string; reloads?: number; finishedAt?: number; imageRef?: string },
  processed: null as ProcessedResult | null,
  preview: vi.fn(),
  publish: vi.fn(),
  reset: vi.fn(),
  currentFileKey: vi.fn(),
}));

vi.mock("react", async (original) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await original()));
vi.mock("../../../context/PreviewContext", () => ({
  fileKeyOf: (file: { id: string; path: string }) => `${file.id}|${file.path}`,
  getRenderRecord: () => ({ processed: state.processed }),
  useFileContext: () => ({ file: state.file }),
  useCubeContext: () => ({ isCube: true, isSpectrum: false, cubeDims: { width: 8, height: 8, frames: 4 }, ramp: { nints: 1, ngroups: 4 }, rampSource: "fits" }),
  useRenderActions: () => ({
    currentFileKey: state.currentFileKey,
    publishProcessed: state.publish,
    resetProcessed: state.reset,
  }),
}));
vi.mock("../../../hooks/useSpectrumStore", () => ({ useSpectrum: () => ({}) }));
vi.mock("../../../hooks/useSpectrum1dStore", () => ({ useOpenSpectrumCsv: () => null, useSpectrum1dDoc: () => ({ spectrum: null, error: null }) }));
vi.mock("../../../infrastructure/tauri", () => ({ getPreviewUrl: state.preview }));

import { hooks } from "../../../test/hookRuntime";
import CubeTool from "../CubeTool";

type Props = Record<string, unknown>;

function find(node: ReactNode, match: (props: Props) => boolean): ReactElement<Props> | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = find(child, match);
      if (found) return found;
    }
  } else if (isValidElement<Props>(node)) {
    if (match(node.props)) return node;
    return find(node.props.children as ReactNode, match);
  }
  return null;
}

function panels() {
  const tree = hooks.render((CubeTool as unknown as { type: () => ReactNode }).type, {}) as ReactNode;
  const spectrum = find(tree, (props) => typeof props.onFramePreview === "function");
  const ramp = find(tree, (props) => props.ramp !== undefined);
  if (!spectrum || !ramp) throw new Error("Missing cube panels");
  return { spectrum, ramp };
}

const result: CubeResult = { fitsPath: "slope.fits", previewUrl: "slope.png", label: "Quick slope", dimensions: [8, 8] };

function invalidate(panel: ReactElement<Props>, paths: string[]) {
  (panel.props.onInvalidateCubeResult as (paths: string[]) => void)(paths);
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}

beforeEach(() => {
  hooks.mount();
  state.file = { id: "first", path: "cube.fits" };
  state.processed = null;
  state.currentFileKey.mockReset().mockImplementation(() => `${state.file.id}|${state.file.path}`);
  state.preview.mockReset().mockResolvedValue("frame.png");
  state.publish.mockReset().mockImplementation((_key: string, value: ProcessedResult) => { state.processed = value; });
  state.reset.mockReset().mockImplementation(() => { state.processed = null; });
});

afterEach(() => hooks.mount());

describe("CubeTool scientific product validity", () => {
  it.each(["spectrum", "ramp"] as const)("removes the exact invalidated %s product from the current viewer", (name) => {
    const panel = panels()[name];
    (panel.props.onCubeResult as (value: CubeResult) => void)(result);
    invalidate(panel, ["other.fits", "slope.fits"]);
    expect(state.reset).toHaveBeenCalledTimes(1);
    expect(state.processed).toBeNull();
  });

  it("keeps a different current product when an older analysis is invalidated", () => {
    const panel = panels().spectrum;
    (panel.props.onCubeResult as (value: CubeResult) => void)(result);
    invalidate(panel, ["old_moment.fits"]);
    expect(state.reset).not.toHaveBeenCalled();
    expect(state.processed?.fitsPath).toBe("slope.fits");
  });

  it("does not remove a processing product with a matching output path", () => {
    const panel = panels().spectrum;
    state.processed = { ...result, kind: "processing", inputPath: "cube.fits" };
    invalidate(panel, ["slope.fits"]);
    expect(state.reset).not.toHaveBeenCalled();
  });

  it("does not let an old file callback clear the current file", () => {
    const panel = panels().spectrum;
    state.file = { id: "second", path: "other.fits" };
    const next = panels().spectrum;
    (next.props.onCubeResult as (value: CubeResult) => void)(result);
    invalidate(panel, ["slope.fits"]);
    expect(state.reset).not.toHaveBeenCalled();
  });

  it("remounts both scientific panels when the same path is reopened with a new identity", () => {
    const before = panels();
    state.file = { ...state.file, id: "reloaded" };
    const after = panels();
    expect(after.spectrum.key).not.toBe(before.spectrum.key);
    expect(after.ramp.key).not.toBe(before.ramp.key);
  });

  it.each([{ reloads: 1 }, { finishedAt: 123 }, { imageRef: "updated.fits" }])("clears the old product and remounts both panels for an updated input %o", (change) => {
    const before = panels();
    (before.spectrum.props.onCubeResult as (value: CubeResult) => void)(result);
    state.file = { ...state.file, ...change };
    const after = panels();
    expect(after.spectrum.key).not.toBe(before.spectrum.key);
    expect(after.ramp.key).not.toBe(before.ramp.key);
    expect(state.processed).toBeNull();
  });

  it("does not publish through a callback from before an input reload", () => {
    const before = panels();
    state.file = { ...state.file, reloads: 1 };
    panels();
    (before.spectrum.props.onCubeResult as (value: CubeResult) => void)(result);
    expect(state.publish).not.toHaveBeenCalled();
  });

  it("rejects the cached product when its input reloads while another file is selected", () => {
    const original = state.file;
    const panel = panels().spectrum;
    (panel.props.onCubeResult as (value: CubeResult) => void)(result);
    const cached = state.processed;
    state.file = { id: "second", path: "other.fits" };
    state.processed = null;
    panels();
    state.file = { ...original, reloads: 1 };
    state.processed = cached;
    panels();
    expect(state.processed).toBeNull();
    expect(state.reset).toHaveBeenCalledTimes(1);
  });

  it("rejects a cached product after an input reload while the tool was closed", () => {
    const panel = panels().spectrum;
    (panel.props.onCubeResult as (value: CubeResult) => void)(result);
    hooks.mount();
    state.file = { ...state.file, reloads: 1 };
    panels();
    expect(state.processed).toBeNull();
  });

  it("keeps a cached product after returning to the unchanged input", () => {
    const original = state.file;
    const panel = panels().spectrum;
    (panel.props.onCubeResult as (value: CubeResult) => void)(result);
    const cached = state.processed;
    state.file = { id: "second", path: "other.fits" };
    state.processed = null;
    panels();
    state.file = original;
    state.processed = cached;
    panels();
    expect(state.processed).toBe(cached);
    expect(state.reset).not.toHaveBeenCalled();
  });

  it("invalidates an unversioned scientific snapshot when the tool is opened", () => {
    state.processed = { ...result, kind: "cube", inputPath: "cube.fits" };
    panels();
    expect(state.processed).toBeNull();
  });

  it("keeps an unversioned processing snapshot when the tool is opened", () => {
    const cached: ProcessedResult = { ...result, kind: "processing", inputPath: "cube.fits" };
    state.processed = cached;
    panels();
    expect(state.processed).toBe(cached);
    expect(state.reset).not.toHaveBeenCalled();
  });

  it("keeps a pending frame preview valid across an unrelated rerender", async () => {
    const pending = deferred<string>();
    state.preview.mockReturnValue(pending.promise);
    const panel = panels().spectrum;
    const loading = (panel.props.onFramePreview as (path: string, index: number) => Promise<void>)("frame.png", 1);
    panels();
    pending.resolve("current_frame.png");
    await loading;
    expect(state.publish).toHaveBeenCalledTimes(1);
    expect(state.processed?.previewUrl).toBe("current_frame.png");
  });

  it("does not let a delayed frame preview replace a newer scientific product", async () => {
    const pending = deferred<string>();
    state.preview.mockReturnValue(pending.promise);
    const panel = panels().spectrum;
    const loading = (panel.props.onFramePreview as (path: string, index: number) => Promise<void>)("frame.png", 1);
    (panel.props.onCubeResult as (value: CubeResult) => void)(result);
    pending.resolve("late_frame.png");
    await loading;
    expect(state.publish).toHaveBeenCalledTimes(1);
    expect(state.processed?.fitsPath).toBe("slope.fits");
  });

  it("does not repopulate a previous file after switching away and back", async () => {
    const pending = deferred<string>();
    state.preview.mockReturnValue(pending.promise);
    const panel = panels().spectrum;
    const loading = (panel.props.onFramePreview as (path: string, index: number) => Promise<void>)("frame.png", 1);
    state.file = { id: "second", path: "other.fits" };
    panels();
    state.file = { id: "first", path: "cube.fits" };
    panels();
    pending.resolve("late_frame.png");
    await loading;
    expect(state.publish).not.toHaveBeenCalled();
  });

  it("keeps publishing a current frame preview", async () => {
    const panel = panels().spectrum;
    await (panel.props.onFramePreview as (path: string, index: number, fits: string) => Promise<void>)("frame.png", 1, "frame.fits");
    expect(state.publish).toHaveBeenCalledWith("first|cube.fits", expect.objectContaining({ fitsPath: "frame.fits", frameIndex: 1, kind: "cube" }));
  });
});
