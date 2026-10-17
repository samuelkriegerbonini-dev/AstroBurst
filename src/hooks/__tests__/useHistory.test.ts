import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HeadView, HistoryDoc, HistoryNode, HistoryState, OpSpec } from "../../shared/types/ops";
import type { ChainBase, ProcessedResult, ProcessingChain } from "../../shared/types/preview";

const { ops, announceMock, dockOpen } = vi.hoisted(() => ({
  dockOpen: vi.fn(),
  ops: {
    runOp: vi.fn(),
    getHistory: vi.fn(),
    setHead: vi.fn(),
    undo: vi.fn(),
    redo: vi.fn(),
    rerun: vi.fn(),
    rehydrate: vi.fn(),
    deleteBranch: vi.fn(),
    estimateNextNode: vi.fn(),
    getKernel: vi.fn(),
  },
  announceMock: vi.fn(),
}));

vi.mock("../../services/ops", () => ops);
vi.mock("../../components/dock/useDockDrag", () => ({ announce: announceMock }));
vi.mock("../../infrastructure/tauri", () => ({ getPreviewUrl: async (path: string) => `asset://${path}` }));
vi.mock("../useDockLayout", () => ({ dockStore: { dispatch: dockOpen } }));

import { EMPTY_CHAIN } from "../../utils/processingChain";
import { EMPTY_HISTORY_SNAPSHOT, historyStore } from "../../utils/historyStore";
import { runExclusive } from "../useProcessingRun";
import { adoptRun, loadHistory, moveHead, removeBranch, requestRerun, revertToOriginal, type HistoryRender, type HistoryTarget } from "../useHistory";
import { historyRerunStore } from "../../utils/historyRerunStore";

const DIR = "C:/app/history/3f9c0a1e5b7d2c44";
const SOURCE = "C:/data/nrca1_i2d.fits#hdu=1";
const KEY = `f1|${SOURCE}`;
const RECORDED_AT = "2026-10-16T12:00:00Z";
const PREVIEW_VERSION = Date.parse(RECORDED_AT);

