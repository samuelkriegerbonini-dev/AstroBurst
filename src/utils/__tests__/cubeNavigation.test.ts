import { describe, it, expect, vi } from "vitest";
import {
  FRAME_STEP_LARGE,
  PLAYBACK_SPEEDS,
  channelFromPlotPixel,
  channelRequestNeeded,
  clampFrame,
  createFramePublishGate,
  cubeFrameOutputPaths,
  displayedChannel,
  drainFrameRequest,
  formatFrameDelta,
  formatFrameLabel,
  frameAxisValue,
  frameFromKey,
  frameLoadPlan,
  frameRecordLabel,
  isPlaybackSpeed,
  mergeFrameRequest,
  nextPlaybackFrame,
  playbackIntervalMs,
  playbackStartFrame,
  stepFrame,
} from "../cubeNavigation";
import { formatAxis } from "../spectralAxis";
import { axisValueToPixel, type PlotMapping } from "../spectrumRange";
import type { ProcessedResult } from "../../shared/types/preview";
import type { SpectralAxisInfo } from "../../shared/types/spectral";

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
  it("numbers the displayed channel from zero, like the spectrum marker, the z0/z1 inputs and the FITS CHANNEL card", () => {
    const frame = 17;
    const label = formatFrameLabel(frame, 40, WAVE[frame], "um");
    expect(label).toBe("ch 17 (0–39) - 1.0170 um");
    expect(label.startsWith(`ch ${frame} `)).toBe(true);
    expect(formatFrameLabel(2, 40, 1.002, "um")).toBe("ch 2 (0–39) - 1.0020 um");
    expect(formatFrameLabel(9, 40, -12.34, "km/s")).toBe("ch 9 (0–39) - -12.3 km/s");
  });

  it("drops the axis part when no spectral value is known", () => {
    expect(formatFrameLabel(0, 40, null, "um")).toBe("ch 0 (0–39)");
    expect(formatFrameLabel(0, 40, 1.0, "ch")).toBe("ch 0 (0–39)");
    expect(formatFrameLabel(0, 40, NaN, "um")).toBe("ch 0 (0–39)");
    expect(formatFrameLabel(0, 0, null, "")).toBe("ch 0 (0–0)");
  });
});

describe("frameRecordLabel", () => {
  const museAirAxis: SpectralAxisInfo = {
    kind: "awav",
    ctype: "AWAV",
    unit: "um",
    header_unit: "Angstrom",
    header_scale: 1e-4,
    values: Array.from({ length: 3681 }, (_, i) => (4750 + i * 1.25) * 1e-4),
    crval: 0.475,
    cdelt: 1.25e-4,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
  };

  it("labels an air-wavelength cube channel with the vacuum wavelength the navigator shows", () => {
    expect(frameRecordLabel(1450, 3681, museAirAxis)).toBe("ch 1450 (0–3680) - 0.6564 um");
    const nav = formatAxis(museAirAxis, "wavelength_vac", null, "optical");
    for (const idx of [0, 1450, 3000, 3680]) {
      const navLabel = formatFrameLabel(idx, 3681, frameAxisValue(idx, nav?.values), nav?.unit ?? "");
      expect(frameRecordLabel(idx, 3681, museAirAxis).split(" ")[4]).toBe(navLabel.split(" ")[4]);
    }
  });

  it("keeps a frequency cube channel in GHz and drops the axis of an unknown one", () => {
    const freq: SpectralAxisInfo = { ...museAirAxis, kind: "freq", ctype: "FREQ", unit: "GHz", values: [230.5] };
    expect(frameRecordLabel(0, 1, freq)).toBe("ch 0 (0–0) - 230.5000 GHz");
    expect(frameRecordLabel(3, 40, { ...museAirAxis, kind: "unknown" })).toBe("ch 3 (0–39)");
    expect(frameRecordLabel(3, 40, null)).toBe("ch 3 (0–39)");
  });
});

