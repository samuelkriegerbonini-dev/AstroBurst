import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { getSpectralAxis, measureSpectralLine } from "../spectral";
import type { SpectrumSource } from "../../shared/types/spectral";

describe("measureSpectralLine", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("sends the pixel source with the optical convention and no model by default", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    const source: SpectrumSource = { kind: "pixel", x: 16, y: 16 };
    await measureSpectralLine("C:/d/cube.fits", source, { z0: 8, z1: 32 });
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_spectral_line_cmd", {
      path: "C:/d/cube.fits",
      source,
      z0: 8,
      z1: 32,
      continuum: null,
      restUm: null,
      convention: "optical",
      model: "none",
      velocityShiftKms: null,
    });
  });

  it("passes the region source, windows, rest wavelength, model and frame shift through", async () => {
    typedInvokeMock.mockResolvedValue({ flux: 1 });
    const source: SpectrumSource = {
      kind: "region",
      shape: { shape: "circle", x: 16, y: 16, r: 6 },
      background: { shape: "annulus", x: 16, y: 16, r_inner: 9, r_outer: 13 },
    };
    const result = await measureSpectralLine("cube.fits", source, {
      z0: 8,
      z1: 32,
      continuum: [
        [0, 5],
        [34, 39],
      ],
      restUm: 1.02,
      convention: "radio",
      model: "gaussian",
      velocityShiftKms: -12.5,
    });
    expect(result).toEqual({ flux: 1 });
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_spectral_line_cmd", {
      path: "cube.fits",
      source,
      z0: 8,
      z1: 32,
      continuum: [
        [0, 5],
        [34, 39],
      ],
      restUm: 1.02,
      convention: "radio",
      model: "gaussian",
      velocityShiftKms: -12.5,
    });
  });
});

describe("getSpectralAxis", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("asks the backend for the header axis of the given path", async () => {
    typedInvokeMock.mockResolvedValue({ kind: "wave" });
    await getSpectralAxis("cube.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("spectral_axis_cmd", { path: "cube.fits" });
  });
});
