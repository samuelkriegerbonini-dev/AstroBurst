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
  getOutputDir: async () => "C:/app/output",
}));

import {
  arcsinhOptionsOf,
  backgroundOptionsOf,
  deconvolveOptionsOf,
  ghsOptionsOf,
  maskedStretchOptionsOf,
  recordArcsinhStretch,
  recordDeconvolveRL,
  recordExtractBackground,
  recordGhsStretch,
  recordMaskedStretch,
  recordWaveletDenoise,
  waveletOptionsOf,
} from "../processing";
import { dbeConfigOf, recordBackgroundDbe } from "../dbe";
import { hdrmtConfigOf, lheConfigOf, recordHdrmt, recordLhe } from "../localContrast";
import { pixelMathOptionsOf, recordPixelMath } from "../pixelmath";
import { recordToneFile, toneOptionsOf } from "../toneFile";
import { recordOp } from "../ops";
import { historyRerunStore } from "../../utils/historyRerunStore";
import type { OpSpec } from "../../shared/types/ops";
import type { DbeConfig } from "../../shared/types/dbe";
import type { HdrConfig, LheConfig } from "../../shared/types/localContrast";

const SCOPE = { imageKey: "C:/data/nrca1_i2d.fits#hdu=1", basePath: null };
const REBASED = { imageKey: "C:/data/nrca1_i2d.fits#hdu=1", basePath: "C:/app/output/stack_mean.fits" };
const DIR = "C:/app/history/3f9c0a1e5b7d2c44";
const INPUT = `${DIR}/n0001.fits`;

function runResponse(node: string, extra: Record<string, unknown> = {}) {
  return {
    node,
    parent: "n0001",
    image_id: "3f9c0a1e5b7d2c44",
    history_dir: DIR,
    fits_path: `${DIR}/${node}.fits`,
    png_path: `${DIR}/${node}.png`,
    model_png: null,
    dimensions: [2058, 2058],
    is_rgb: false,
    domain: "linear",
    elapsed_ms: 812,
    previewUrl: `asset://${DIR}/${node}.png`,
    ...extra,
  };
}

function lastRun(): { cmd: string; dir: unknown; args: Record<string, unknown> } {
  const call = withPreviewMock.mock.calls.at(-1);
  if (!call) throw new Error("withPreview was not called");
  return { cmd: call[0], dir: call[1], args: call[2] };
}

function expectRecorded(input: string, op: OpSpec, scope = SCOPE, psfKernel: number[][] | null = null): void {
  const { cmd, dir, args } = lastRun();
  expect(cmd).toBe("run_op_cmd");
  expect(dir).toBeUndefined();
  expect(args).toEqual({ imageKey: scope.imageKey, basePath: scope.basePath, inputPath: input, op, psfKernel });
}

beforeEach(() => {
  withPreviewMock.mockReset();
  typedInvokeMock.mockReset();
  historyRerunStore.clear();
});