describe("displayedChannel", () => {
  const record: ProcessedResult = {
    fitsPath: "./output/cube_frame_x_12.fits",
    previewUrl: "u",
    dimensions: [8, 8],
    label: "ch 12 (0–39)",
    kind: "cube",
    inputPath: "/data/cube.fits",
  };

  it("reads the published channel back from the render record so a remounted panel marks channel 12, not 0", () => {
    expect(displayedChannel({ ...record, frameIndex: 12 }, "/data/cube.fits")).toBe(12);
  });

  it("treats a reset preview, which shows the original cube plane 0, as channel 0", () => {
    expect(displayedChannel(null, "/data/cube.fits")).toBe(0);
  });

  it("reports no displayed channel for a collapse, another file, a processing result or a malformed index", () => {
    const collapse: ProcessedResult = { ...record, label: "Collapse mean" };
    expect(displayedChannel(collapse, "/data/cube.fits")).toBeNull();
    expect(displayedChannel({ ...record, frameIndex: 12 }, "/data/other.fits")).toBeNull();
    expect(displayedChannel({ ...record, kind: "processing", frameIndex: 12 }, "/data/cube.fits")).toBeNull();
    expect(displayedChannel({ ...record, frameIndex: -1 }, "/data/cube.fits")).toBeNull();
    expect(displayedChannel({ ...record, frameIndex: 2.5 }, "/data/cube.fits")).toBeNull();
    expect(displayedChannel(null, undefined)).toBeNull();
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

describe("channelRequestNeeded", () => {
  it("re-shows the marked channel after a Reset or a collapse even though the cursor already sits on it", () => {
    expect(channelRequestNeeded(17, 17, 0, false)).toBe(true);
    expect(channelRequestNeeded(17, 17, null, false)).toBe(true);
  });

  it("upgrades a PNG-only record of the same channel and loads any other channel", () => {
    expect(channelRequestNeeded(17, 17, 17, true)).toBe(true);
    expect(channelRequestNeeded(18, 17, 17, false)).toBe(true);
  });

  it("does nothing when the channel is already on screen with its FITS, as when ArrowLeft is held at channel 0", () => {
    expect(channelRequestNeeded(17, 17, 17, false)).toBe(false);
    expect(channelRequestNeeded(0, 0, 0, false)).toBe(false);
  });
});

describe("drainFrameRequest", () => {
  it("drops a queued request for a frame the user has already left for a cached one", () => {
    expect(drainFrameRequest({ idx: 5, withFits: false }, 4)).toBeNull();
    expect(drainFrameRequest({ idx: 5, withFits: true }, 4)).toBeNull();
  });

  it("keeps a queued request for the displayed frame with its FITS flag", () => {
    expect(drainFrameRequest({ idx: 4, withFits: true }, 4)).toEqual({ idx: 4, withFits: true });
    expect(drainFrameRequest({ idx: 4, withFits: false }, 4)).toEqual({ idx: 4, withFits: false });
    expect(drainFrameRequest(null, 4)).toBeNull();
  });
});

describe("frameLoadPlan", () => {
  it("publishes a cached FITS immediately on a committed step instead of a PNG-only record", () => {
    expect(frameLoadPlan(true, true)).toEqual({ withFits: true, deferFits: false });
  });

  it("defers the FITS of an unvisited channel on a committed step", () => {
    expect(frameLoadPlan(true, false)).toEqual({ withFits: false, deferFits: true });
  });

  it("keeps transient steps PNG-only even when the FITS is cached", () => {
    expect(frameLoadPlan(false, true)).toEqual({ withFits: false, deferFits: false });
    expect(frameLoadPlan(false, false)).toEqual({ withFits: false, deferFits: false });
  });
});

describe("createFramePublishGate", () => {
  it("drops the deferred channel FITS publish when a moment map is shown or the preview is reset after the commit", () => {
    vi.useFakeTimers();
    try {
      const gate = createFramePublishGate();
      const shown: string[] = [];
      gate.commit();
      setTimeout(() => {
        if (gate.isCurrent()) shown.push("channel 13 fits");
      }, 300);
      vi.advanceTimersByTime(150);
      gate.supersede();
      shown.push("moment m1");
      vi.advanceTimersByTime(300);
      expect(shown).toEqual(["moment m1"]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("publishes again for a channel committed after the cube result", () => {
    const gate = createFramePublishGate();
    expect(gate.isCurrent()).toBe(true);
    gate.commit();
    gate.supersede();
    gate.supersede();
    expect(gate.isCurrent()).toBe(false);
    gate.commit();
    expect(gate.isCurrent()).toBe(true);
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