const BACKGROUND: OpSpec = {
  op: "background",
  v: 1,
  model: "polynomial",
  config: { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" },
};
const DENOISE: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [3, 2.5, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const ARCSINH: OpSpec = { op: "arcsinh", v: 1, factor: 50 };
const ARCSINH_SOFT: OpSpec = { op: "arcsinh", v: 1, factor: 10 };
const TONE: OpSpec = { op: "tone", v: 1, levels: { black: 0.1, gamma: 1, white: 0.9 } };

function node(id: string, parent: string | null, op: OpSpec, present = true): HistoryNode {
  return {
    id,
    parent,
    op,
    mask: null,
    output: { path: `${id}.fits`, png: `${id}.png`, extras: [], dims: [2048, 2048], planes: 1, domain: "linear" },
    at: RECORDED_AT,
    elapsed_ms: 10,
    app: "0.6.9",
    threads: 8,
    present,
    bytes: 1,
    checkpoint: false,
  };
}

const NODES = [node("n0001", null, BACKGROUND), node("n0002", "n0001", DENOISE), node("n0003", "n0002", ARCSINH), node("n0004", "n0003", TONE)];

function doc(head: string | null, nodes: HistoryNode[] = NODES): HistoryDoc {
  return {
    format: "astroburst-history",
    version: 1,
    image: { id: "3f9c0a1e5b7d2c44", ref: SOURCE, size: 1, mtime: null },
    image_key: SOURCE,
    base_path: null,
    head,
    policy: { k: 4 },
    app: "0.6.9",
    threads: 8,
    masks: {},
    nodes,
  };
}

function state(head: string | null, nodes: HistoryNode[] = NODES, fingerprintMatches = true): HistoryState {
  return { image_id: "3f9c0a1e5b7d2c44", history_dir: DIR, root_path: SOURCE, doc: doc(head, nodes), read_only: null, fingerprint_matches: fingerprintMatches, busy: false };
}

function view(head: string | null, present = true): HeadView {
  const found = NODES.find((n) => n.id === head);
  return {
    image_id: "3f9c0a1e5b7d2c44",
    history_dir: DIR,
    head,
    node: found
      ? {
          id: found.id,
          parent: found.parent,
          op: found.op,
          fits_path: `${DIR}/${found.id}.fits`,
          png_path: `${DIR}/${found.id}.png`,
          extras: [],
          dimensions: [2048, 2048],
          planes: 1,
          domain: "linear",
          present,
        }
      : null,
    root_path: SOURCE,
    can_undo: head !== null,
    can_redo: true,
    node_count: NODES.length,
  };
}

interface Published {
  key: string;
  result: ProcessedResult;
  chain: ProcessingChain;
}

function harness(base: ChainBase | null = null) {
  const published: Published[] = [];
  const render: HistoryRender = {
    currentFileKey: () => KEY,
    publishProcessed: vi.fn((key: string, result: ProcessedResult, update?: (c: ProcessingChain) => ProcessingChain) => {
      published.push({ key, result, chain: update ? update(EMPTY_CHAIN) : EMPTY_CHAIN });
    }),
    setChain: vi.fn(),
    resetProcessed: vi.fn(),
    setHistoryId: vi.fn(),
  };
  const target: HistoryTarget = { fileKey: KEY, scope: { imageKey: SOURCE, basePath: base?.path ?? null }, base, rootPreviewUrl: "asset://source.png" };
  return { render, target, published };
}

beforeEach(() => {
  for (const fn of Object.values(ops)) fn.mockReset();
  announceMock.mockReset();
  historyStore.set(EMPTY_HISTORY_SNAPSHOT);
  historyRerunStore.clear();
  dockOpen.mockReset();
});

describe("undo and redo", () => {
  it("undo_publishes_the_parent_node_files_without_invoking_run_op", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });
    ops.undo.mockResolvedValue(view("n0003"));
    ops.getHistory.mockResolvedValue(state("n0003"));

    const moved = await moveHead(target, render, { kind: "undo" });

    expect(moved?.head).toBe("n0003");
    expect(ops.undo).toHaveBeenCalledWith(target.scope);
    expect(ops.runOp).not.toHaveBeenCalled();
    expect(ops.rerun).not.toHaveBeenCalled();
    expect(ops.rehydrate).not.toHaveBeenCalled();
    expect(published).toHaveLength(1);
    expect(published[0].key).toBe(KEY);
    expect(published[0].result).toMatchObject({ fitsPath: `${DIR}/n0003.fits`, inputPath: `${DIR}/n0002.fits`, label: "Arcsinh Stretch", kind: "processing" });
    expect(published[0].result.previewUrl).toBe(`asset://${DIR}/n0003.png?v=${PREVIEW_VERSION}`);
    expect(published[0].chain.steps.background?.previewUrl).toBe(`asset://${DIR}/n0001.png?v=${PREVIEW_VERSION}`);
    expect(Object.keys(published[0].chain.steps).sort()).toEqual(["background", "denoise", "stretch"]);
    expect(render.setHistoryId).toHaveBeenCalledWith(KEY, "3f9c0a1e5b7d2c44");
    expect(historyStore.get().state?.doc?.head).toBe("n0003");
    expect(announceMock).toHaveBeenCalledWith("Undo: Curves");
  });

  it("redo_publishes_the_most_recent_child", async () => {
    const branched = [...NODES.slice(0, 3), node("n0005", "n0003", TONE), node("n0004", "n0003", ARCSINH_SOFT)];
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0003", branched) });
    ops.redo.mockResolvedValue({ ...view(null), head: "n0005", node: { ...view("n0004").node!, id: "n0005", parent: "n0003", fits_path: `${DIR}/n0005.fits`, png_path: `${DIR}/n0005.png` } });
    ops.getHistory.mockResolvedValue(state("n0005", branched));

    await moveHead(target, render, { kind: "redo" });

    expect(ops.redo).toHaveBeenCalledWith(target.scope);
    expect(ops.runOp).not.toHaveBeenCalled();
    expect(published[0].result.fitsPath).toBe(`${DIR}/n0005.fits`);
    expect(published[0].chain.steps.tone?.fitsPath).toBe(`${DIR}/n0005.fits`);
    expect(announceMock).toHaveBeenCalledWith("Redo: Curves");
  });

  it("undo to the original clears the shown result and keeps the history binding", async () => {
    const { render, target, published } = harness();
    ops.undo.mockResolvedValue(view(null));
    ops.getHistory.mockResolvedValue(state(null));

    await moveHead(target, render, { kind: "undo" });

    expect(published).toHaveLength(0);
    expect(render.resetProcessed).toHaveBeenCalledTimes(1);
    expect(render.setHistoryId).toHaveBeenCalledWith(KEY, "3f9c0a1e5b7d2c44");
  });

  it("going to the root of a rebased history shows the chain base", async () => {
    const base: ChainBase = { path: "C:/app/output/stack.fits", label: "Stack (mean)", previewUrl: "asset://stack.png", dimensions: [2048, 2048], kind: "stacking", inputPath: SOURCE };
    const { render, target, published } = harness(base);
    ops.setHead.mockResolvedValue(view(null));
    ops.getHistory.mockResolvedValue(state(null));

    await moveHead(target, render, { kind: "to", node: null });

    expect(ops.setHead).toHaveBeenCalledWith(target.scope, null);
    expect(render.resetProcessed).not.toHaveBeenCalled();
    expect(published[0].result).toMatchObject({ fitsPath: base.path, label: base.label, kind: "stacking" });
    expect(published[0].chain.base).toBe(base);
  });

  it("rebuilds an evicted node before showing it", async () => {
    const { render, target, published } = harness();
    ops.setHead.mockResolvedValue(view("n0002", false));
    ops.rehydrate.mockResolvedValue({ image_id: "x", head: "n0002", node: view("n0002").node, rebuilt: ["n0002"] });
    ops.getHistory.mockResolvedValue(state("n0002"));

    await moveHead(target, render, { kind: "to", node: "n0002" });

    expect(ops.rehydrate).toHaveBeenCalledWith(target.scope, "n0002");
    expect(ops.runOp).not.toHaveBeenCalled();
    expect(published[0].result.fitsPath).toBe(`${DIR}/n0002.fits`);
  });

  it("announces the refusal and publishes nothing when there is nothing to undo", async () => {
    const { render, target, published } = harness();
    ops.undo.mockRejectedValue(new Error("nothing to undo: the original image is already shown"));

    expect(await moveHead(target, render, { kind: "undo" })).toBeNull();

    expect(published).toHaveLength(0);
    expect(announceMock).toHaveBeenCalledWith("nothing to undo: the original image is already shown");
    expect(historyStore.get().error).toBe("nothing to undo: the original image is already shown");
  });

  it("Ctrl+Z on a file with no history yet is announced without asking Rust and without an error", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: { ...state(null), doc: null } });

    expect(await moveHead(target, render, { kind: "undo" })).toBeNull();
    expect(await moveHead(target, render, { kind: "redo" })).toBeNull();

    expect(ops.undo).not.toHaveBeenCalled();
    expect(ops.redo).not.toHaveBeenCalled();
    expect(published).toHaveLength(0);
    expect(announceMock.mock.calls.map((c) => c[0])).toEqual(["this image has no processing history yet", "this image has no processing history yet"]);
    expect(historyStore.get().error).toBeNull();
  });

  it("Ctrl+Z at the original is announced without asking Rust and without an error", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state(null) });

    expect(await moveHead(target, render, { kind: "undo" })).toBeNull();

    expect(ops.undo).not.toHaveBeenCalled();
    expect(published).toHaveLength(0);
    expect(render.resetProcessed).not.toHaveBeenCalled();
    expect(announceMock).toHaveBeenCalledWith("nothing to undo: the original image is already shown");
    expect(historyStore.get().error).toBeNull();
    expect(historyStore.get().state?.doc?.head).toBeNull();
  });

  it("Ctrl+Shift+Z on the newest step is announced without asking Rust and without an error", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });

    expect(await moveHead(target, render, { kind: "redo" })).toBeNull();

    expect(ops.redo).not.toHaveBeenCalled();
    expect(published).toHaveLength(0);
    expect(announceMock).toHaveBeenCalledWith("nothing to redo from the current step");
    expect(historyStore.get().error).toBeNull();
  });

  it("redo from the original still asks Rust for the newest first step", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state(null) });
    ops.redo.mockResolvedValue(view("n0001"));
    ops.getHistory.mockResolvedValue(state("n0001"));

    expect((await moveHead(target, render, { kind: "redo" }))?.head).toBe("n0001");

    expect(ops.redo).toHaveBeenCalledWith(target.scope);
    expect(published[0].result.fitsPath).toBe(`${DIR}/n0001.fits`);
  });

  it("a running step is reported before nothing to undo", async () => {
    const { render, target } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state(null) });
    let release: () => void = () => {};
    const running = runExclusive("tone", KEY, () => new Promise<void>((resolve) => { release = resolve; }));

    expect(await moveHead(target, render, { kind: "undo" })).toBeNull();

    expect(ops.undo).not.toHaveBeenCalled();
    expect(announceMock.mock.calls.map((c) => c[0])).toEqual(["A step is still running on this image"]);
    release();
    await running;
  });

  it("a second Ctrl+Z pressed while the first head move runs waits for it instead of being dropped", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });
    let releaseFirst: () => void = () => {};
    ops.undo.mockImplementationOnce(() => new Promise<HeadView>((resolve) => { releaseFirst = () => resolve(view("n0003")); }));
    ops.undo.mockResolvedValueOnce(view("n0002"));
    ops.getHistory.mockResolvedValueOnce(state("n0003")).mockResolvedValueOnce(state("n0002"));

    const first = moveHead(target, render, { kind: "undo" });
    const second = moveHead(target, render, { kind: "undo" });
    await vi.waitFor(() => expect(ops.undo).toHaveBeenCalledTimes(1));
    releaseFirst();

    expect((await first)?.head).toBe("n0003");
    expect((await second)?.head).toBe("n0002");
    expect(announceMock).not.toHaveBeenCalledWith("A step is still running on this image");
    expect(announceMock.mock.calls.map((c) => c[0])).toEqual(["Undo: Curves", "Undo: Arcsinh Stretch"]);
    expect(published.map((p) => p.result.fitsPath)).toEqual([`${DIR}/n0003.fits`, `${DIR}/n0002.fits`]);
    expect(historyStore.get().state?.doc?.head).toBe("n0002");
  });

  it("moves the Rust head back when the step it moved to cannot be shown", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });
    ops.undo.mockResolvedValue(view("n0003"));
    ops.getHistory.mockRejectedValue(new Error("the history of this image cannot be read"));
    ops.setHead.mockResolvedValue(view("n0004"));

    expect(await moveHead(target, render, { kind: "undo" })).toBeNull();

    expect(ops.setHead).toHaveBeenCalledWith(target.scope, "n0004");
    expect(published).toHaveLength(0);
    expect(historyStore.get().state?.doc?.head).toBe("n0004");
    expect(historyStore.get().error).toBe("the history of this image cannot be read");
  });

  it("waits for a running step: a held file lock refuses the head move", async () => {
    const { render, target, published } = harness();
    let release: () => void = () => {};
    const running = runExclusive("tone", KEY, () => new Promise<void>((resolve) => { release = resolve; }));

    expect(await moveHead(target, render, { kind: "undo" })).toBeNull();
    expect(ops.undo).not.toHaveBeenCalled();
    expect(published).toHaveLength(0);
    expect(announceMock).toHaveBeenCalledWith("A step is still running on this image");

    release();
    await running;
    ops.undo.mockResolvedValue(view("n0003"));
    ops.getHistory.mockResolvedValue(state("n0003"));
    expect((await moveHead(target, render, { kind: "undo" }))?.head).toBe("n0003");
  });
});

