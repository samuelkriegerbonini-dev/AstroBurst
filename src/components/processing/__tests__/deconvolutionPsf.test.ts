import { describe, expect, it, vi, beforeEach } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, clicks, deconvolveMock, compositeDeconvolveMock, clickDuringRender } = vi.hoisted(() => {
  const clicks = new Set<string>();
  type Factory = (type: unknown, props: Record<string, unknown> | null, ...rest: unknown[]) => unknown;
  const clickDuringRender = (factory: Factory): Factory => (type, props, ...rest) => {
    const id = props?.["data-testid"];
    if (typeof type === "string" && typeof id === "string" && clicks.delete(id)) ((props?.onClick ?? props?.onChange) as () => void)();
    return factory(type, props, ...rest);
  };
  return {
    captured: { onClick: null as null | (() => void) },
    clicks,
    deconvolveMock: vi.fn(),
    compositeDeconvolveMock: vi.fn(),
    clickDuringRender,
  };
});

vi.mock("react/jsx-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-runtime")>();
  return { ...actual, jsx: clickDuringRender(actual.jsx as never), jsxs: clickDuringRender(actual.jsxs as never) };
});

vi.mock("react/jsx-dev-runtime", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react/jsx-dev-runtime")>();
  return { ...actual, jsxDEV: clickDuringRender(actual.jsxDEV as never) };
});

vi.mock("../../ui", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ui")>();
  return {
    ...actual,
    RunButton: (props: { onClick: () => void }) => {
      captured.onClick = props.onClick;
      return null;
    },
  };
});

vi.mock("../../../services/processing", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/processing")>()),
  recordDeconvolveRL: deconvolveMock,
}));

