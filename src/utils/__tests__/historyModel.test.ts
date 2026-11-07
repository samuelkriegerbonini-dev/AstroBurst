import { describe, expect, it } from "vitest";
import type { HistoryDoc, HistoryNode, OpSpec } from "../../shared/types/ops";
import type { ChainBase, ChainEntry } from "../../shared/types/preview";
import { maskedStretchOp, waveletOp } from "../../services/processing";
import { inputFor } from "../processingChain";
import {
  activePath,
  backgroundModelLabel,
  branchMarkers,
  chainStepLabel,
  compareLabelsFor,
  compositeCrumbLabelFor,
  crumbLabelFor,
  headKernelRef,
  nodeAtPath,
  nodeFile,
  nodeMaskView,
  nodePreviewVersion,
  nodeRecord,
  opLabel,
  opNameLabel,
  opSummary,
  opToStep,
  projectChain,
  redoTail,
  stepLabel,
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
const GHS: OpSpec = { op: "ghs", v: 1, d: 3, b: 0, sp: 0.05, lp: 0, hp: 1 };
const LHE: OpSpec = { op: "lhe", v: 1, kernelRadius: 64, contrastLimit: 2, amount: 1, histBits: 8, circular: true };
const HDRMT: OpSpec = { op: "hdrmt", v: 1, layers: 6, iterations: 1, overdrive: 0, inverted: false, toLightness: true, deringing: false, deringingAmount: 0.5 };
const DBE: OpSpec = {
  op: "background",
  v: 1,
  model: "spline",
  config: { sampleRadius: 5, tolerance: 1, smoothing: 0.5, autoGrid: 8, manualSamples: [], rejectStars: true, mode: "subtract", normalize: true, maxSamples: 400 },
};
const DEBAND: OpSpec = { op: "background", v: 1, model: "deband_rows", config: { sigmaClip: 3, iterations: 3 } };

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

  it("keeps the op of each step on its entry, so crumbs and compare bases name the tool without a node", () => {
    const toned = doc([node("n0001", null, DBE), node("n0002", "n0001", GHS), node("n0003", "n0002", HDRMT)], "n0003");
    const chain = projectChain(toned, "n0003", { dir: DIR, base: null });
    expect(chain.steps.background?.op).toEqual(DBE);
    expect(chain.steps.stretch?.op).toEqual(GHS);
    expect(crumbLabelFor("background", chain.steps.background)).toBe("DBE");
    expect(crumbLabelFor("stretch", chain.steps.stretch)).toBe("GHS");
    expect(crumbLabelFor("localContrast", chain.steps.localContrast)).toBe("HDRMT");
    expect(compareLabelsFor(chain.steps)).toMatchObject({ background: "DBE", stretch: "GHS", localContrast: "HDRMT" });
    const recordedBefore: ChainEntry = { fitsPath: `${DIR}\\n0002.fits`, previewUrl: null, dimensions: null };
    expect(crumbLabelFor("stretch", recordedBefore)).toBe("Stretch");
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

  it("names a tool before it runs with the label History will record", () => {
    expect(backgroundModelLabel("polynomial")).toBe(opLabel(BACKGROUND));
    expect(backgroundModelLabel("spline")).toBe("DBE");
    expect(backgroundModelLabel("deband_rows")).toBe(opLabel({ op: "background", v: 1, model: "deband_rows", config: { sigmaClip: 3, iterations: 3 } }));
    expect(opNameLabel("arcsinh")).toBe(opLabel(ARCSINH));
    expect(opNameLabel("ghs")).toBe("GHS Stretch");
    expect(opNameLabel("lhe")).toBe("LHE");
    expect(opNameLabel("hdrmt")).toBe("HDRMT");
    expect(opNameLabel("background")).toBe("Background Extraction");
  });

  it("names a chain step that records no op by the label History gives its default tool", () => {
    expect(chainStepLabel("background")).toBe(opNameLabel("background"));
    expect(chainStepLabel("denoise")).toBe(opNameLabel("wavelet_denoise"));
    expect(chainStepLabel("deconv")).toBe(opNameLabel("rl_deconv"));
    expect(chainStepLabel("stretch")).toBe(opNameLabel("arcsinh"));
    expect(chainStepLabel("maskedStretch")).toBe(opNameLabel("masked_stretch"));
    expect(chainStepLabel("localContrast")).toBe(`${opNameLabel("lhe")} / ${opNameLabel("hdrmt")}`);
    expect(chainStepLabel("localContrast")).toBe("LHE / HDRMT");
    expect(chainStepLabel("tone")).toBe(opNameLabel("tone"));
    expect(chainStepLabel("pixelMath")).toBe(opNameLabel("pixelmath"));
  });

  it("names a crumb by the tool that made its step, and by the step when no node says which", () => {
    expect(crumbLabelFor("localContrast", node("n0004", "n0003", HDRMT))).toBe("HDRMT");
    expect(crumbLabelFor("localContrast", node("n0004", "n0003", LHE))).toBe("LHE");
    expect(crumbLabelFor("localContrast", null)).toBe("LHE/HDRMT");
    expect(crumbLabelFor("stretch", node("n0003", "n0002", GHS))).toBe("GHS");
    expect(crumbLabelFor("stretch", node("n0003", "n0002", ARCSINH))).toBe("Stretch");
    expect(crumbLabelFor("background", node("n0001", null, BACKGROUND))).toBe("BG");
    expect(crumbLabelFor("background", node("n0001", null, DBE))).toBe("DBE");
    expect(crumbLabelFor("background", node("n0001", null, DEBAND))).toBe("De-band");
    expect(crumbLabelFor("denoise", node("n0002", "n0001", DENOISE))).toBe("Denoise");
    expect(crumbLabelFor("deconv", node("n0002", "n0001", DECONV_KERNEL))).toBe("Deconv");
    expect(crumbLabelFor("tone", node("n0005", "n0004", TONE))).toBe("Curves");
    expect(crumbLabelFor("pixelMath", node("n0006", "n0005", PIXELMATH))).toBe("PixelMath");
  });

  it("names a compare base by the tool that made its step", () => {
    expect(compareLabelsFor({ background: node("n0001", null, DBE), stretch: node("n0003", "n0002", GHS), localContrast: node("n0004", "n0003", HDRMT) })).toEqual({
      background: "DBE",
      denoise: "Denoise",
      deconv: "Deconvolution",
      stretch: "GHS",
      maskedStretch: "Masked stretch",
      localContrast: "HDRMT",
      tone: "Curves",
      pixelMath: "PixelMath",
    });
    expect(compareLabelsFor({ background: node("n0001", null, BACKGROUND), stretch: node("n0002", "n0001", ARCSINH), localContrast: node("n0003", "n0002", LHE) })).toMatchObject({
      background: "Background",
      stretch: "Stretch",
      localContrast: "LHE",
    });
    expect(compareLabelsFor({})).toMatchObject({ background: "Background", stretch: "Stretch", localContrast: "LHE / HDRMT" });
  });

  it("spells De-band in History as the Background panel does", () => {
    expect(opLabel(DEBAND)).toBe("De-band");
    expect(backgroundModelLabel("deband_auto")).toBe("De-band");
    expect(compareLabelsFor({ background: node("n0001", null, DEBAND) }).background).toBe("De-band");
  });

  it("names a composite crumb by the tool its entry label records, and by the step otherwise", () => {
    expect(compositeCrumbLabelFor("background", "DBE")).toBe("DBE");
    expect(compositeCrumbLabelFor("background", "Background Removed")).toBe("BG");
    expect(compositeCrumbLabelFor("localContrast", "HDRMT")).toBe("HDRMT");
    expect(compositeCrumbLabelFor("localContrast", "LHE")).toBe("LHE");
    expect(compositeCrumbLabelFor("stretch", "Stretch")).toBe("Stretch");
    expect(compositeCrumbLabelFor("denoise", "Denoised")).toBe("Denoise");
    expect(compositeCrumbLabelFor("deconv", "Deconvolved")).toBe("Deconv");
    expect(compositeCrumbLabelFor("maskedStretch", "Masked stretch")).toBe("Masked");
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

  it("keeps the plane count and value domain of the node, so a Curves result on an RGB file stays a 3-plane display-referred product", () => {
    const root = { dir: DIR, rootPath: "C:/data/runtime-rgb.fits", rootPreviewUrl: "asset://rgb.png" };
    const rgbNode = node("n0001", null, { op: "tone", v: 1, levels: { black: 0, gamma: 1.5, white: 1 } });
    const rgbDoc = doc([{ ...rgbNode, output: { ...rgbNode.output, dims: [128, 128], planes: 3, domain: "display_referred" } }], "n0001");
    expect(nodeRecord(rgbDoc, "n0001", root)).toMatchObject({ fitsPath: `${DIR}\\n0001.fits`, label: "Curves", planes: 3, domain: "display_referred" });
    expect(nodeRecord(LINEAR, "n0001", root)).toMatchObject({ planes: 1, domain: "linear" });
  });
});

describe("masked steps", () => {
  const LUMA_RECIPE = { kind: "luminance", auto_stf: true, feather_px: 0, invert: false };
  const RANGE_RECIPE = { kind: "range", lo: 0.2, hi: 0.8, feather_px: 0, invert: false };
  const STARS_INLINE = { kind: "stars", sigma: 4, feather_px: 0, invert: false };
  const MASKS = { masks: { m01: LUMA_RECIPE, m02: RANGE_RECIPE }, deleted_masks: ["m02"] };

  function masked(mask: HistoryNode["mask"]): HistoryNode {
    return { ...node("n0003", "n0002", LHE), mask };
  }

  it("names the mask of a masked step in its History label", () => {
    expect(stepLabel(MASKS, masked(null))).toBe("LHE");
    expect(stepLabel(MASKS, masked("m01"))).toBe("LHE · mask m01");
    expect(stepLabel(MASKS, masked("m02"))).toBe("LHE · mask m02 (deleted)");
    expect(stepLabel(MASKS, masked(STARS_INLINE))).toBe("LHE · stars mask");
    expect(stepLabel(null, { ...node("n0002", "n0001", DENOISE), mask: "m01" })).toBe("Wavelet Denoise · mask m01");
  });

  it("reads a step's mask as the backend node view does", () => {
    expect(nodeMaskView(MASKS, null)).toBeNull();
    expect(nodeMaskView(MASKS, "m01")).toEqual({ id: "m01", recipe: LUMA_RECIPE, deleted: false });
    expect(nodeMaskView(MASKS, "m02")).toEqual({ id: "m02", recipe: RANGE_RECIPE, deleted: true });
    expect(nodeMaskView({ masks: {} }, "m09")).toEqual({ id: "m09", recipe: null, deleted: false });
    expect(nodeMaskView(null, STARS_INLINE)).toEqual({ id: null, recipe: STARS_INLINE, deleted: false });
  });
});

describe("opSummary", () => {
  it("names the settings a user would change", () => {
    expect(opSummary(BACKGROUND)).toBe("grid 8 · degree 3 · subtract");
    expect(opSummary({ op: "background", v: 1, model: "deband_auto", config: { sigmaClip: 3, iterations: 3 } })).toBe("auto axis · σ 3");
    expect(opSummary(DENOISE_STRONG)).toBe("thresholds 4 / 3 / 2 / 1.5 / 1");
    expect(opSummary({ ...DENOISE, linearDenoise: false } as OpSpec)).toBe("thresholds 3 / 2.5 / 2 / 1.5 / 1 · hard threshold");
    expect(opSummary(DECONV_KERNEL)).toBe("20 iterations · provided PSF 31×31");
    expect(opSummary(DECONV_GAUSSIAN)).toBe("20 iterations · Gaussian PSF σ 2");
    expect(opSummary(ARCSINH)).toBe("factor 50");
    expect(opSummary(TONE)).toBe("levels 0.1 / 1 / 0.9");
    expect(opSummary({ op: "tone", v: 1, curve: { points: [[0, 0], [0.5, 0.7], [1, 1]] } })).toBe("curve 3 points");
    expect(opSummary(PIXELMATH)).toBe("$T * 2");
    expect(opSummary({ ...PIXELMATH, expression: "x".repeat(60) } as OpSpec)).toBe(`${"x".repeat(39)}…`);
  });

  it("calls a hard-threshold Denoise run a hard threshold, not non-linear", () => {
    const hard = opSummary(waveletOp({ linear: false }));
    expect(hard).toContain("hard threshold");
    expect(hard).not.toContain("non-linear");
    expect(opSummary(waveletOp({ linear: true }))).toBe("thresholds 3 / 2.5 / 2 / 1.5 / 1");
  });

  it("gives the Masked target and the LHE amount in percent, as their panels do", () => {
    expect(opSummary(maskedStretchOp({ targetBackground: 0.25 }))).toBe("10 iterations · target 25 %");
    expect(opSummary(maskedStretchOp({ iterations: 12, targetBackground: 0.07 }))).toBe("12 iterations · target 7 %");
    expect(opSummary(LHE)).toBe("radius 64 px · limit 2 · amount 100 %");
    expect(opSummary({ ...LHE, amount: 0.7 } as OpSpec)).toBe("radius 64 px · limit 2 · amount 70 %");
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