describe("loading and recorded runs", () => {
  it("restores the head of a saved history when the file has no result yet", async () => {
    const { render, target, published } = harness();
    ops.getHistory.mockResolvedValue(state("n0004"));

    await loadHistory(target, render);

    expect(published[0].result.fitsPath).toBe(`${DIR}/n0004.fits`);
    expect(Object.keys(published[0].chain.steps)).toHaveLength(4);
    expect(historyStore.get()).toMatchObject({ fileKey: KEY, error: null, pending: false });
    expect(ops.runOp).not.toHaveBeenCalled();
  });

  it("keeps the original on screen when the source file changed since the history was recorded", async () => {
    const { render, target, published } = harness();
    ops.getHistory.mockResolvedValueOnce(state("n0004", NODES, false)).mockResolvedValueOnce(state(null, NODES, false));
    ops.setHead.mockResolvedValue(view(null));

    await loadHistory(target, render);

    expect(published).toHaveLength(0);
    expect(ops.rehydrate).not.toHaveBeenCalled();
    expect(ops.setHead).toHaveBeenCalledWith(target.scope, null);
    expect(render.setHistoryId).toHaveBeenCalledWith(KEY, "3f9c0a1e5b7d2c44");
    expect(historyStore.get().state).toMatchObject({ fingerprint_matches: false, doc: { head: null } });
  });

  it("does not restore a read-only history", async () => {
    const { render, target, published } = harness();
    ops.getHistory.mockResolvedValue({ ...state("n0004"), read_only: "written with format version 2" });

    await loadHistory(target, render);

    expect(published).toHaveLength(0);
    expect(historyStore.get().state?.read_only).toContain("version 2");
  });

  it("publishes the new head with the projected chain after a recorded run", async () => {
    const { render, target, published } = harness();
    ops.getHistory.mockResolvedValue(state("n0002", NODES.slice(0, 2)));

    await adoptRun(target, render);

    expect(published[0].result).toMatchObject({ fitsPath: `${DIR}/n0002.fits`, inputPath: `${DIR}/n0001.fits` });
    expect(published[0].chain.steps.background?.fitsPath).toBe(`${DIR}/n0001.fits`);
    expect(published[0].chain.steps.denoise?.fitsPath).toBe(`${DIR}/n0002.fits`);
    expect(render.setHistoryId).toHaveBeenCalledWith(KEY, "3f9c0a1e5b7d2c44");
  });

  it("publishes nothing for a run whose node the history does not hold", async () => {
    const { render, target, published } = harness();
    ops.getHistory.mockResolvedValue(state("n0002", NODES.slice(0, 2)));

    await adoptRun(target, render, "n0009");

    expect(published).toHaveLength(0);
    expect(render.setHistoryId).not.toHaveBeenCalled();
  });

  it("drops a run made on a chain base that was replaced while it ran", async () => {
    const base: ChainBase = { path: "C:/app/output/f120.fits", label: "Frame 120", previewUrl: null, dimensions: [64, 48], kind: "cube", inputPath: SOURCE };
    const { render, target, published } = harness(base);
    ops.getHistory.mockResolvedValue(state("n0001", NODES.slice(0, 1)));
    const record = { processed: null, chain: { steps: {}, psfKernel: null, base: { ...base, path: "C:/app/output/f121.fits" } }, version: 1, historyId: null };
    const spy = vi.spyOn(await import("../../context/PreviewContext"), "getRenderRecord").mockReturnValue(record);

    await adoptRun(target, render, "n0001");

    expect(published).toHaveLength(0);
    spy.mockRestore();
  });

  it("revert_to_original_keeps_the_history", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });
    ops.setHead.mockResolvedValue(view(null));
    ops.getHistory.mockResolvedValue(state(null));

    await revertToOriginal(target, render);

    expect(ops.setHead).toHaveBeenCalledWith(target.scope, null);
    expect(ops.deleteBranch).not.toHaveBeenCalled();
    expect(ops.runOp).not.toHaveBeenCalled();
    expect(published).toHaveLength(0);
    expect(render.resetProcessed).toHaveBeenCalledTimes(1);
    expect(render.setHistoryId).toHaveBeenCalledWith(KEY, "3f9c0a1e5b7d2c44");
    expect(historyStore.get().state?.doc?.nodes).toHaveLength(4);
    expect(historyStore.get().state?.doc?.head).toBeNull();
  });

  it("a second Revert to original clears the chain base and the provided PSF as before", async () => {
    const { render, target } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });
    ops.setHead.mockResolvedValue(view(null));
    ops.getHistory.mockResolvedValue(state(null));

    await revertToOriginal(target, render);
    expect(render.resetProcessed).toHaveBeenCalledTimes(1);
    await revertToOriginal(target, render);

    expect(ops.setHead).toHaveBeenCalledTimes(1);
    expect(render.resetProcessed).toHaveBeenCalledTimes(2);
  });

  it("revert without a history clears the shown result as before", async () => {
    const { render, target } = harness();

    await revertToOriginal(target, render);

    expect(render.resetProcessed).toHaveBeenCalledTimes(1);
    expect(ops.setHead).not.toHaveBeenCalled();
    expect(ops.getHistory).not.toHaveBeenCalled();
  });

  it("Re-run with… shows the step and opens its panel pre-filled", async () => {
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });
    ops.setHead.mockResolvedValue(view("n0002"));
    ops.getHistory.mockResolvedValue(state("n0002"));

    expect(await requestRerun(target, render, NODES[1], false)).toBe(true);

    expect(ops.setHead).toHaveBeenCalledWith(target.scope, "n0002");
    expect(published[0].result.fitsPath).toBe(`${DIR}/n0002.fits`);
    expect(historyRerunStore.peek("denoise")).toMatchObject({ node: "n0002", op: DENOISE, downstream: false });
    expect(historyRerunStore.armed()).toBeNull();
    expect(dockOpen).toHaveBeenCalledWith({ type: "open", tool: "processing" });
  });

  it("a downstream re-run keeps the head so the later steps stay downstream of the step", async () => {
    const { render, target } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0004") });

    expect(await requestRerun(target, render, NODES[1], true)).toBe(true);

    expect(ops.setHead).not.toHaveBeenCalled();
    expect(historyRerunStore.armed()).toMatchObject({ node: "n0002", downstream: true });
  });

  it("a downstream re-run on the current step keeps the head so its undone steps are re-applied", async () => {
    const { render, target } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0002") });

    expect(await requestRerun(target, render, NODES[1], true)).toBe(true);

    expect(ops.setHead).not.toHaveBeenCalled();
    expect(ops.rehydrate).not.toHaveBeenCalled();
    expect(historyRerunStore.armed()).toMatchObject({ node: "n0002", downstream: true });
  });

  it("a downstream re-run on another branch leaves the head where it is and rebuilds nothing", async () => {
    const branched = [...NODES.map((n) => ({ ...n, present: n.id === "n0001" })), node("n0005", "n0001", DENOISE), node("n0006", "n0005", ARCSINH)];
    const { render, target, published } = harness();
    historyStore.set({ ...EMPTY_HISTORY_SNAPSHOT, fileKey: KEY, scope: target.scope, state: state("n0006", branched) });

    expect(await requestRerun(target, render, NODES[1], true)).toBe(true);

    expect(ops.setHead).not.toHaveBeenCalled();
    expect(ops.rehydrate).not.toHaveBeenCalled();
    expect(published).toHaveLength(0);
    expect(historyRerunStore.armed()?.node).toBe("n0002");
  });

  it("a GHS step has no file panel to re-run in", async () => {
    const { render, target } = harness();
    const ghs = node("n0009", null, { op: "ghs", v: 1, d: 5, b: 2, sp: 0.05, lp: 0, hp: 1 });
    expect(await requestRerun(target, render, ghs, false)).toBe(false);
    expect(ops.setHead).not.toHaveBeenCalled();
    expect(historyRerunStore.get()).toBeNull();
  });

  it("deleting a branch shows the head that remains", async () => {
    const { render, target, published } = harness();
    ops.deleteBranch.mockResolvedValue({ ...view("n0002"), removed: ["n0003", "n0004"] });
    ops.getHistory.mockResolvedValue(state("n0002", NODES.slice(0, 2)));

    await removeBranch(target, render, "n0003");

    expect(ops.deleteBranch).toHaveBeenCalledWith(target.scope, "n0003");
    expect(published[0].result.fitsPath).toBe(`${DIR}/n0002.fits`);
  });
});
