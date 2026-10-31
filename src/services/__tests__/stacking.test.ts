import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";
import {
  DRIZZLE_ALIGNMENT_METHODS,
  type DrizzleRgbResult,
  type StackFrameAlignment,
  type StackResult,
} from "../../shared/types/stacking";

const { withPreviewMock, typedInvokeMock, getPreviewUrlMock } = vi.hoisted(() => ({
  withPreviewMock: vi.fn(),
  typedInvokeMock: vi.fn(),
  getPreviewUrlMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  withPreview: withPreviewMock,
  typedInvoke: typedInvokeMock,
  getPreviewUrl: getPreviewUrlMock,
}));

import { channelOrNull, drizzleRgbStack, estimateMosaic, runCalibrationPipeline, runMosaic } from "../stacking";
import type { MosaicRequest, PipelineRequest } from "../../shared/types/stacking";

describe("channelOrNull", () => {
  it("passes a channel with enough frames through", () => {
    expect(channelOrNull(["a.fits", "b.fits"])).toEqual(["a.fits", "b.fits"]);
  });

  it("treats an unused channel as absent", () => {
    expect(channelOrNull([])).toBeNull();
  });

  it("drops a single-frame channel so the backend synthesises the plane", () => {
    expect(channelOrNull(["only.fits"])).toBeNull();
  });
});

describe("drizzleRgbStack", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue({});
  });

  it("sends a single-frame channel as null instead of refusing to run", async () => {
    await drizzleRgbStack(["only.fits"], ["g1.fits", "g2.fits"], ["b1.fits", "b2.fits"]);

    expect(withPreviewMock).toHaveBeenCalledTimes(1);
    const args = withPreviewMock.mock.calls[0][2] as {
      rPaths: string[] | null;
      gPaths: string[] | null;
      bPaths: string[] | null;
    };
    expect(args.rPaths).toBeNull();
    expect(args.gPaths).toEqual(["g1.fits", "g2.fits"]);
    expect(args.bPaths).toEqual(["b1.fits", "b2.fits"]);
  });

  it("forwards every channel that has enough frames", async () => {
    await drizzleRgbStack(
      ["r1.fits", "r2.fits"],
      ["g1.fits", "g2.fits"],
      ["b1.fits", "b2.fits"],
    );

    const args = withPreviewMock.mock.calls[0][2] as { rPaths: string[] | null };
    expect(args.rPaths).toEqual(["r1.fits", "r2.fits"]);
  });

  it("sends star-based alignment as affine, the method the backend runs", async () => {
    await drizzleRgbStack(["r1.fits", "r2.fits"], [], [], undefined, { alignmentMethod: "affine" });

    const args = withPreviewMock.mock.calls[0][2] as { alignmentMethod: string | null };
    expect(args.alignmentMethod).toBe("affine");
  });
});

describe("runCalibrationPipeline", () => {
  const request: PipelineRequest = {
    channels: [{ label: "R", paths: ["/subs/673nmos.fits"] }],
    dark_paths: [],
    flat_paths: [],
    bias_paths: [],
  };

  beforeEach(() => {
    withPreviewMock.mockReset();
    getPreviewUrlMock.mockReset();
    getPreviewUrlMock.mockImplementation(async (path: string) => `asset://localhost/${path}`);
  });

  it("asks the backend to write the outputs under the run name and resolves the RGB PNG to a viewer URL", async () => {
    withPreviewMock.mockResolvedValue({ masters: [] });

    await runCalibrationPipeline(request, "/out", "673nmos_pipeline3_20260923-142530-123");

    expect(withPreviewMock).toHaveBeenCalledWith(
      "run_pipeline_cmd",
      "/out",
      { request, name: "673nmos_pipeline3_20260923-142530-123" },
      [["rgb_png_path", "rgbPreviewUrl"]],
    );
  });

  it("gives every master a viewer URL for its own PNG and keeps the rest of the response", async () => {
    withPreviewMock.mockResolvedValue({
      rgbPreviewUrl: "asset://localhost//out/run_rgb.png",
      masters: [
        { label: "R", png_path: "/out/run_R.png", fits_path: "/out/run_R.fits", dimensions: [1600, 1600], input_path: "/subs/673nmos.fits" },
        { label: "G", png_path: "/out/run_G.png", fits_path: "/out/run_G.fits", dimensions: [1600, 1600], input_path: "/subs/656nmos.fits" },
      ],
    });

    const res = await runCalibrationPipeline(request);

    expect(res.masters?.map((m) => m.previewUrl)).toEqual(["asset://localhost//out/run_R.png", "asset://localhost//out/run_G.png"]);
    expect(res.masters?.[1].fits_path).toBe("/out/run_G.fits");
    expect(res.rgbPreviewUrl).toBe("asset://localhost//out/run_rgb.png");
  });
});

describe("mosaic commands", () => {
  const config: MosaicRequest = {
    canvas: { mode: "like", path: "/out/m42_ha_mosaic3.fits" },
    levels: "offset",
    references: [],
    feather_px: 64,
    dq_mask: true,
    reject: { mode: "auto" },
    solve: { online: false, use_twomass: true, max_sip_order: null },
  };
  const paths = ["/panels/m42_p1_OIII.fits", "/panels/m42_p2_OIII.fits"];

  beforeEach(() => {
    withPreviewMock.mockReset();
    typedInvokeMock.mockReset();
  });

  it("estimates through mosaic_estimate_cmd with the panels and the request, writing nothing", async () => {
    typedInvokeMock.mockResolvedValue({ cols: 10, rows: 10 });
    await expect(estimateMosaic(paths, config)).resolves.toEqual({ cols: 10, rows: 10 });
    expect(typedInvokeMock).toHaveBeenCalledWith("mosaic_estimate_cmd", { paths, config });
    expect(withPreviewMock).not.toHaveBeenCalled();
  });

  it("runs mosaic_cmd under the run name in the output folder and resolves its preview", async () => {
    withPreviewMock.mockResolvedValue({ fits_path: "/out/m42_p1_OIII_mosaic2.fits", previewUrl: "asset://m42.png" });
    const res = await runMosaic(paths, "/out", "m42_p1_OIII_mosaic2_20261031-090507-042", config);
    expect(withPreviewMock).toHaveBeenCalledWith("mosaic_cmd", "/out", { paths, name: "m42_p1_OIII_mosaic2_20261031-090507-042", config });
    expect(res.previewUrl).toBe("asset://m42.png");
  });
});

describe("drizzle alignment choices", () => {
  it("name the star-based option after the affine fit it runs, not a ZNCC search that does not exist", () => {
    expect(DRIZZLE_ALIGNMENT_METHODS.map((m) => m.value)).toEqual(["phase_correlation", "affine"]);
    expect(DRIZZLE_ALIGNMENT_METHODS.some((m) => /zncc/i.test(m.label))).toBe(false);
  });
});

describe("stack result payload types", () => {
  it("types offsets as the {dy, dx} objects Rust emits and declares the alignment and warning reports", () => {
    expectTypeOf<NonNullable<StackResult["offsets"]>[number]>().toEqualTypeOf<{ dy: number; dx: number }>();
    expectTypeOf<NonNullable<StackResult["alignment"]>[number]>().toEqualTypeOf<StackFrameAlignment>();
    expectTypeOf<StackFrameAlignment["confidence"]>().toEqualTypeOf<number | null>();
    expectTypeOf<StackResult["warnings"]>().toEqualTypeOf<string[] | undefined>();
    expectTypeOf<DrizzleRgbResult["warnings"]>().toEqualTypeOf<string[] | undefined>();
  });
});
