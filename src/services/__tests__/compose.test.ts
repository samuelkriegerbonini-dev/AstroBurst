import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock, getOutputDirMock, getPreviewUrlMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  getOutputDirMock: vi.fn(),
  getPreviewUrlMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: getOutputDirMock,
  getPreviewUrl: getPreviewUrlMock,
}));

import { channelOverlayPreview, detectCropBounds } from "../compose";

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
