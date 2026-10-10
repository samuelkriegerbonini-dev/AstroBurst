import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock, getOutputDirMock, getPreviewUrlMock, withPreviewMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  getOutputDirMock: vi.fn(),
  getPreviewUrlMock: vi.fn(),
  withPreviewMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: withPreviewMock,
  getOutputDir: getOutputDirMock,
  getPreviewUrl: getPreviewUrlMock,
}));

import { alignChannels, blendChannels, channelOverlayPreview, cropChannels, detectCropBounds } from "../compose";
import { measureChannelLevels } from "../channelLevels";
import { extractBackgroundBatch, spccCalibrate } from "../processing";

const overlayResponse = {
  png_path: "C:/out/channel_overlay_1_0_overlay.png",
  channel_previews: ["C:/out/channel_overlay_1_0_0.png", "C:/out/channel_overlay_1_0_1.png"],
  dimensions: [520, 400],
  preview_dimensions: [256, 197],
  elapsed_ms: 12,
};

describe("channelOverlayPreview", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    getOutputDirMock.mockReset();
    getPreviewUrlMock.mockReset();
    getPreviewUrlMock.mockImplementation(async (path: string) => `asset://${path}`);
  });

  it("sends the camelCase arguments of channel_overlay_preview_cmd with null and false defaults", async () => {
    getOutputDirMock.mockResolvedValue("C:/appdata/output");
    typedInvokeMock.mockResolvedValue({ ...overlayResponse, channel_previews: [] });

    await channelOverlayPreview(["__wizard_ch_r_aligned", "__wizard_ch_g_aligned"]);

    expect(typedInvokeMock).toHaveBeenCalledWith("channel_overlay_preview_cmd", {
      keys: ["__wizard_ch_r_aligned", "__wizard_ch_g_aligned"],
      maskKeys: null,
      withChannelFrames: false,
      outputDir: "C:/appdata/output",
      maxDim: null,
    });
  });

  it("passes explicit options through and skips the output dir lookup", async () => {
    typedInvokeMock.mockResolvedValue(overlayResponse);

    await channelOverlayPreview(["a", "b", "c"], {
      maskKeys: ["d"],
      withChannelFrames: true,
      maxDim: 1024,
      outputDir: "D:/explicit",
    });

    expect(getOutputDirMock).not.toHaveBeenCalled();
    expect(typedInvokeMock).toHaveBeenCalledWith("channel_overlay_preview_cmd", {
      keys: ["a", "b", "c"],
      maskKeys: ["d"],
      withChannelFrames: true,
      outputDir: "D:/explicit",
      maxDim: 1024,
    });
  });

  it("adds a preview URL for the overlay and one per channel frame, in order, without mutating the response", async () => {
    getOutputDirMock.mockResolvedValue("C:/out");
    const response = { ...overlayResponse };
    typedInvokeMock.mockResolvedValue(response);

    const result = await channelOverlayPreview(["a", "b"], { withChannelFrames: true });

    expect(result).toEqual({
      ...overlayResponse,
      previewUrl: "asset://C:/out/channel_overlay_1_0_overlay.png",
      channelPreviewUrls: [
        "asset://C:/out/channel_overlay_1_0_0.png",
        "asset://C:/out/channel_overlay_1_0_1.png",
      ],
    });
    expect(response).not.toHaveProperty("previewUrl");
  });
});

describe("detectCropBounds", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("invokes detect_crop_bounds_cmd with the paths and returns the bounds as sent", async () => {
    const bounds = {
      dimensions: [50, 40],
      crop_top: 6,
      crop_bottom: 4,
      crop_left: 3,
      crop_right: 9,
      auto_detected: true,
    };
    typedInvokeMock.mockResolvedValue(bounds);

    await expect(detectCropBounds(["k1", "k2"])).resolves.toEqual(bounds);
    expect(typedInvokeMock).toHaveBeenCalledWith("detect_crop_bounds_cmd", { paths: ["k1", "k2"] });
  });
});

describe("alignChannels", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    getOutputDirMock.mockReset();
    typedInvokeMock.mockResolvedValue({ channels: [], align_method: "phase_correlation", dimensions: [1, 1], elapsed_ms: 1 });
  });

  it("sends the chosen reference index and the run token", async () => {
    await alignChannels(["/a.fits", "/b.fits"], "D:/out", "affine", ["r", "g"], 1, "kq3x9z");
    expect(typedInvokeMock).toHaveBeenCalledWith("align_channels_cmd", {
      paths: ["/a.fits", "/b.fits"],
      outputDir: "D:/out",
      alignMethod: "affine",
      binIds: ["r", "g"],
      persistToDisk: false,
      referenceIndex: 1,
      runToken: "kq3x9z",
    });
  });

  it("sends null for an automatic reference and an untokened run", async () => {
    getOutputDirMock.mockResolvedValue("C:/appdata/output");
    await alignChannels(["/a.fits", "/b.fits"]);
    expect(typedInvokeMock).toHaveBeenCalledWith("align_channels_cmd", {
      paths: ["/a.fits", "/b.fits"],
      outputDir: "C:/appdata/output",
      alignMethod: "phase_correlation",
      binIds: null,
      persistToDisk: false,
      referenceIndex: null,
      runToken: null,
    });
  });
});

