import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { clearCompanion, setCompanion, setFlagExclusion, setSdssFrameError, suggestCompanions } from "../companions";
import type { PlaneInfo } from "../../shared/types/fits.types";
import type { CompanionSuggestions } from "../../shared/types/layers";

const PATH = "C:/d/sci.fits";

const SUGGESTIONS: CompanionSuggestions = {
  role: { role: "science", basis: "EXTNAME SCI", assumed: false },
  err: { source: "C:/d/rms.fits", role: "sigma", basis: "file name -RMS" },
  dq: null,
  applied: false,
  candidates: {
    err: [{ source: "C:/d/rms.fits", role: "sigma", basis: "file name -RMS", usable: true, reason: "CRPIX1 differs: image 50.5 against layer 51.5" }],
    dq: [],
  },
  notes: [],
};

describe("companion layer commands", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins set_companion_cmd with a null kind by default", async () => {
    typedInvokeMock.mockResolvedValue({});
    await setCompanion(PATH, "err", "C:/d/wgt.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("set_companion_cmd", {
      path: PATH,
      role: "err",
      source: "C:/d/wgt.fits",
      layerRole: null,
    });
    await setCompanion(PATH, "err", "C:/d/wgt.fits", "variance");
    expect(typedInvokeMock).toHaveBeenLastCalledWith("set_companion_cmd", {
      path: PATH,
      role: "err",
      source: "C:/d/wgt.fits",
      layerRole: "variance",
    });
  });

  it("pins clear_companion_cmd, set_flag_exclusion_cmd and suggest_companions_cmd", async () => {
    typedInvokeMock.mockResolvedValue({});
    await clearCompanion(PATH, "dq");
    expect(typedInvokeMock).toHaveBeenLastCalledWith("clear_companion_cmd", { path: PATH, role: "dq", disable: false });
    await clearCompanion(PATH, "dq", true);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("clear_companion_cmd", { path: PATH, role: "dq", disable: true });
    await setFlagExclusion(PATH, null);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("set_flag_exclusion_cmd", { path: PATH, mask: null });
    await setFlagExclusion(PATH, 72);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("set_flag_exclusion_cmd", { path: PATH, mask: 72 });

    typedInvokeMock.mockResolvedValue(SUGGESTIONS);
    const suggested = await suggestCompanions(PATH);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("suggest_companions_cmd", { path: PATH });
    expect(suggested).toBe(SUGGESTIONS);
  });

  it("pins set_sdss_frame_error_cmd", async () => {
    const plane = { err_ref: "C:/d/frame-r.fits.bz2#array=__sdss_sigma.g4.76.d1.3225" };
    typedInvokeMock.mockResolvedValue(plane);
    const returned = await setSdssFrameError("C:/d/frame-r.fits.bz2", 4.76, 1.3225);
    expect(typedInvokeMock).toHaveBeenCalledWith("set_sdss_frame_error_cmd", {
      path: "C:/d/frame-r.fits.bz2",
      gain: 4.76,
      darkVariance: 1.3225,
    });
    expect(returned).toBe(plane);
  });

  it("types the recipe and the warnings of the plane description as optional", () => {
    expectTypeOf<NonNullable<PlaneInfo["err_recipe"]>["gain"]>().toEqualTypeOf<number | null>();
    expectTypeOf<PlaneInfo["warnings"]>().toEqualTypeOf<string[] | undefined>();
  });
});
