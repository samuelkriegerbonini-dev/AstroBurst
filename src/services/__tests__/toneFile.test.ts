import { beforeEach, describe, expect, it, vi } from "vitest";

const { withPreviewMock } = vi.hoisted(() => ({ withPreviewMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ withPreview: withPreviewMock }));

import { TONE_IDENTITY_MESSAGE, applyToneFile, isToneIdentity, type ToneFileResult } from "../toneFile";

const PATH = "C:/data/502nmos.fits";
const IDENTITY_LEVELS = { black: 0, gamma: 1, white: 1 };
const IDENTITY_CURVE = [{ x: 0, y: 0 }, { x: 1, y: 1 }];
const LEVELS = { black: 0.02, gamma: 1.5, white: 0.6 };
const CURVE = [{ x: 0, y: 0 }, { x: 0.5, y: 0.7 }, { x: 1, y: 1 }];

const RESULT: ToneFileResult = {
  png_path: "C:/out/502nmos_tone.png",
  fits_path: "C:/out/502nmos_tone.fits",
  previewUrl: "asset://C:/out/502nmos_tone.png",
  dimensions: [1600, 1600],
  is_rgb: false,
  levels_applied: true,
  curves_applied: true,
  input_normalized: null,
  elapsed_ms: 12,
};

function invokeArgs(): Record<string, unknown> {
  expect(withPreviewMock).toHaveBeenCalledTimes(1);
  const [cmd, dir, args] = withPreviewMock.mock.calls[0];
  expect(cmd).toBe("apply_tone_cmd");
  expect(dir).toBe("C:/out");
  return args as Record<string, unknown>;
}

describe("applyToneFile", () => {
  beforeEach(() => {
    withPreviewMock.mockReset();
    withPreviewMock.mockResolvedValue(RESULT);
  });

  it("omits identity levels and an identity curve from the invoke args", async () => {
    await applyToneFile(PATH, "C:/out", { levels: IDENTITY_LEVELS, curve: IDENTITY_CURVE });
    expect(invokeArgs()).toEqual({ path: PATH });
  });

  it("sends non-identity levels as given and the curve as { points: [[x, y], ...] }", async () => {
    await applyToneFile(PATH, "C:/out", { levels: LEVELS, curve: CURVE });
    expect(invokeArgs()).toEqual({ path: PATH, levels: LEVELS, curve: { points: [[0, 0], [0.5, 0.7], [1, 1]] } });
  });

  it("sends only the part that is not identity", async () => {
    await applyToneFile(PATH, "C:/out", { levels: IDENTITY_LEVELS, curve: CURVE });
    expect(invokeArgs()).toEqual({ path: PATH, curve: { points: [[0, 0], [0.5, 0.7], [1, 1]] } });
    withPreviewMock.mockClear();
    await applyToneFile(PATH, "C:/out", { levels: LEVELS });
    expect(invokeArgs()).toEqual({ path: PATH, levels: LEVELS });
  });

  it("returns the backend result with the output FITS path", async () => {
    await expect(applyToneFile(PATH, "C:/out", { curve: CURVE })).resolves.toMatchObject({ fits_path: "C:/out/502nmos_tone.fits", is_rgb: false });
  });
});

describe("isToneIdentity", () => {
  it("is true when every given part is identity or absent", () => {
    expect(isToneIdentity({})).toBe(true);
    expect(isToneIdentity({ levels: IDENTITY_LEVELS, curve: IDENTITY_CURVE })).toBe(true);
  });

  it("is false when the levels or the curve change anything", () => {
    expect(isToneIdentity({ levels: LEVELS, curve: IDENTITY_CURVE })).toBe(false);
    expect(isToneIdentity({ levels: IDENTITY_LEVELS, curve: CURVE })).toBe(false);
  });
});

describe("TONE_IDENTITY_MESSAGE", () => {
  it("is the backend's E-TONE-IDENTITY text verbatim", () => {
    expect(TONE_IDENTITY_MESSAGE).toBe("Nothing to apply: the levels and the curve are both identity.");
  });
});
