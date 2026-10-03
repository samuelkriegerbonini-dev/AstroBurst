import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock, withPreviewMock, getOutputDirMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  withPreviewMock: vi.fn(),
  getOutputDirMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: withPreviewMock,
  getOutputDir: getOutputDirMock,
  getPreviewUrl: vi.fn(),
}));

import {
  QSLOPE_PROGRESS_EVENT,
  cancelQuickSlope,
  compareWithRate,
  getRampFrame,
  getRampInfo,
  getRampPixelFit,
  getRampPixelSeries,
  getRampTables,
  rampSpectrumFromSeries,
  runQuickSlope,
} from "../ramp";
import { DEFAULT_QUICK_SLOPE_PARAMS, type RampPixelSeries } from "../../shared/types/ramp";

const UNCAL = "C:/fits/jw01266005001_02103_00001_nrs2_uncal.fits";

describe("ramp inspection services", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    getOutputDirMock.mockReset();
    typedInvokeMock.mockResolvedValue({});
  });

  it("getRampInfo invokes ramp_info_cmd with the path", async () => {
    typedInvokeMock.mockResolvedValueOnce({ source: "fits", ramp: null, frames: 10, width: 2048, height: 3200, array_key: null });
    const info = await getRampInfo(UNCAL);
    expect(typedInvokeMock).toHaveBeenCalledWith("ramp_info_cmd", { path: UNCAL });
    expect(info.frames).toBe(10);
  });

  it("getRampPixelSeries sends x, y and the integration", async () => {
    await getRampPixelSeries(UNCAL, 12, 700, 1);
    expect(typedInvokeMock).toHaveBeenCalledWith("ramp_pixel_series_cmd", { path: UNCAL, x: 12, y: 700, integration: 1 });
  });

  it("getRampTables sends maxRows, defaulting to 200", async () => {
    await getRampTables(UNCAL);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("ramp_tables_cmd", { path: UNCAL, maxRows: 200 });
    await getRampTables(UNCAL, 20);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("ramp_tables_cmd", { path: UNCAL, maxRows: 20 });
  });

  it("getRampFrame resolves ./output paths like getCubeFrame and sends frameIndex", async () => {
    getOutputDirMock.mockResolvedValue("C:/app/output");
    typedInvokeMock.mockResolvedValueOnce({ frame_index: 3, output_path: "C:/app/output/f_3.png", fits_path: null });
    const res = await getRampFrame("C:/roman/r.asdf#array=roman.data", 3, "./output/f_3.png", "./output/f_3.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("ramp_frame_cmd", {
      path: "C:/roman/r.asdf#array=roman.data",
      frameIndex: 3,
      outputPath: "C:/app/output/f_3.png",
      outputFits: "C:/app/output/f_3.fits",
    });
    expect(res.output_path).toBe("C:/app/output/f_3.png");

    await getRampFrame("C:/roman/r.asdf", 0, "D:/frames/f_0.png");
    expect(typedInvokeMock).toHaveBeenLastCalledWith("ramp_frame_cmd", {
      path: "C:/roman/r.asdf",
      frameIndex: 0,
      outputPath: "D:/frames/f_0.png",
      outputFits: undefined,
    });
  });

  it("getRampPixelFit sends the integration and the snake_case parameter object", async () => {
    const params = { ...DEFAULT_QUICK_SLOPE_PARAMS, ref_window_rows: null };
    await getRampPixelFit(UNCAL, 12, 700, 0, params);
    expect(typedInvokeMock).toHaveBeenCalledWith("ramp_pixel_fit_cmd", { path: UNCAL, x: 12, y: 700, integration: 0, params });
    expect(typedInvokeMock.mock.calls[0][1].params).toHaveProperty("ref_window_rows", null);
  });
});

describe("quick slope services", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue({});
  });

  it("runQuickSlope goes through withPreview with path, integration and params", async () => {
    await runQuickSlope(UNCAL, undefined, 0, DEFAULT_QUICK_SLOPE_PARAMS);
    expect(withPreviewMock).toHaveBeenCalledWith("ramp_quick_slope_cmd", undefined, {
      path: UNCAL,
      integration: 0,
      params: DEFAULT_QUICK_SLOPE_PARAMS,
    });
  });

  it("compareWithRate always sends a string ratePath with qslopePath and uncalRows and resolves the ratio preview", async () => {
    await compareWithRate("C:/out/x_qslope.fits", "C:/fits/x_rate.fits", "D:/cmp", [1300, 1500]);
    expect(withPreviewMock).toHaveBeenCalledWith(
      "ramp_compare_rate_cmd",
      "D:/cmp",
      { qslopePath: "C:/out/x_qslope.fits", ratePath: "C:/fits/x_rate.fits", uncalRows: [1300, 1500] },
      [["ratio_png_path", "ratioPreviewUrl"]],
    );
    await compareWithRate("C:/out/x_qslope.fits", "C:/fits/x_rate.fits", undefined, null);
    const args = withPreviewMock.mock.calls[1][2];
    expect(typeof args.ratePath).toBe("string");
    expect(args.uncalRows).toBeNull();
  });

  it("cancelQuickSlope cancels the qslope-progress event", async () => {
    typedInvokeMock.mockResolvedValueOnce(true);
    await expect(cancelQuickSlope()).resolves.toBe(true);
    expect(QSLOPE_PROGRESS_EVENT).toBe("qslope-progress");
    expect(typedInvokeMock).toHaveBeenCalledWith("cancel_progress_cmd", { event: "qslope-progress" });
  });
});

describe("rampSpectrumFromSeries", () => {
  it("plots DN against the group index: not spectral, no wavelengths", () => {
    const series: RampPixelSeries = {
      x: 12,
      y: 700,
      integration: 0,
      ngroups: 3,
      nints: 1,
      values: [31012, 31112, 31212],
      group_times_s: [0, 14.589, 29.178],
      unit: "DN",
    };
    expect(rampSpectrumFromSeries(series)).toEqual({
      values: [31012, 31112, 31212],
      wavelengths: [],
      x: 12,
      y: 700,
      is_spectral: false,
      flux_jy: null,
    });
  });
});
