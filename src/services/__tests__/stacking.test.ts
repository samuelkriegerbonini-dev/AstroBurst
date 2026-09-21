import { describe, it, expect, beforeEach, vi } from "vitest";

const { withPreviewMock, typedInvokeMock } = vi.hoisted(() => ({
  withPreviewMock: vi.fn(),
  typedInvokeMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({
  withPreview: withPreviewMock,
  typedInvoke: typedInvokeMock,
}));

import { channelOrNull, drizzleRgbStack } from "../stacking";

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
});
