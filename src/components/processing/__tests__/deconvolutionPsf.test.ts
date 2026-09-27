import { describe, expect, it, vi, beforeEach } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { captured, deconvolveMock, compositeDeconvolveMock } = vi.hoisted(() => ({
  captured: { onClick: null as null | (() => void) },
  deconvolveMock: vi.fn(),
  compositeDeconvolveMock: vi.fn(),
}));

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
  deconvolveRL: deconvolveMock,
}));

vi.mock("../../../services/compositeChain", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/compositeChain")>()),
  compositeDeconvolveRL: compositeDeconvolveMock,
}));

import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import DeconvolutionPanel from "../DeconvolutionPanel";
import {
  ESTIMATED_PSF_HINT,
  PSF_TAB_KERNEL_HINT,
  empiricalPsfHint,
  psfSourceLabel,
  requestedPsfKernel,
} from "../deconvPsf";
import type { CompositeInputView } from "../compositeProps";
import type { ProcessedFile } from "../../../shared/types";

const KERNEL = [[0, 1, 0], [1, 4, 1], [0, 1, 0]];
const COMPOSITE_INPUT: CompositeInputView = { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" };

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

describe("requestedPsfKernel", () => {
  it("sends the PSF-tab kernel only when Empirical PSF is on and a kernel exists", () => {
    expect(requestedPsfKernel(true, KERNEL)).toBe(KERNEL);
    expect(requestedPsfKernel(false, KERNEL)).toBeNull();
    expect(requestedPsfKernel(true, null)).toBeNull();
    expect(requestedPsfKernel(true, undefined)).toBeNull();
  });
});

describe("empirical PSF hint", () => {
  it("names the PSF-tab kernel when one is available", () => {
    expect(empiricalPsfHint(KERNEL)).toBe("Uses the kernel from the PSF tab");
    expect(empiricalPsfHint(null)).toBe("Estimates the PSF from this image");
  });

  it.each([false, true])("renders under the toggle in composite mode = %s", (compositeMode) => {
    const withKernel = render({ psfKernel: KERNEL, compositeMode });
    expect(withKernel).toContain(PSF_TAB_KERNEL_HINT);
    expect(withKernel).not.toContain(ESTIMATED_PSF_HINT);
    const withoutKernel = render({ psfKernel: null, compositeMode });
    expect(withoutKernel).toContain(ESTIMATED_PSF_HINT);
    expect(withoutKernel).not.toContain(PSF_TAB_KERNEL_HINT);
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
