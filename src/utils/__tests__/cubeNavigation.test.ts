import { describe, it, expect } from "vitest";
import {
  FRAME_STEP_LARGE,
  PLAYBACK_SPEEDS,
  channelFromPlotPixel,
  clampFrame,
  cubeFrameOutputPaths,
  formatFrameDelta,
  formatFrameLabel,
  frameAxisValue,
  frameFromKey,
  isPlaybackSpeed,
  mergeFrameRequest,
  nextPlaybackFrame,
  playbackIntervalMs,
  playbackStartFrame,
  stepFrame,
} from "../cubeNavigation";
import { axisValueToPixel, type PlotMapping } from "../spectrumRange";

const WAVE = Array.from({ length: 40 }, (_, i) => 1.0 + i * 0.001);

function mapping(xValues: number[] | null, n = 40): PlotMapping {
  const values = xValues ?? Array.from({ length: n }, (_, i) => i);
  return {
    width: 400,
    padLeft: 50,
    padRight: 12,
    xMin: Math.min(...values),
    xMax: Math.max(...values),
    xValues,
    n,
  };
}

describe("clampFrame", () => {
  it("keeps a frame inside [0, total - 1] and rounds fractional indices", () => {
    expect(clampFrame(-3, 10)).toBe(0);
    expect(clampFrame(12, 10)).toBe(9);
    expect(clampFrame(4.6, 10)).toBe(5);
    expect(clampFrame(4, 10)).toBe(4);
  });

  it("falls back to frame 0 for an empty cube or a non-finite index", () => {
    expect(clampFrame(5, 0)).toBe(0);
    expect(clampFrame(NaN, 10)).toBe(0);
    expect(clampFrame(3, NaN)).toBe(0);
  });
});

describe("stepFrame and frameFromKey", () => {
  it("steps by one, or by ten with shift, and stops at both ends", () => {
    expect(stepFrame(5, 1, 40, false)).toBe(6);
    expect(stepFrame(5, -1, 40, false)).toBe(4);
    expect(stepFrame(5, 1, 40, true)).toBe(5 + FRAME_STEP_LARGE);
    expect(stepFrame(5, -1, 40, true)).toBe(0);
    expect(stepFrame(39, 1, 40, false)).toBe(39);
    expect(stepFrame(35, 1, 40, true)).toBe(39);
  });

  it("maps only the arrow keys and ignores keys on a single-frame image", () => {
    expect(frameFromKey("ArrowRight", false, 3, 40)).toBe(4);
    expect(frameFromKey("ArrowLeft", true, 30, 40)).toBe(20);
    expect(frameFromKey("ArrowUp", false, 3, 40)).toBeNull();
    expect(frameFromKey("a", false, 3, 40)).toBeNull();
    expect(frameFromKey("ArrowRight", false, 0, 1)).toBeNull();
  });
});

describe("channelFromPlotPixel", () => {
  it("resolves a double-click to the nearest channel on a wavelength axis", () => {
    const m = mapping(WAVE);
    expect(channelFromPlotPixel(axisValueToPixel(WAVE[17], m), m, 40)).toBe(17);
    expect(channelFromPlotPixel(axisValueToPixel(WAVE[17] + 0.0004, m), m, 40)).toBe(17);
    expect(channelFromPlotPixel(axisValueToPixel(WAVE[17] + 0.0006, m), m, 40)).toBe(18);
  });

  it("clamps to the plot ends and to the cube's frame count", () => {
    const m = mapping(null);
    expect(channelFromPlotPixel(-100, m, 40)).toBe(0);
    expect(channelFromPlotPixel(10_000, m, 40)).toBe(39);
    expect(channelFromPlotPixel(axisValueToPixel(30, m), m, 20)).toBe(19);
  });

  it("returns null for a non-finite pixel or an empty cube", () => {
    const m = mapping(null);
    expect(channelFromPlotPixel(NaN, m, 40)).toBeNull();
    expect(channelFromPlotPixel(100, m, 0)).toBeNull();
    expect(channelFromPlotPixel(100, mapping(null, 0), 40)).toBeNull();
  });
});