describe("services_call_run_op_with_image_key_and_input_path", () => {
  it("recordDeconvolveRL sends rl_deconv with the chosen PSF", async () => {
    withPreviewMock.mockResolvedValue(runResponse("n0002", { iterations_run: 20, convergence: 0.01, psf_source: "gaussian" }));
    const res = await recordDeconvolveRL(SCOPE, INPUT, { iterations: 20, psfSigma: 2, psfSize: 15, regularization: 0.001, deringing: true, deringThreshold: 0.1 });
    expectRecorded(INPUT, {
      op: "rl_deconv",
      v: 1,
      iterations: 20,
      regularization: 0.001,
      deringing: true,
      deringingThreshold: 0.1,
      psf: { kind: "gaussian", sigma: 2, size: 15 },
    });
    expect(res).toMatchObject({ node: "n0002", fits_path: `${DIR}/n0002.fits`, iterations_run: 20, psf_source: "gaussian" });

    await recordDeconvolveRL(SCOPE, INPUT, { useEmpiricalPsf: true });
    expect((lastRun().args.op as { psf: unknown }).psf).toEqual({ kind: "empirical", numStars: 30, cutoutRadius: 15 });

    const kernel = [[0, 1, 0], [1, 4, 1], [0, 1, 0]];
    await recordDeconvolveRL(REBASED, INPUT, { psfSigma: 3, psfKernel: kernel });
    expect((lastRun().args.op as { psf: unknown }).psf).toEqual({ kind: "gaussian", sigma: 3, size: 15 });
    await recordDeconvolveRL(REBASED, INPUT, { useEmpiricalPsf: true, psfKernel: kernel });
    expect(lastRun().args).toMatchObject({ basePath: REBASED.basePath, psfKernel: kernel, op: { psf: { kind: "kernel", hash: "", rows: 3, cols: 3 } } });
  });

  it("recordExtractBackground sends the polynomial model or a deband model and maps the corrected output", async () => {
    withPreviewMock.mockResolvedValue(runResponse("n0001", { model_png: `${DIR}/n0001_model.png`, modelPreviewUrl: `asset://${DIR}/n0001_model.png`, sample_count: 64, rms_residual: 0.5, axis: null }));
    const res = await recordExtractBackground(SCOPE, SCOPE.imageKey, { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" });
    expectRecorded(SCOPE.imageKey, {
      op: "background",
      v: 1,
      model: "polynomial",
      config: { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" },
    });
    expect(res).toMatchObject({
      corrected_fits: `${DIR}/n0001.fits`,
      corrected_png: `${DIR}/n0001.png`,
      model_png: `${DIR}/n0001_model.png`,
      previewUrl: `asset://${DIR}/n0001.png`,
      modelUrl: `asset://${DIR}/n0001_model.png`,
      sample_count: 64,
    });
    await recordExtractBackground(SCOPE, SCOPE.imageKey, { sigmaClip: 3, iterations: 2, mode: "deband_auto" });
    expect(lastRun().args.op).toEqual({ op: "background", v: 1, model: "deband_auto", config: { sigmaClip: 3, iterations: 2 } });
  });

  it("recordWaveletDenoise sends wavelet_denoise", async () => {
    withPreviewMock.mockResolvedValue(runResponse("n0002", { scales_processed: 5, noise_estimate: 0.2 }));
    const res = await recordWaveletDenoise(SCOPE, INPUT, { numScales: 5, thresholds: [4, 3, 2, 1.5, 1], linear: false, layerBias: [0, 0, 0, 0, 0] });
    expectRecorded(INPUT, { op: "wavelet_denoise", v: 1, numScales: 5, thresholds: [4, 3, 2, 1.5, 1], linearDenoise: false, layerBias: [0, 0, 0, 0, 0] });
    expect(res.noise_estimate).toBe(0.2);
  });

  it("recordArcsinhStretch and recordGhsStretch send their stretch", async () => {
    withPreviewMock.mockResolvedValue(runResponse("n0003", { stretch_factor: 50 }));
    await recordArcsinhStretch(SCOPE, INPUT, 50);
    expectRecorded(INPUT, { op: "arcsinh", v: 1, factor: 50 });
    await recordGhsStretch(SCOPE, INPUT, { stretchFactor: 5, localIntensity: 2 });
    expectRecorded(INPUT, { op: "ghs", v: 1, d: 5, b: 2, sp: 0.05, lp: 0, hp: 1 });
  });

  it("recordMaskedStretch sends masked_stretch with the engine defaults for the fields the panel does not set", async () => {
    withPreviewMock.mockResolvedValue(runResponse("n0003", { iterations_run: 7, converged: true }));
    const res = await recordMaskedStretch(SCOPE, INPUT, { iterations: 12, targetBackground: 0.2 });
    expectRecorded(INPUT, {
      op: "masked_stretch",
      v: 1,
      iterations: 12,
      targetBackground: 0.2,
      maskGrowth: 2.5,
      maskSoftness: 4,
      detectionSigma: 8,
      maxEccentricity: 0.85,
      luminanceProtect: true,
      luminanceCeiling: 0.85,
      protectionAmount: 0.85,
      convergenceThreshold: 0.00001,
    });
    expect(res.converged).toBe(true);
  });

  it("recordBackgroundDbe sends the spline model with the DBE config", async () => {
    const config: DbeConfig = { sampleRadius: 5, tolerance: 1, smoothing: 0.25, autoGrid: 10, manualSamples: [[12, 40.5]], rejectStars: true, mode: "divide", normalize: true, maxSamples: 400 };
    withPreviewMock.mockResolvedValue(runResponse("n0001", { model_png: `${DIR}/n0001_model.png`, modelPreviewUrl: "asset://model", sample_count: 81, rejected_count: 3, rms_residual: 0.1, samples: [] }));
    const res = await recordBackgroundDbe(SCOPE, SCOPE.imageKey, config);
    expectRecorded(SCOPE.imageKey, { op: "background", v: 1, model: "spline", config });
    expect(res).toMatchObject({ corrected_fits: `${DIR}/n0001.fits`, modelUrl: "asset://model", rejected_count: 3 });
  });

  it("recordLhe and recordHdrmt send their configs", async () => {
    const lhe: LheConfig = { kernelRadius: 64, contrastLimit: 2, amount: 1, histBits: 8, circular: true };
    const hdr: HdrConfig = { layers: 6, iterations: 1, overdrive: 0, inverted: false, toLightness: true, deringing: false, deringingAmount: 0.5 };
    withPreviewMock.mockResolvedValue(runResponse("n0004"));
    await recordLhe(SCOPE, INPUT, lhe);
    expectRecorded(INPUT, { op: "lhe", v: 1, ...lhe });
    await recordHdrmt(SCOPE, INPUT, hdr);
    expectRecorded(INPUT, { op: "hdrmt", v: 1, ...hdr });
  });

  it("recordPixelMath sends the referenced slots as refs", async () => {
    withPreviewMock.mockResolvedValue(runResponse("n0005", { stats: null, non_finite_count: 0, warnings: [] }));
    await recordPixelMath(SCOPE, INPUT, "0.5 * $T + 0.5 * Ha", { slots: [{ name: " Ha ", path: "C:/data/ha.fits" }], truncate: true, rescale: false });
    expectRecorded(INPUT, { op: "pixelmath", v: 1, expression: "0.5 * $T + 0.5 * Ha", slots: [{ name: "Ha", ref: "C:/data/ha.fits" }], truncate: true, rescale: false });
  });

  it("recordToneFile sends only the non-identity parts", async () => {
    withPreviewMock.mockResolvedValue(runResponse("n0004", { levels_applied: true, curves_applied: false, input_normalized: null }));
    const res = await recordToneFile(SCOPE, INPUT, { levels: { black: 0.1, gamma: 1, white: 0.9 }, curve: [{ x: 0, y: 0 }, { x: 1, y: 1 }] });
    expectRecorded(INPUT, { op: "tone", v: 1, levels: { black: 0.1, gamma: 1, white: 0.9 } });
    expect(res.levels_applied).toBe(true);
    await recordToneFile(SCOPE, INPUT, { curve: [{ x: 0, y: 0 }, { x: 0.5, y: 0.7 }, { x: 1, y: 1 }] });
    expectRecorded(INPUT, { op: "tone", v: 1, curve: { points: [[0, 0], [0.5, 0.7], [1, 1]] } });
  });
});

describe("recordOp", () => {
  it("routes an armed downstream re-run of the same step through history_rerun_cmd", async () => {
    const op: OpSpec = { op: "arcsinh", v: 1, factor: 20 };
    const pending = historyRerunStore.request({ section: "stretch", op: { op: "arcsinh", v: 1, factor: 50 }, node: "n0003", downstream: true, scope: SCOPE });
    historyRerunStore.consume(pending.seq);
    typedInvokeMock.mockResolvedValue({
      image_id: "3f9c0a1e5b7d2c44",
      history_dir: DIR,
      head: "n0007",
      node: null,
      root_path: SCOPE.imageKey,
      can_undo: true,
      can_redo: false,
      node_count: 7,
      created: ["n0006", "n0007"],
      run: runResponse("n0006", { stretch_factor: 20, previewUrl: undefined }),
    });
    const res = await recordOp(SCOPE, INPUT, op);
    expect(withPreviewMock).not.toHaveBeenCalled();
    expect(typedInvokeMock).toHaveBeenCalledWith("history_rerun_cmd", { ...SCOPE, node: "n0003", op, replayDownstream: true, psfKernel: null });
    expect(res.node).toBe("n0006");
    expect(res.previewUrl).toBe(`asset://${DIR}/n0006.png`);
    expect(historyRerunStore.armed()).toBeNull();

    withPreviewMock.mockResolvedValue(runResponse("n0008"));
    await recordOp(SCOPE, INPUT, op);
    expect(withPreviewMock).toHaveBeenCalledTimes(1);
  });
});

describe("panel settings decoded from a recorded op", () => {
  it("round-trips the settings of every panel", () => {
    const deconv = { iterations: 30, psfSigma: 2.5, psfSize: 21, regularization: 0.002, deringing: false, deringThreshold: 0.25, useEmpiricalPsf: false };
    expect(deconvolveOptionsOf({ op: "rl_deconv", v: 1, iterations: 30, regularization: 0.002, deringing: false, deringingThreshold: 0.25, psf: { kind: "gaussian", sigma: 2.5, size: 21 } })).toEqual(deconv);
    expect(deconvolveOptionsOf({ op: "rl_deconv", v: 1, iterations: 30, regularization: 0.002, deringing: false, deringingThreshold: 0.25, psf: { kind: "empirical", numStars: 30, cutoutRadius: 15 } })?.useEmpiricalPsf).toBe(true);
    expect(deconvolveOptionsOf({ op: "arcsinh", v: 1, factor: 2 })).toBeNull();

    expect(backgroundOptionsOf({ op: "background", v: 1, model: "polynomial", config: { gridSize: 6, polyDegree: 2, sigmaClip: 2, iterations: 4, mode: "divide" } })).toEqual({
      model: "polynomial",
      params: { gridSize: 6, polyDegree: 2, sigmaClip: 2, iterations: 4, mode: "divide" },
    });
    expect(backgroundOptionsOf({ op: "background", v: 1, model: "deband_rows", config: { sigmaClip: 3, iterations: 2 } })).toEqual({
      model: "polynomial",
      params: { sigmaClip: 3, iterations: 2, mode: "deband_rows" },
    });
    const dbe: DbeConfig = { sampleRadius: 5, tolerance: 1, smoothing: 0.25, autoGrid: 10, manualSamples: [], rejectStars: true, mode: "subtract", normalize: false, maxSamples: 400 };
    expect(backgroundOptionsOf({ op: "background", v: 1, model: "spline", config: dbe })).toEqual({ model: "spline", dbe });
    expect(dbeConfigOf({ op: "background", v: 1, model: "spline", config: dbe })).toEqual(dbe);

    expect(waveletOptionsOf({ op: "wavelet_denoise", v: 1, numScales: 4, thresholds: [4, 3, 2, 1], linearDenoise: false, layerBias: [0.5, 0, 0, 0] })).toEqual({
      numScales: 4,
      thresholds: [4, 3, 2, 1],
      linear: false,
      layerBias: [0.5, 0, 0, 0],
    });
    expect(waveletOptionsOf({ op: "wavelet_denoise", v: 1, numScales: 2, thresholds: [3, 2], linearDenoise: true, layerBias: null })?.layerBias).toEqual([0, 0]);
    expect(arcsinhOptionsOf({ op: "arcsinh", v: 1, factor: 120 })).toEqual({ factor: 120 });
    expect(ghsOptionsOf({ op: "ghs", v: 1, d: 5, b: 2, sp: 0.1, lp: 0.01, hp: 0.9 })).toEqual({ stretchFactor: 5, localIntensity: 2, symmetryPoint: 0.1, shadowProtect: 0.01, highlightProtect: 0.9 });
    expect(maskedStretchOptionsOf({
      op: "masked_stretch",
      v: 1,
      iterations: 12,
      targetBackground: 0.2,
      maskGrowth: 3,
      maskSoftness: 5,
      detectionSigma: 6,
      maxEccentricity: 0.7,
      luminanceProtect: false,
      luminanceCeiling: 0.85,
      protectionAmount: 0.5,
      convergenceThreshold: 0.00001,
    })).toEqual({ iterations: 12, targetBackground: 0.2, maskGrowth: 3, maskSoftness: 5, protectionAmount: 0.5, luminanceProtect: false, detectionSigma: 6, maxEccentricity: 0.7 });

    const lhe: LheConfig = { kernelRadius: 32, contrastLimit: 3, amount: 0.7, histBits: 10, circular: false };
    expect(lheConfigOf({ op: "lhe", v: 1, ...lhe })).toEqual(lhe);
    const hdr: HdrConfig = { layers: 5, iterations: 2, overdrive: 0.1, inverted: true, toLightness: false, deringing: true, deringingAmount: 0.4 };
    expect(hdrmtConfigOf({ op: "hdrmt", v: 1, ...hdr })).toEqual(hdr);
    expect(pixelMathOptionsOf({ op: "pixelmath", v: 1, expression: "$T - A", slots: [{ name: "A", ref: "C:/a.fits" }], truncate: true, rescale: true })).toEqual({
      expression: "$T - A",
      slots: [{ name: "A", path: "C:/a.fits" }],
      truncate: true,
      rescale: true,
    });
    expect(toneOptionsOf({ op: "tone", v: 1, curve: { points: [[0, 0], [0.5, 0.7], [1, 1]] } })).toEqual({
      levels: { black: 0, gamma: 1, white: 1 },
      curve: [{ x: 0, y: 0 }, { x: 0.5, y: 0.7 }, { x: 1, y: 1 }],
    });
  });
});
