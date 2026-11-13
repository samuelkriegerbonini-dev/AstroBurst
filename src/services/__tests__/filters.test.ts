import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { identifyFilter, isIdentifiedFilter } from "../filters";
import type { IdentifiedFilter } from "../../shared/types/filters";

const DECAM_R: IdentifiedFilter = {
  dataset: "DECAM",
  filter: "r",
  pivot_um: 0.64176,
  instrument: "DECam",
  source: "header FILTER",
  warning: null,
};

describe("identifyFilter", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins identify_filter_cmd with a null dataset", async () => {
    typedInvokeMock.mockResolvedValue(DECAM_R);
    const answer = await identifyFilter("C:/d/a.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("identify_filter_cmd", { path: "C:/d/a.fits", dataset: null });
    expect(answer).toBe(DECAM_R);
  });

  it("forwards the dataset and a band plane key unchanged", async () => {
    typedInvokeMock.mockResolvedValue(DECAM_R);
    await identifyFilter("C:/d/cut.fits#hdu=0&plane=1", "DECAM");
    expect(typedInvokeMock).toHaveBeenCalledWith("identify_filter_cmd", { path: "C:/d/cut.fits#hdu=0&plane=1", dataset: "DECAM" });
  });

  it("tells an identified filter from an error answer", () => {
    expect(
      isIdentifiedFilter({ dataset: "DECAM", filter: "r", pivot_um: 0.64176, instrument: "DECam", source: "header FILTER", warning: null }),
    ).toBe(true);
    expect(isIdentifiedFilter({ error: "x" })).toBe(false);
    expectTypeOf<IdentifiedFilter["warning"]>().toEqualTypeOf<string | null>();
    expectTypeOf<IdentifiedFilter["pivot_um"]>().toEqualTypeOf<number>();
  });
});