vi.mock("../../../services/compositeChain", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/compositeChain")>()),
  compositeDeconvolveRL: compositeDeconvolveMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import { historyRerunStore } from "../../../utils/historyRerunStore";
import DeconvolutionPanel from "../DeconvolutionPanel";
import {
  ESTIMATED_PSF_HINT,
  GAUSSIAN_PSF_HINT,
  MEASURED_PSF_CHIP,
  MEASURED_PSF_FALLBACK,
  NO_MEASURED_PSF_REASON,
  PSF_CHOICES,
  PSF_TAB_KERNEL_HINT,
  deconvPanelSettingsOf,
  effectivePsfChoice,
  measuredPsfNote,
  psfChoiceHint,
  psfChoiceLabel,
  psfRequest,
  psfSourceLabel,
  type PsfChoice,
} from "../deconvPsf";
import type { CompositeInputView } from "../compositeProps";
import type { ProcessedFile } from "../../../shared/types";
import type { OpSpec, PsfSpec } from "../../../shared/types/ops";

const KERNEL = [[0, 1, 0], [1, 4, 1], [0, 1, 0]];
const COMPOSITE_INPUT: CompositeInputView = { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" };
const DECONV_STEP = { op: "rl_deconv", v: 1, iterations: 30, regularization: 0.002, deringing: true, deringingThreshold: 0.1 } as const;
const KERNEL_PSF: PsfSpec = { kind: "kernel", hash: "abc", rows: 3, cols: 3 };
const ESTIMATED_PSF: PsfSpec = { kind: "empirical", numStars: 30, cutoutRadius: 15 };

const FILE: ProcessedFile = {
  id: "f1",
  name: "m42.fits",
  path: "C:/data/m42.fits",
  sourcePath: "C:/data/m42.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: null,
  error: null,
  startedAt: null,
  finishedAt: null,
};

function render(props: { psfKernel: number[][] | null; compositeMode: boolean }): string {
  const element = createElement(DeconvolutionPanel, {
    selectedFile: FILE,
    fileKey: FILE.path,
    compositeInput: props.compositeMode ? COMPOSITE_INPUT : null,
    onCompositeDone: () => {},
    fileName: FILE.name,
    ...props,
  });
  const preview = createElement(PreviewProvider, { file: FILE, doneFiles: [], children: element });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function renderRerun(psf: PsfSpec, psfKernel: number[][] | null): string {
  historyRerunStore.request({ section: "deconvolution", op: { ...DECONV_STEP, psf }, node: "n0002", downstream: false, scope: { imageKey: FILE.path, basePath: null } });
  return render({ psfKernel, compositeMode: false });
}

function radio(html: string, choice: PsfChoice): string {
  const tag = new RegExp(`<input[^>]*value="${choice}"[^>]*>`).exec(html)?.[0];
  if (!tag) throw new Error(`no ${choice} radio`);
  return tag;
}

function checkedChoices(html: string): PsfChoice[] {
  return PSF_CHOICES.filter((choice) => radio(html, choice).includes('checked=""'));
}

function slidersShown(html: string): string[] {
  return [...html.matchAll(/<label class="ab-slider-label"[^>]*>([^<]*)/g)].map((m) => m[1]);
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function groupDescription(html: string): string[] {
  const group = /<div[^>]*role="radiogroup"[^>]*>/.exec(html)?.[0] ?? "";
  const ids = /aria-describedby="([^"]+)"/.exec(group)?.[1].split(" ") ?? [];
  return ids.map((id) => new RegExp(`id="${escapeRegExp(id)}"[^>]*>([^<]*)<`).exec(html)?.[1] ?? `missing #${id}`);
}

async function runPanel(compositeMode: boolean): Promise<Record<string, unknown>> {
  const mock = compositeMode ? compositeDeconvolveMock : deconvolveMock;
  captured.onClick?.();
  await vi.waitFor(() => expect(mock).toHaveBeenCalledTimes(1));
  return mock.mock.calls[0][2] as Record<string, unknown>;
}

beforeEach(() => {
  clicks.clear();
  historyRerunStore.clear();
});

function armRunMocks(): void {
  captured.onClick = null;
  deconvolveMock.mockReset();
  compositeDeconvolveMock.mockReset();
  deconvolveMock.mockResolvedValue({ png_path: "/out/d.png", dimensions: [4, 4], elapsed_ms: 1, iterations_run: 20, psf_source: "gaussian" });
  compositeDeconvolveMock.mockResolvedValue({ previewUrl: "asset://out/c.png", iterations_run: [20, 20, 20], convergence: [0, 0, 0], psf_source: "gaussian" });
}

describe("psfRequest", () => {
  it("sends the PSF-tab kernel only for Measured with a kernel, and estimates when Measured has none", () => {
    expect(psfRequest("gaussian", KERNEL)).toEqual({ useEmpiricalPsf: false, psfKernel: null });
    expect(psfRequest("measured", KERNEL).psfKernel).toBe(KERNEL);
    expect(psfRequest("measured", KERNEL).useEmpiricalPsf).toBe(true);
    expect(psfRequest("estimate", KERNEL)).toEqual({ useEmpiricalPsf: true, psfKernel: null });
    expect(psfRequest("measured", null)).toEqual({ useEmpiricalPsf: true, psfKernel: null });
    expect(psfRequest("measured", undefined)).toEqual({ useEmpiricalPsf: true, psfKernel: null });
    expect(psfRequest("measured", [])).toEqual({ useEmpiricalPsf: true, psfKernel: null });
  });
});

describe("PSF choice", () => {
  it("labels Measured with the kernel's own size and describes only the active choice", () => {
    expect(psfChoiceLabel("gaussian", KERNEL)).toBe("Gaussian σ");
    expect(psfChoiceLabel("measured", KERNEL)).toBe("Measured (PSF tab, 3×3)");
    expect(psfChoiceLabel("measured", [[1, 2, 3, 4, 5]])).toBe("Measured (PSF tab, 1×5)");
    expect(psfChoiceLabel("measured", null)).toBe("Measured (PSF tab)");
    expect(psfChoiceLabel("estimate", KERNEL)).toBe("Estimate from this image");
    expect(psfChoiceHint("gaussian")).toBe(GAUSSIAN_PSF_HINT);
    expect(psfChoiceHint("measured")).toBe(PSF_TAB_KERNEL_HINT);
    expect(psfChoiceHint("estimate")).toBe(ESTIMATED_PSF_HINT);
  });

  it("falls back from Measured to Estimate when no kernel is loaded and says why", () => {
    expect(effectivePsfChoice("measured", KERNEL)).toBe("measured");
    expect(effectivePsfChoice("measured", null)).toBe("estimate");
    expect(effectivePsfChoice("gaussian", null)).toBe("gaussian");
    expect(effectivePsfChoice("estimate", KERNEL)).toBe("estimate");
    expect(measuredPsfNote("measured", KERNEL)).toBeNull();
    expect(measuredPsfNote("gaussian", KERNEL)).toBeNull();
    expect(measuredPsfNote("gaussian", null)).toBe(NO_MEASURED_PSF_REASON);
    expect(measuredPsfNote("measured", null)).toBe(MEASURED_PSF_FALLBACK);
  });

  it("decodes a recorded step's PSF kind into the panel's choice", () => {
    expect(deconvPanelSettingsOf({ ...DECONV_STEP, psf: { kind: "gaussian", sigma: 2.5, size: 21 } })).toEqual({
      iterations: 30,
      psfSigma: 2.5,
      psfSize: 21,
      regularization: 0.002,
      deringing: true,
      deringThreshold: 0.1,
      psf: "gaussian",
    });
    expect(deconvPanelSettingsOf({ ...DECONV_STEP, psf: KERNEL_PSF })?.psf).toBe("measured");
    expect(deconvPanelSettingsOf({ ...DECONV_STEP, psf: ESTIMATED_PSF })?.psf).toBe("estimate");
    expect(deconvPanelSettingsOf({ op: "arcsinh", v: 1, factor: 2 } satisfies OpSpec)).toBeNull();
  });

  it.each([false, true])("with a measured PSF keeps Gaussian, offers the chip and drops the kernel hint (composite mode = %s)", (compositeMode) => {
    const html = render({ psfKernel: KERNEL, compositeMode });
    expect(html).not.toContain(PSF_TAB_KERNEL_HINT);
    expect(html).toContain(MEASURED_PSF_CHIP);
    expect(html).toContain('role="radiogroup"');
    expect(checkedChoices(html)).toEqual(["gaussian"]);
    expect(groupDescription(html)).toEqual([GAUSSIAN_PSF_HINT]);
    expect(radio(html, "measured")).not.toContain('disabled=""');
    expect(html).toContain("Measured (PSF tab, 3×3)");
    expect(html).toContain("PSF Sigma");
  });

  it.each([false, true])("without a measured PSF disables Measured and gives its reason (composite mode = %s)", (compositeMode) => {
    const html = render({ psfKernel: null, compositeMode });
    expect(radio(html, "measured")).toContain('disabled=""');
    expect(checkedChoices(html)).toEqual(["gaussian"]);
    expect(radio(html, "gaussian")).not.toContain('disabled=""');
    expect(radio(html, "estimate")).not.toContain('disabled=""');
    expect(groupDescription(html)).toEqual([GAUSSIAN_PSF_HINT, NO_MEASURED_PSF_REASON]);
    expect(html).not.toContain(MEASURED_PSF_CHIP);
    expect(html).not.toContain("Empirical PSF");
  });
});

describe("psfSourceLabel", () => {
  it("labels each backend PSF source", () => {
    expect(psfSourceLabel("gaussian")).toBe("Gaussian");
    expect(psfSourceLabel("estimated")).toBe("Estimated");
    expect(psfSourceLabel("provided")).toBe("PSF tab");
    expect(psfSourceLabel(undefined)).toBeNull();
  });
});

describe("DeconvolutionPanel run with Empirical PSF off", () => {
  beforeEach(() => {
    captured.onClick = null;
    deconvolveMock.mockReset();
    compositeDeconvolveMock.mockReset();
    deconvolveMock.mockResolvedValue({ png_path: "/out/d.png", dimensions: [4, 4], elapsed_ms: 1, iterations_run: 20, psf_source: "gaussian" });
    compositeDeconvolveMock.mockResolvedValue({ previewUrl: "asset://out/c.png", iterations_run: [20, 20, 20], convergence: [0, 0, 0], psf_source: "gaussian" });
  });

  it("does not send the PSF-tab kernel in file mode", async () => {
    render({ psfKernel: KERNEL, compositeMode: false });
    captured.onClick?.();
    await vi.waitFor(() => expect(deconvolveMock).toHaveBeenCalledTimes(1));
    expect(deconvolveMock.mock.calls[0][0]).toEqual({ imageKey: FILE.path, basePath: null });
    expect(deconvolveMock.mock.calls[0][2]).toHaveProperty("psfKernel", null);
    expect(deconvolveMock.mock.calls[0][2]).toHaveProperty("useEmpiricalPsf", false);
  });

  it("does not send the PSF-tab kernel in composite mode", async () => {
    render({ psfKernel: KERNEL, compositeMode: true });
    captured.onClick?.();
    await vi.waitFor(() => expect(compositeDeconvolveMock).toHaveBeenCalledTimes(1));
    expect(compositeDeconvolveMock.mock.calls[0][2]).toHaveProperty("psfKernel", null);
    expect(compositeDeconvolveMock.mock.calls[0][2]).toHaveProperty("useEmpiricalPsf", false);
  });
});

describe("DeconvolutionPanel run with the PSF choice", () => {
  beforeEach(armRunMocks);

  it.each([false, true])("the chip selects Measured and the run sends the PSF-tab kernel (composite mode = %s)", async (compositeMode) => {
    clicks.add("deconv-psf-chip");
    const html = render({ psfKernel: KERNEL, compositeMode });
    expect(checkedChoices(html)).toEqual(["measured"]);
    expect(html).not.toContain(MEASURED_PSF_CHIP);
    expect(groupDescription(html)).toEqual([PSF_TAB_KERNEL_HINT]);
    expect(html).not.toContain("PSF Sigma");
    const options = await runPanel(compositeMode);
    expect(options.useEmpiricalPsf).toBe(true);
    expect(options.psfKernel).toBe(KERNEL);
  });

  it("Estimate with a measured PSF loaded estimates from the image instead of sending the kernel", async () => {
    clicks.add("deconv-psf-estimate");
    const html = render({ psfKernel: KERNEL, compositeMode: false });
    expect(checkedChoices(html)).toEqual(["estimate"]);
    expect(groupDescription(html)).toEqual([ESTIMATED_PSF_HINT]);
    expect(html).not.toContain(MEASURED_PSF_CHIP);
    const options = await runPanel(false);
    expect(options).toMatchObject({ useEmpiricalPsf: true, psfKernel: null });
  });
});

describe("Re-run with on a deconvolution step", () => {
  beforeEach(armRunMocks);

  it("opens Measured for a step that used the PSF-tab kernel while that kernel is loaded", async () => {
    const html = renderRerun(KERNEL_PSF, KERNEL);
    expect(checkedChoices(html)).toEqual(["measured"]);
    expect(slidersShown(html)).toEqual(["Iterations", "Regularization", "Dering Threshold"]);
    const options = await runPanel(false);
    expect(options.useEmpiricalPsf).toBe(true);
    expect(options.psfKernel).toBe(KERNEL);
    expect(options.iterations).toBe(30);
  });

  it("falls back to Estimate and says so when the step's kernel is not loaded", async () => {
    const html = renderRerun(KERNEL_PSF, null);
    expect(checkedChoices(html)).toEqual(["estimate"]);
    expect(radio(html, "measured")).toContain('disabled=""');
    expect(groupDescription(html)).toEqual([ESTIMATED_PSF_HINT, MEASURED_PSF_FALLBACK]);
    expect(html).toContain('role="note"');
    const options = await runPanel(false);
    expect(options).toMatchObject({ useEmpiricalPsf: true, psfKernel: null });
  });

  it("keeps Estimate for a step that estimated its PSF, even with a measured PSF loaded", async () => {
    const html = renderRerun(ESTIMATED_PSF, KERNEL);
    expect(checkedChoices(html)).toEqual(["estimate"]);
    expect(html).not.toContain(MEASURED_PSF_CHIP);
    const options = await runPanel(false);
    expect(options).toMatchObject({ useEmpiricalPsf: true, psfKernel: null });
  });
});
