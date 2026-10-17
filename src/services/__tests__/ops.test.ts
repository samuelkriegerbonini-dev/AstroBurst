import { beforeEach, describe, expect, it, vi } from "vitest";

const { withPreviewMock, typedInvokeMock, getPreviewUrlMock } = vi.hoisted(() => ({
  withPreviewMock: vi.fn(),
  typedInvokeMock: vi.fn(),
  getPreviewUrlMock: vi.fn(async (path: string) => `asset://${path}`),
}));

vi.mock("../../infrastructure/tauri", () => ({
  withPreview: withPreviewMock,
  typedInvoke: typedInvokeMock,
  getPreviewUrl: getPreviewUrlMock,
}));

import { deleteBranch, estimateNextNode, getHistory, getKernel, redo, rehydrate, rerun, runOp, setHead, undo } from "../ops";
import type { HeadView, OpSpec } from "../../shared/types/ops";

const SCOPE = { imageKey: "C:/data/nrca1_i2d.fits#hdu=1", basePath: null };
const NODE_DIR = "C:/app/history/3f9c0a1e5b7d2c44";
const TONE: OpSpec = { op: "tone", v: 1, levels: { black: 0.1, gamma: 1, white: 0.9 } };

function headView(head: string | null): HeadView {
  return {
    image_id: "3f9c0a1e5b7d2c44",
    history_dir: NODE_DIR,
    head,
    node: head
      ? {
          id: head,
          parent: null,
          op: TONE,
          fits_path: `${NODE_DIR}/${head}.fits`,
          png_path: `${NODE_DIR}/${head}.png`,
          extras: [],
          dimensions: [2058, 2058],
          planes: 1,
          domain: "display_referred",
          present: true,
        }
      : null,
    root_path: SCOPE.imageKey,
    can_undo: head !== null,
    can_redo: false,
    node_count: 1,
  };
}

describe("runOp", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue({ node: "n0001", fits_path: `${NODE_DIR}/n0001.fits`, png_path: `${NODE_DIR}/n0001.png` });
  });

  it("records through run_op_cmd with the image key, the chain base and the input path", async () => {
    const res = await runOp(SCOPE, "C:/data/nrca1_i2d.fits#hdu=1", TONE);
    expect(res.node).toBe("n0001");
    expect(withPreviewMock).toHaveBeenCalledTimes(1);
    const [cmd, , args, previews] = withPreviewMock.mock.calls[0];
    expect(cmd).toBe("run_op_cmd");
    expect(args).toEqual({ imageKey: SCOPE.imageKey, basePath: null, inputPath: SCOPE.imageKey, op: TONE, psfKernel: null });
    expect(previews).toEqual([["png_path", "previewUrl"], ["model_png", "modelPreviewUrl"]]);
  });

  it("never routes a recorded node to a caller-chosen output folder", async () => {
    const stray = { outputDir: "C:/elsewhere" } as unknown as Parameters<typeof runOp>[3];
    await runOp(SCOPE, SCOPE.imageKey, TONE, stray);
    expect(withPreviewMock.mock.calls[0][1]).toBeUndefined();
  });

  it("sends a supplied PSF kernel beside a kernel op", async () => {
    const op: OpSpec = {
      op: "rl_deconv",
      v: 1,
      iterations: 10,
      regularization: 0.001,
      deringing: true,
      deringingThreshold: 0.1,
      psf: { kind: "kernel", hash: "", rows: 3, cols: 3 },
    };
    const kernel = [[0, 1, 0], [1, 4, 1], [0, 1, 0]];
    await runOp({ imageKey: "C:/a.fits", basePath: "C:/app/output/stack.fits" }, `${NODE_DIR}/n0001.fits`, op, { psfKernel: kernel });
    expect(withPreviewMock.mock.calls[0][2]).toEqual({
      imageKey: "C:/a.fits",
      basePath: "C:/app/output/stack.fits",
      inputPath: `${NODE_DIR}/n0001.fits`,
      op,
      psfKernel: kernel,
    });
  });
});

