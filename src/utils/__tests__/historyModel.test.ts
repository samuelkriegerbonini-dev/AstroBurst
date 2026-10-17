import { describe, expect, it } from "vitest";
import type { HistoryDoc, HistoryNode, OpSpec } from "../../shared/types/ops";
import type { ChainBase } from "../../shared/types/preview";
import { inputFor } from "../processingChain";
import {
  activePath,
  branchMarkers,
  headKernelRef,
  nodeAtPath,
  nodeFile,
  nodePreviewVersion,
  nodeRecord,
  opLabel,
  opSummary,
  opToStep,
  projectChain,
  redoTail,
} from "../historyModel";

const DIR = "C:\\Users\\me\\AppData\\Roaming\\astroburst\\history\\3f9c0a1e5b7d2c44";
const SOURCE = "C:/data/nrca1_i2d.fits#hdu=1";

const BACKGROUND: OpSpec = {
  op: "background",
  v: 1,
  model: "polynomial",
  config: { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" },
};
const DENOISE: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [3, 2.5, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const DENOISE_STRONG: OpSpec = { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [4, 3, 2, 1.5, 1], linearDenoise: true, layerBias: null };
const ARCSINH: OpSpec = { op: "arcsinh", v: 1, factor: 50 };
const TONE: OpSpec = { op: "tone", v: 1, levels: { black: 0.1, gamma: 1, white: 0.9 } };
const DECONV_KERNEL: OpSpec = {
  op: "rl_deconv",
  v: 1,
  iterations: 20,
  regularization: 0.001,
  deringing: true,
  deringingThreshold: 0.1,
  psf: { kind: "kernel", hash: "9f3a51c2e07b8d14", rows: 31, cols: 31 },
};
const DECONV_GAUSSIAN: OpSpec = { ...DECONV_KERNEL, psf: { kind: "gaussian", sigma: 2, size: 15 } } as OpSpec;
const PIXELMATH: OpSpec = { op: "pixelmath", v: 1, expression: "$T * 2", slots: [], truncate: true, rescale: false };

function node(id: string, parent: string | null, op: OpSpec, present = true): HistoryNode {
  return {
    id,
    parent,
    op,
    mask: null,
    output: { path: `${id}.fits`, png: `${id}.png`, extras: [], dims: [2048, 2048], planes: 1, domain: "linear" },
    at: "2026-10-16T12:00:00Z",
    elapsed_ms: 10,
    app: "0.6.9",
    threads: 8,
    present,
    bytes: 16_777_216,
    checkpoint: false,
  };
}

function doc(nodes: HistoryNode[], head: string | null, basePath: string | null = null): HistoryDoc {
  return {
    format: "astroburst-history",
    version: 1,
    image: { id: "3f9c0a1e5b7d2c44", ref: SOURCE, size: 1, mtime: null },
    image_key: SOURCE,
    base_path: basePath,
    head,
    policy: { k: 4 },
    app: "0.6.9",
    threads: 8,
    masks: {},
    nodes,
  };
}

const LINEAR = doc(
  [node("n0001", null, BACKGROUND), node("n0002", "n0001", DENOISE), node("n0003", "n0002", ARCSINH), node("n0004", "n0003", TONE)],
  "n0004",
);

const BRANCHED = doc(
  [
    node("n0001", null, BACKGROUND),
    node("n0002", "n0001", DENOISE),
    node("n0003", "n0002", ARCSINH),
    node("n0004", "n0003", TONE),
    node("n0005", "n0001", DENOISE_STRONG),
    node("n0006", "n0005", ARCSINH),
  ],
  "n0006",
);

describe("projectChain", () => {
  it("projectChain_maps_a_stage_monotonic_path_onto_ProcessingChain", () => {
    const chain = projectChain(LINEAR, "n0004", { dir: DIR, base: null });
    expect(Object.keys(chain.steps).sort()).toEqual(["background", "denoise", "stretch", "tone"]);
    expect(chain.steps.background?.fitsPath).toBe(`${DIR}\\n0001.fits`);
    expect(inputFor(chain, "tone", SOURCE)).toBe(`${DIR}\\n0003.fits`);
    expect(inputFor(chain, "background", SOURCE)).toBe(SOURCE);
    expect(chain.steps.tone?.dimensions).toEqual([2048, 2048]);
    expect(chain.base).toBeNull();
  });

  it("projects only the ancestry of the head, not later or sibling nodes", () => {
    const chain = projectChain(BRANCHED, "n0006", { dir: DIR, base: null });
    expect(chain.steps.denoise?.fitsPath).toBe(`${DIR}\\n0005.fits`);
    expect(chain.steps.stretch?.fitsPath).toBe(`${DIR}\\n0006.fits`);
    expect(chain.steps.tone).toBeUndefined();
    const undone = projectChain(LINEAR, "n0002", { dir: DIR, base: null });
    expect(Object.keys(undone.steps).sort()).toEqual(["background", "denoise"]);
  });

  it("projects the root as an empty chain", () => {
    const chain = projectChain(LINEAR, null, { dir: DIR, base: null, psfKernel: null });
    expect(chain.steps).toEqual({});
    expect(chain.psfKernel).toBeNull();
  });

  it("projectChain_keeps_chain_base_when_the_root_is_a_stack_result", () => {
    const base: ChainBase = {
      path: "C:/app/output/stack_mean.fits",
      label: "Stack (mean)",
      previewUrl: "asset://stack.png",
      dimensions: [2048, 2048],
      kind: "stacking",
      inputPath: SOURCE,
    };
    const rebased = doc([node("n0001", null, DENOISE), node("n0002", "n0001", ARCSINH)], "n0002", base.path);
    const chain = projectChain(rebased, "n0002", { dir: DIR, base });
    expect(chain.base).toBe(base);
    expect(inputFor(chain, "denoise", SOURCE)).toBe(base.path);
    expect(inputFor(chain, "stretch", SOURCE)).toBe(`${DIR}\\n0001.fits`);
  });

  it("projectChain_carries_the_psf_kernel_reference_of_the_head_path", () => {
    const withKernel = doc([node("n0001", null, BACKGROUND), node("n0002", "n0001", DECONV_KERNEL), node("n0003", "n0002", ARCSINH)], "n0003");
    expect(headKernelRef(withKernel, "n0003")).toEqual({ hash: "9f3a51c2e07b8d14", rows: 31, cols: 31 });
    expect(headKernelRef(withKernel, "n0001")).toBeNull();
    const rows = [[0, 1, 0], [1, 4, 1], [0, 1, 0]];
    const kernels = new Map([["9f3a51c2e07b8d14", rows]]);
    const chain = projectChain(withKernel, "n0003", { dir: DIR, base: null, kernels, psfKernel: [[1]] });
    expect(chain.psfKernel).toBe(rows);
    expect(chain.steps.deconv?.psfUsed).toBe(true);
    const before = projectChain(withKernel, "n0001", { dir: DIR, base: null, kernels, psfKernel: [[1]] });
    expect(before.psfKernel).toEqual([[1]]);
  });

  it("marks a Gaussian deconvolution as not using the provided PSF", () => {
    const gaussian = doc([node("n0001", null, DECONV_GAUSSIAN)], "n0001");
    expect(headKernelRef(gaussian, "n0001")).toBeNull();
    expect(projectChain(gaussian, "n0001", { dir: DIR, base: null }).steps.deconv?.psfUsed).toBe(false);
  });

  it("resolves node previews through the supplied lookup", () => {
    const chain = projectChain(LINEAR, "n0002", { dir: DIR, base: null, previewOf: (png) => `asset://${png}` });
    expect(chain.steps.denoise?.previewUrl).toBe(`asset://${DIR}\\n0002.png`);
    expect(projectChain(LINEAR, "n0002", { dir: DIR, base: null }).steps.denoise?.previewUrl).toBeNull();
  });
});

describe("history paths", () => {
  it("lists the head ancestry root first and the undone tail after it", () => {
    expect(activePath(LINEAR).map((n) => n.id)).toEqual(["n0001", "n0002", "n0003", "n0004"]);
    const undone = { ...LINEAR, head: "n0002" };
    expect(activePath(undone).map((n) => n.id)).toEqual(["n0001", "n0002"]);
    expect(redoTail(undone).map((n) => n.id)).toEqual(["n0003", "n0004"]);
    const atRoot = { ...BRANCHED, head: null };
    expect(activePath(atRoot)).toEqual([]);
    expect(redoTail(atRoot).map((n) => n.id)).toEqual(["n0001", "n0005", "n0006"]);
  });

  it("marks the fork where another branch leaves the shown path", () => {
    const markers = branchMarkers(BRANCHED);
    expect(markers).toHaveLength(1);
    expect(markers[0].at).toBe("n0001");
    expect(markers[0].branches.map((b) => b.map((n) => n.id))).toEqual([["n0002", "n0003", "n0004"]]);
    expect(branchMarkers(LINEAR)).toEqual([]);
  });

  it("puts the newest alternative branch first and survives a parent cycle", () => {
    const three = doc([...BRANCHED.nodes, node("n0007", "n0001", DENOISE)], "n0006");
    expect(branchMarkers(three)[0].branches.map((b) => b[0].id)).toEqual(["n0007", "n0002"]);
    const cyclic = doc([node("n0001", "n0002", BACKGROUND), node("n0002", "n0001", DENOISE)], "n0002");
    expect(activePath(cyclic).length).toBeLessThanOrEqual(2);
  });
});

describe("node mapping", () => {
  it("maps every op to its chain step and label", () => {
    expect(opToStep(BACKGROUND)).toBe("background");
    expect(opToStep(DENOISE)).toBe("denoise");
    expect(opToStep(DECONV_KERNEL)).toBe("deconv");
    expect(opToStep(ARCSINH)).toBe("stretch");
    expect(opToStep({ op: "ghs", v: 1, d: 5, b: 2, sp: 0.05, lp: 0, hp: 1 })).toBe("stretch");
    expect(opToStep(TONE)).toBe("tone");
    expect(opToStep(PIXELMATH)).toBe("pixelMath");
    expect(opLabel(DENOISE)).toBe("Wavelet Denoise");
    expect(opLabel(TONE)).toBe("Curves");
  });

  it("joins node files with the separator of the history folder", () => {
    expect(nodeFile(DIR, "n0001.fits")).toBe(`${DIR}\\n0001.fits`);
    expect(nodeFile("/home/me/.local/share/astroburst/history/abc", "n0001.fits")).toBe("/home/me/.local/share/astroburst/history/abc/n0001.fits");
    expect(nodeFile(`${DIR}\\`, "n0001.fits")).toBe(`${DIR}\\n0001.fits`);
  });

  it("finds the node behind a chain entry path in any spelling", () => {
    expect(nodeAtPath(LINEAR, DIR, `${DIR.replace(/\\/g, "/").toUpperCase()}/N0003.FITS`)?.id).toBe("n0003");
    expect(nodeAtPath(LINEAR, DIR, SOURCE)).toBeNull();
  });

  it("builds the record of a node with its parent as the input", () => {
    const root = { dir: DIR, rootPath: SOURCE, rootPreviewUrl: "asset://source.png", previewOf: (png: string) => `asset://${png}` };
    const first = nodeRecord(LINEAR, "n0001", root);
    expect(first).toMatchObject({ fitsPath: `${DIR}\\n0001.fits`, label: "Background Extraction", kind: "processing", inputPath: SOURCE });
    const third = nodeRecord(LINEAR, "n0003", root);
    expect(third?.inputPath).toBe(`${DIR}\\n0002.fits`);
    expect(third?.previewUrl).toBe(`asset://${DIR}\\n0003.png`);
    const math = nodeRecord(doc([node("n0001", null, BACKGROUND), node("n0002", "n0001", PIXELMATH)], "n0002"), "n0002", root);
    expect(math).toMatchObject({ kind: "pixelmath", inputPath: `${DIR}\\n0001.fits`, inputPreviewUrl: `asset://${DIR}\\n0001.png` });
    const rootMath = nodeRecord(doc([node("n0001", null, PIXELMATH)], "n0001"), "n0001", root);
    expect(rootMath?.inputPreviewUrl).toBe("asset://source.png");
    expect(nodeRecord(LINEAR, "n0099", root)).toBeNull();
  });
});

describe("opSummary", () => {
  it("names the settings a user would change", () => {
    expect(opSummary(BACKGROUND)).toBe("grid 8 · degree 3 · subtract");
    expect(opSummary({ op: "background", v: 1, model: "deband_auto", config: { sigmaClip: 3, iterations: 3 } })).toBe("auto axis · σ 3");
    expect(opSummary(DENOISE_STRONG)).toBe("thresholds 4 / 3 / 2 / 1.5 / 1");
    expect(opSummary({ ...DENOISE, linearDenoise: false } as OpSpec)).toBe("thresholds 3 / 2.5 / 2 / 1.5 / 1 · non-linear");
    expect(opSummary(DECONV_KERNEL)).toBe("20 iterations · provided PSF 31×31");
    expect(opSummary(DECONV_GAUSSIAN)).toBe("20 iterations · Gaussian PSF σ 2");
    expect(opSummary(ARCSINH)).toBe("factor 50");
    expect(opSummary(TONE)).toBe("levels 0.1 / 1 / 0.9");
    expect(opSummary({ op: "tone", v: 1, curve: { points: [[0, 0], [0.5, 0.7], [1, 1]] } })).toBe("curve 3 points");
    expect(opSummary(PIXELMATH)).toBe("$T * 2");
    expect(opSummary({ ...PIXELMATH, expression: "x".repeat(60) } as OpSpec)).toBe(`${"x".repeat(39)}…`);
  });
});

describe("nodePreviewVersion", () => {
  it("turns the recorded time of a step into a cache-busting version", () => {
    const at = (value: string) => ({ ...BRANCHED.nodes[0], at: value });
    expect(nodePreviewVersion(at("2026-10-16T12:00:00Z"))).toBe(Date.parse("2026-10-16T12:00:00Z"));
    expect(nodePreviewVersion(at("2025-10-09T08:53:20.120000700Z"))).toBe(Date.parse("2025-10-09T08:53:20.120Z"));
    expect(nodePreviewVersion(at("2025-10-09T08:53:20.5Z"))).toBe(Date.parse("2025-10-09T08:53:20.500Z"));
    expect(nodePreviewVersion(at("yesterday"))).toBeNull();
  });
});
