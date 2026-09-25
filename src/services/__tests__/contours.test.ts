import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { contourLines } from "../contours";
import { CONTOUR_MODES } from "../../shared/types/contours";

describe("contourLines", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ levels: [], bin: 1, n_points: 0, notes: [] });
  });

  it("offers every mode LevelMode::parse accepts", () => {
    expect(CONTOUR_MODES).toEqual(["list", "linear", "log", "sqrt", "sigma"]);
  });

  it("sends the command name and nulls for every unset option", async () => {
    await contourLines("/a.fits", { mode: "sigma" });
    expect(typedInvokeMock).toHaveBeenCalledWith("contour_lines_cmd", {
      path: "/a.fits",
      mode: "sigma",
      levels: null,
      nLevels: null,
      lo: null,
      hi: null,
      sigmaMultiples: null,
      smoothSigma: null,
      bin: null,
      excludeDq: false,
    });
  });

  it("passes every option through as camelCase arguments", async () => {
    await contourLines("/b.fits", {
      mode: "log",
      levels: [1, 2],
      nLevels: 5,
      lo: 0.5,
      hi: 50,
      sigmaMultiples: [3, 5],
      smoothSigma: 1.5,
      bin: 2,
      excludeDq: true,
    });
    expect(typedInvokeMock).toHaveBeenCalledWith("contour_lines_cmd", {
      path: "/b.fits",
      mode: "log",
      levels: [1, 2],
      nLevels: 5,
      lo: 0.5,
      hi: 50,
      sigmaMultiples: [3, 5],
      smoothSigma: 1.5,
      bin: 2,
      excludeDq: true,
    });
  });
});