describe("head moves", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    getPreviewUrlMock.mockClear();
  });

  it("undo, redo and set head move the head and resolve the node preview", async () => {
    typedInvokeMock.mockResolvedValue(headView("n0002"));
    const undone = await undo(SCOPE);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_undo_cmd", { imageKey: SCOPE.imageKey, basePath: null });
    expect(undone.node?.previewUrl).toBe(`asset://${NODE_DIR}/n0002.png`);

    await redo(SCOPE);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_redo_cmd", { imageKey: SCOPE.imageKey, basePath: null });

    typedInvokeMock.mockResolvedValue(headView(null));
    const original = await setHead(SCOPE, null);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_set_head_cmd", { imageKey: SCOPE.imageKey, basePath: null, node: null });
    expect(original.node).toBeNull();
    expect(getPreviewUrlMock).toHaveBeenCalledTimes(2);
  });

  it("rerun, rehydrate, history, estimate and kernel call their commands", async () => {
    typedInvokeMock.mockResolvedValue({ ...headView("n0007"), created: ["n0005", "n0006", "n0007"] });
    const rerunResult = await rerun(SCOPE, "n0002", { op: TONE, replayDownstream: true });
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_rerun_cmd", {
      imageKey: SCOPE.imageKey,
      basePath: null,
      node: "n0002",
      op: TONE,
      replayDownstream: true,
      psfKernel: null,
    });
    expect(rerunResult.created).toEqual(["n0005", "n0006", "n0007"]);
    expect(rerunResult.node?.previewUrl).toBe(`asset://${NODE_DIR}/n0007.png`);

    typedInvokeMock.mockResolvedValue({ image_id: "x", head: "n0004", node: headView("n0004").node, rebuilt: ["n0003", "n0004"] });
    const rebuilt = await rehydrate(SCOPE, "n0004");
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_rehydrate_cmd", { imageKey: SCOPE.imageKey, basePath: null, node: "n0004" });
    expect(rebuilt.node?.previewUrl).toBe(`asset://${NODE_DIR}/n0004.png`);

    typedInvokeMock.mockResolvedValue({ doc: null });
    await getHistory(SCOPE);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_get_cmd", { imageKey: SCOPE.imageKey, basePath: null });

    typedInvokeMock.mockResolvedValue({ next_node_bytes: 16941456 });
    expect((await estimateNextNode("C:/data/a.fits")).next_node_bytes).toBe(16941456);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_estimate_cmd", { inputPath: "C:/data/a.fits" });

    typedInvokeMock.mockResolvedValue({ hash: "9f3a51c2e07b8d14", kernel: [[1]] });
    expect(await getKernel(SCOPE, { hash: "9f3a51c2e07b8d14", rows: 1, cols: 1 })).toEqual([[1]]);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_kernel_cmd", {
      imageKey: SCOPE.imageKey,
      basePath: null,
      hash: "9f3a51c2e07b8d14",
      rows: 1,
      cols: 1,
    });
  });

  it("reports the first re-run step with its preview beside the new head", async () => {
    const run = { node: "n0005", parent: "n0001", fits_path: `${NODE_DIR}/n0005.fits`, png_path: `${NODE_DIR}/n0005.png`, noise_estimate: 0.2 };
    typedInvokeMock.mockResolvedValue({ ...headView("n0007"), created: ["n0005", "n0006", "n0007"], run });
    const result = await rerun(SCOPE, "n0002", { op: TONE, replayDownstream: true });
    expect(result.run.node).toBe("n0005");
    expect(result.run.previewUrl).toBe(`asset://${NODE_DIR}/n0005.png`);
    expect(result.run.noise_estimate).toBe(0.2);
  });

  it("deletes a branch through history_delete_branch_cmd and resolves the new head preview", async () => {
    typedInvokeMock.mockResolvedValue({ ...headView("n0001"), removed: ["n0002", "n0003"] });
    const result = await deleteBranch(SCOPE, "n0002");
    expect(typedInvokeMock).toHaveBeenLastCalledWith("history_delete_branch_cmd", { imageKey: SCOPE.imageKey, basePath: null, node: "n0002" });
    expect(result.removed).toEqual(["n0002", "n0003"]);
    expect(result.node?.previewUrl).toBe(`asset://${NODE_DIR}/n0001.png`);
  });
});
