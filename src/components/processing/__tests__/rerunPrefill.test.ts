import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";

interface SliderProps {
  label: string;
  value: number;
  format?: (v: number) => string;
}

interface ToggleProps {
  label: string;
  checked: boolean;
}

const { sliders, toggles, curves } = vi.hoisted(() => ({
  sliders: [] as SliderProps[],
  toggles: [] as ToggleProps[],
  curves: [] as { x: number; y: number }[][],
}));

vi.mock("../../ui", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ui")>()),
  Slider: (props: SliderProps) => {
    sliders.push(props);
    return null;
  },
  Toggle: (props: ToggleProps) => {
    toggles.push(props);
    return null;
  },
}));

vi.mock("../../compose/CurveEditor", () => ({
  default: (props: { points: { x: number; y: number }[] }) => {
    curves.push(props.points);
    return null;
  },
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf } from "../../../context/PreviewContext";
import { historyRerunStore, type RerunSection } from "../../../utils/historyRerunStore";
import { rerunUpdate } from "../../../hooks/useRerunParams";
import type { OpSpec } from "../../../shared/types/ops";
import type { ProcessedFile } from "../../../shared/types";
import CurvesPanel from "../CurvesPanel";
import WaveletPanel from "../WaveletPanel";
import DeconvolutionPanel from "../DeconvolutionPanel";
import ArcsinhStretchPanel from "../ArcsinhStretchPanel";
import MaskedStretchPanel from "../MaskedStretchPanel";
import LocalContrastPanel from "../LocalContrastPanel";
import HdrPanel from "../HdrPanel";
import BackgroundPanel from "../BackgroundPanel";
import PixelMathPanel from "../PixelMathPanel";

const PATH = "C:/data/nrca1_i2d.fits";
const SCOPE = { imageKey: PATH, basePath: null };

function fileOf(id: string): ProcessedFile {
  return {
    id,
    name: "nrca1_i2d.fits",
    path: PATH,
    sourcePath: PATH,
    imageRef: null,
    size: 1,
    status: "done",
    result: { png_path: "C:/out/a.png", previewUrl: "asset://a.png", dimensions: [2048, 2048], elapsed_ms: 1, is_rgb: false },
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function renderPrefilled(section: RerunSection, op: OpSpec, panel: unknown): string {
  sliders.length = 0;
  toggles.length = 0;
  curves.length = 0;
  historyRerunStore.request({ section, op, node: "n0002", downstream: false, scope: SCOPE });
  const file = fileOf(`prefill-${section}`);
  const props = { selectedFile: file, outputDir: "C:/out", fileKey: fileKeyOf(file), compositeMode: false, compositeInput: null, onCompositeDone: () => {}, fileName: file.name };
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: createElement(panel as ComponentType<typeof props>, props) });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function slider(label: string): SliderProps {
  const found = sliders.find((s) => s.label === label);
  if (!found) throw new Error(`no slider ${label}: ${sliders.map((s) => s.label).join(", ")}`);
  return found;
}

function toggle(label: string): boolean {
  const found = toggles.find((t) => t.label === label);
  if (!found) throw new Error(`no toggle ${label}: ${toggles.map((t) => t.label).join(", ")}`);
  return found.checked;
}

beforeEach(() => {
  historyRerunStore.clear();
});

describe("rerun_opens_the_panel_prefilled", () => {
  it("Curves takes the levels and the curve of the tone step", () => {
    renderPrefilled("curves", { op: "tone", v: 1, levels: { black: 0.1, gamma: 1.4, white: 0.9 }, curve: { points: [[0, 0], [0.5, 0.7], [1, 1]] } }, CurvesPanel);
    expect(slider("Black point").value).toBe(0.1);
    expect(slider("Gamma").value).toBe(1.4);
    expect(slider("White point").value).toBe(0.9);
    expect(curves.at(-1)).toEqual([{ x: 0, y: 0 }, { x: 0.5, y: 0.7 }, { x: 1, y: 1 }]);
  });

  it("Wavelet takes the scales, thresholds, layer bias and threshold kind", () => {
    renderPrefilled("denoise", { op: "wavelet_denoise", v: 1, numScales: 4, thresholds: [4, 3, 2, 1.5], linearDenoise: false, layerBias: [0.5, 0, 0, -0.25] }, WaveletPanel);
    expect(slider("Scales").value).toBe(4);
    expect(sliders.filter((s) => s.label === "").map((s) => s.value)).toEqual([4, 3, 2, 1.5, 0.5, 0, 0, -0.25]);
    expect(toggle("Soft threshold (linear)")).toBe(false);
  });

  it("Deconvolution takes the iterations, PSF and regularisation", () => {
    renderPrefilled(
      "deconvolution",
      { op: "rl_deconv", v: 1, iterations: 30, regularization: 0.002, deringing: false, deringingThreshold: 0.25, psf: { kind: "gaussian", sigma: 2.5, size: 21 } },
      DeconvolutionPanel,
    );
    expect(slider("Iterations").value).toBe(30);
    expect(slider("PSF Sigma").value).toBe(2.5);
    expect(slider("PSF Size").value).toBe(21);
    expect(slider("Regularization").value).toBe(0.002);
    expect(toggle("Deringing")).toBe(false);
  });

  it("Arcsinh takes the stretch factor", () => {
    renderPrefilled("stretch", { op: "arcsinh", v: 1, factor: 120 }, ArcsinhStretchPanel);
    expect(slider("Stretch Factor (S)").format?.(0)).toBe("120.0");
  });

  it("Masked stretch takes its targets and protection", () => {
    renderPrefilled(
      "masked_stretch",
      {
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
      },
      MaskedStretchPanel,
    );
    expect(slider("Target Background").value).toBe(0.2);
    expect(slider("Star Protection").value).toBe(0.5);
    expect(slider("Mask Growth").value).toBe(3);
    expect(slider("Mask Softness").value).toBe(5);
    expect(toggle("Luminance Protection")).toBe(false);
  });

  it("LHE takes its kernel, limit, amount and shape", () => {
    renderPrefilled("local_contrast", { op: "lhe", v: 1, kernelRadius: 32, contrastLimit: 3, amount: 0.7, histBits: 10, circular: false }, LocalContrastPanel);
    expect(slider("Kernel Radius").value).toBe(32);
    expect(slider("Contrast Limit").value).toBe(3);
    expect(slider("Amount").value).toBe(0.7);
    expect(toggle("Circular Kernel")).toBe(false);
  });

  it("HDRMT takes its layers, iterations and switches", () => {
    renderPrefilled(
      "hdr",
      { op: "hdrmt", v: 1, layers: 5, iterations: 2, overdrive: 0.1, inverted: true, toLightness: false, deringing: true, deringingAmount: 0.4 },
      HdrPanel,
    );
    expect(slider("Layers").value).toBe(5);
    expect(slider("Iterations").value).toBe(2);
    expect(slider("Overdrive").value).toBe(0.1);
    expect(toggle("Invert (compress dark structures)")).toBe(true);
    expect(toggle("Deringing (protect star cores)")).toBe(true);
    expect(slider("Deringing Amount").value).toBe(0.4);
  });

  it("Background takes the polynomial settings, a deband mode or the DBE settings", () => {
    const polynomial = renderPrefilled(
      "background",
      { op: "background", v: 1, model: "polynomial", config: { gridSize: 6, polyDegree: 2, sigmaClip: 2, iterations: 4, mode: "divide" } },
      BackgroundPanel,
    );
    expect(slider("Grid Size").value).toBe(6);
    expect(slider("Polynomial Degree").value).toBe(2);
    expect(slider("Sigma Clip").value).toBe(2);
    expect(slider("Iterations").value).toBe(4);
    expect(polynomial).toMatch(/<option value="divide" selected="">/);

    const deband = renderPrefilled(
      "background",
      { op: "background", v: 1, model: "deband_rows", config: { sigmaClip: 3, iterations: 2 } },
      BackgroundPanel,
    );
    expect(deband).toMatch(/<option value="deband_rows" selected="">/);
    expect(slider("Sigma Clip").value).toBe(3);

    const spline = renderPrefilled(
      "background",
      {
        op: "background",
        v: 1,
        model: "spline",
        config: { sampleRadius: 7, tolerance: 1.5, smoothing: 0.4, autoGrid: 12, manualSamples: [], rejectStars: false, mode: "subtract", normalize: true, maxSamples: 400 },
      },
      BackgroundPanel,
    );
    expect(spline).toMatch(/<option value="spline" selected="">/);
    expect(slider("Sample Radius (px)").value).toBe(7);
    expect(slider("Tolerance (sigma)").value).toBe(1.5);
    expect(toggle("Reject stars")).toBe(false);
    expect(toggle("Keep image median")).toBe(true);
  });

  it("PixelMath takes the expression, slots and output flags", () => {
    const html = renderPrefilled(
      "pixelmath",
      { op: "pixelmath", v: 1, expression: "$T - 0.5 * Ha", slots: [{ name: "Ha", ref: "C:/data/ha.fits" }], truncate: true, rescale: true },
      PixelMathPanel,
    );
    expect(html).toContain("$T - 0.5 * Ha");
    expect(html).toContain('value="Ha"');
    expect(toggle("Truncate result to [0, 1]")).toBe(true);
    expect(toggle("Rescale result to [0, 1]")).toBe(true);
  });
});

describe("rerunUpdate", () => {
  const op: OpSpec = { op: "arcsinh", v: 1, factor: 20 };
  const decode = (o: OpSpec) => (o.op === "arcsinh" ? { factor: o.factor } : null);

  it("applies a request that arrives after the panel mounted and consumes the one it mounted with", () => {
    expect(rerunUpdate({ seq: 3, section: "stretch", op, node: "n0003", downstream: false, scope: SCOPE }, 2, decode)).toEqual({ factor: 20 });
    expect(rerunUpdate({ seq: 2, section: "stretch", op, node: "n0003", downstream: false, scope: SCOPE }, 2, decode)).toBeNull();
    expect(rerunUpdate({ seq: 4, section: "stretch", op: { op: "tone", v: 1 }, node: "n0003", downstream: false, scope: SCOPE }, null, decode)).toBeNull();
  });
});