describe("frameAxisValue", () => {
  it("reads the axis value of a frame and rejects out-of-range or non-finite entries", () => {
    expect(frameAxisValue(3, WAVE)).toBeCloseTo(1.003, 9);
    expect(frameAxisValue(40, WAVE)).toBeNull();
    expect(frameAxisValue(-1, WAVE)).toBeNull();
    expect(frameAxisValue(0, [NaN])).toBeNull();
    expect(frameAxisValue(0, null)).toBeNull();
  });
});

describe("formatFrameLabel", () => {
  it("names the one-based channel and the axis value with its unit", () => {
    expect(formatFrameLabel(2, 40, 1.002, "um")).toBe("Channel 3/40 - 1.0020 um");
    expect(formatFrameLabel(9, 40, -12.34, "km/s")).toBe("Channel 10/40 - -12.3 km/s");
  });

  it("drops the axis part when no spectral value is known", () => {
    expect(formatFrameLabel(0, 40, null, "um")).toBe("Channel 1/40");
    expect(formatFrameLabel(0, 40, 1.0, "ch")).toBe("Channel 1/40");
    expect(formatFrameLabel(0, 40, NaN, "um")).toBe("Channel 1/40");
  });
});

describe("formatFrameDelta", () => {
  it("shows the signed channel offset from the displayed frame", () => {
    expect(formatFrameDelta(34, 30)).toBe("+4 from displayed");
    expect(formatFrameDelta(28, 30)).toBe("-2 from displayed");
    expect(formatFrameDelta(30, 30)).toBe("displayed");
  });
});

describe("playback", () => {
  it("advances by one and stops at the last frame unless looping", () => {
    expect(nextPlaybackFrame(3, 40, false)).toBe(4);
    expect(nextPlaybackFrame(39, 40, false)).toBeNull();
    expect(nextPlaybackFrame(39, 40, true)).toBe(0);
    expect(nextPlaybackFrame(0, 1, true)).toBeNull();
  });

  it("starts from the first frame when play is pressed on the last one", () => {
    expect(playbackStartFrame(39, 40)).toBe(0);
    expect(playbackStartFrame(4, 40)).toBe(5);
  });

  it("exposes fast, normal and slow intervals", () => {
    expect(PLAYBACK_SPEEDS.map((s) => s.id)).toEqual(["fast", "normal", "slow"]);
    expect(playbackIntervalMs("fast")).toBe(50);
    expect(playbackIntervalMs("normal")).toBe(150);
    expect(playbackIntervalMs("slow")).toBe(400);
    expect(isPlaybackSpeed("slow")).toBe(true);
    expect(isPlaybackSpeed("warp")).toBe(false);
  });
});

describe("mergeFrameRequest", () => {
  it("keeps the FITS request when a transient load is queued for the same frame", () => {
    expect(mergeFrameRequest({ idx: 5, withFits: true }, { idx: 5, withFits: false })).toEqual({ idx: 5, withFits: true });
    expect(mergeFrameRequest({ idx: 5, withFits: false }, { idx: 5, withFits: true })).toEqual({ idx: 5, withFits: true });
  });

  it("lets the latest frame replace a pending request for another frame", () => {
    expect(mergeFrameRequest({ idx: 5, withFits: true }, { idx: 7, withFits: false })).toEqual({ idx: 7, withFits: false });
    expect(mergeFrameRequest(null, { idx: 7, withFits: true })).toEqual({ idx: 7, withFits: true });
  });
});

describe("cubeFrameOutputPaths", () => {
  it("derives stable png and fits names from the cube path and frame index", () => {
    const a = cubeFrameOutputPaths("/data/cube.fits", 3);
    const b = cubeFrameOutputPaths("/data/cube.fits", 3);
    const other = cubeFrameOutputPaths("/data/other.fits", 3);
    expect(a).toEqual(b);
    expect(a.png).toMatch(/^\.\/output\/cube_frame_[0-9a-z]+_3\.png$/);
    expect(a.fits).toBe(a.png.replace(/\.png$/, ".fits"));
    expect(other.png).not.toBe(a.png);
    expect(cubeFrameOutputPaths("/data/cube.fits", 4).png).not.toBe(a.png);
  });
});
