import { describe, it, expect, beforeEach, vi } from "vitest";

const { withPreviewMock } = vi.hoisted(() => ({ withPreviewMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: vi.fn(),
  withPreview: withPreviewMock,
  getOutputDir: vi.fn(async () => "/out"),
}));

import { deconvolveRL } from "../processing";
import { compositeDeconvolveRL } from "../compositeChain";
import type { CompositeChainCall } from "../../shared/types/compositeChain";

const KERNEL = [[0, 1, 0], [1, 4, 1], [0, 1, 0]];
const STF = { shadow: 0, midtone: 0.5, highlight: 1 };
const CHAIN: CompositeChainCall = { chainInput: "base", displayStf: { r: STF, g: STF, b: STF, linked: true } };

function sentArgs(): Record<string, unknown> {
  expect(withPreviewMock).toHaveBeenCalledTimes(1);
  return withPreviewMock.mock.calls[0][2] as Record<string, unknown>;
}

describe("deconvolveRL psfKernel", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue({ psf_source: "provided" });
  });

  it("forwards the kernel to deconvolve_rl_cmd", async () => {
    const res = await deconvolveRL("/a.fits", "/out", { useEmpiricalPsf: true, psfKernel: KERNEL });
    expect(withPreviewMock.mock.calls[0][0]).toBe("deconvolve_rl_cmd");
    expect(sentArgs()).toMatchObject({ path: "/a.fits", useEmpiricalPsf: true, psfKernel: KERNEL });
    expect(res.psf_source).toBe("provided");
  });

  it("sends an explicit null kernel by default", async () => {
    await deconvolveRL("/a.fits", "/out");
    expect(sentArgs()).toHaveProperty("psfKernel", null);
  });
});

describe("compositeDeconvolveRL psfKernel", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue({ psf_source: "estimated" });
  });

  it("forwards the kernel to composite_deconvolve_rl_cmd next to the chain arguments", async () => {
    const res = await compositeDeconvolveRL("/out", CHAIN, { useEmpiricalPsf: true, psfKernel: KERNEL });
    expect(withPreviewMock.mock.calls[0][0]).toBe("composite_deconvolve_rl_cmd");
    expect(sentArgs()).toMatchObject({ chainInput: "base", useEmpiricalPsf: true, psfKernel: KERNEL });
    expect(res.psf_source).toBe("estimated");
  });

  it("sends an explicit null kernel by default", async () => {
    await compositeDeconvolveRL("/out", CHAIN);
    expect(sentArgs()).toHaveProperty("psfKernel", null);
  });
});