describe("cropChannels", () => {
  const cropped = { paths: [], dimensions: [1, 1], crop_top: 0, crop_bottom: 0, crop_left: 0, crop_right: 0, elapsed_ms: 1 };

  beforeEach(() => {
    typedInvokeMock.mockReset();
    getOutputDirMock.mockReset();
    typedInvokeMock.mockResolvedValue(cropped);
  });

  it("sends the run token of the Align run whose keys it crops", async () => {
    await cropChannels(["__wizard_ch_tkq3x9z_r_aligned"], "D:/out", 1, 2, 3, 4, false, ["r"], "kq3x9z");
    expect(typedInvokeMock).toHaveBeenCalledWith("crop_channels_cmd", {
      paths: ["__wizard_ch_tkq3x9z_r_aligned"],
      outputDir: "D:/out",
      top: 1,
      bottom: 2,
      left: 3,
      right: 4,
      autoDetect: false,
      binIds: ["r"],
      persistToDisk: false,
      runToken: "kq3x9z",
    });
  });

  it("sends a null run token by default", async () => {
    await cropChannels(["/a.fits"], "D:/out");
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({ binIds: null, runToken: null });
  });
});

describe("extractBackgroundBatch", () => {
  const batch = { results: [], mode: "subtract", rms_residual: null, dimensions: [1, 1], elapsed_ms: 1 };

  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue(batch);
  });

  it("sends the run token so the BG keys join the Align run", async () => {
    await extractBackgroundBatch(["k1", "k2"], ["r", "g"], "D:/out", { runToken: "kq3x9z" });
    expect(typedInvokeMock).toHaveBeenCalledWith("extract_background_batch_cmd", {
      paths: ["k1", "k2"],
      binIds: ["r", "g"],
      outputDir: "D:/out",
      gridSize: 8,
      polyDegree: 3,
      sigmaClip: 2.5,
      iterations: 3,
      mode: "subtract",
      referenceBin: null,
      runToken: "kq3x9z",
    });
  });

  it("sends a null run token when the options carry none", async () => {
    await extractBackgroundBatch(["k1"], ["r"], "D:/out");
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({ referenceBin: null, runToken: null });
  });
});

describe("measureChannelLevels", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("measures every path in one command and returns the per-path median and MAD", async () => {
    const levels = [
      { path: "__wizard_ch_tkq3x9z_r_aligned", median: 5.188, mad: 0.66491, sigma: 0.98580, valid_count: 9216 },
      { path: "/n/f187n.fits", median: 40.71, mad: 6.3634, sigma: 9.4344, valid_count: 9216 },
    ];
    typedInvokeMock.mockResolvedValue({ levels, elapsed_ms: 4 });
    await expect(measureChannelLevels(["__wizard_ch_tkq3x9z_r_aligned", "/n/f187n.fits"])).resolves.toEqual(levels);
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_channel_levels_cmd", {
      paths: ["__wizard_ch_tkq3x9z_r_aligned", "/n/f187n.fits"],
    });
  });
});

describe("blendChannels", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue({ png_path: "/out/blend.png", dimensions: [64, 64], elapsed_ms: 1 });
  });

  it("sends each weight row's level-match offset to the backend", async () => {
    const weights = [
      { channelIdx: 0, r: 9.5703, g: 0, b: 0, offset: -0.9342 },
      { channelIdx: 1, r: 0, g: 1, b: 0, offset: 0 },
      { channelIdx: 2, r: 0, g: 0, b: 1 },
    ];
    await blendChannels(["/r.fits", "/g.fits", "/b.fits"], weights, "D:/out", { preset: "rgb" });
    expect(withPreviewMock).toHaveBeenCalledWith("blend_channels_cmd", "D:/out", {
      channelPaths: ["/r.fits", "/g.fits", "/b.fits"],
      weights,
      preset: "rgb",
    });
  });
});

describe("spccCalibrate", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ r_factor: 1, g_factor: 1, b_factor: 1, stars_matched: 3, stars_total: 3, elapsed_ms: 1 });
  });

  it("sends the R, G and B filter wavelengths to the backend", async () => {
    await spccCalibrate("/r.fits", "/g.fits", "/b.fits", { catalog: "builtin", wavelengthsNm: [814, 555, 435] });
    expect(typedInvokeMock).toHaveBeenCalledWith("spcc_calibrate_cmd", {
      rPath: "/r.fits",
      gPath: "/g.fits",
      bPath: "/b.fits",
      wcsPath: null,
      whiteReference: "average_spiral",
      minSnr: 20,
      maxStars: 200,
      catalog: "builtin",
      wavelengthsNm: [814, 555, 435],
    });
  });

  it("sends null wavelengths by default, so the backend uses 640/530/460 nm", async () => {
    await spccCalibrate("/r.fits", "/g.fits", "/b.fits");
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({ catalog: "gaia", wavelengthsNm: null });
  });
});
