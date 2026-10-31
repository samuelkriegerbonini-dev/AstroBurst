import { describe, expect, it } from "vitest";
import {
  DEFAULT_SUBFRAME_SETTINGS,
  DEFAULT_SUBFRAME_THRESHOLDS,
  SUBFRAME_STALE_NOTE,
  analysedThresholdsOf,
  subframeFilesChanged,
  subframeResultStale,
  subframeSettingsChanged,
  subframeStaleNote,
} from "../subframeSettings";

describe("subframeFilesChanged", () => {
  it("sees a frame added, removed or swapped since the analysis, whatever the order", () => {
    expect(subframeFilesChanged(["C:/s/a.fits", "C:/s/b.fits"], ["C:/s/b.fits", "C:/s/a.fits"])).toBe(false);
    expect(subframeFilesChanged(["C:/s/a.fits", "C:/s/b.fits"], ["C:/s/a.fits", "C:/s/b.fits", "C:/s/c.fits"])).toBe(true);
    expect(subframeFilesChanged(["C:/s/a.fits", "C:/s/b.fits"], ["C:/s/a.fits"])).toBe(true);
    expect(subframeFilesChanged(["C:/s/a.fits", "C:/s/b.fits"], ["C:/s/a.fits", "C:/s/c.fits"])).toBe(true);
  });
});

describe("subframeStaleNote", () => {
  it("names what changed since the analysis", () => {
    expect(subframeStaleNote(false, false)).toBeNull();
    expect(subframeStaleNote(true, false)).toBe(SUBFRAME_STALE_NOTE);
    expect(subframeStaleNote(false, true)).toBe("Frames changed since the analysis: Analyze again");
    expect(subframeStaleNote(true, true)).toBe("Frames and settings changed since the analysis: Analyze again");
  });
});

describe("subframeSettingsChanged", () => {
  const analysed = { ...DEFAULT_SUBFRAME_SETTINGS };

  it("is false while every threshold and weight equals the analysed one", () => {
    expect(subframeSettingsChanged(analysed, { ...analysed })).toBe(false);
  });

  it.each([
    ["maxFwhm", 5.0],
    ["maxEccentricity", 0.75],
    ["minSnr", 6],
    ["minStars", 6],
    ["fwhmWeight", 1.1],
    ["eccentricityWeight", 0.6],
    ["snrWeight", 0.9],
    ["noiseWeight", 0.4],
  ] as const)("is true after %s changes", (key, value) => {
    expect(subframeSettingsChanged(analysed, { ...analysed, [key]: value })).toBe(true);
  });

  it("compares the thresholds alone for the wizard, which sends no weights", () => {
    expect(subframeSettingsChanged(DEFAULT_SUBFRAME_THRESHOLDS, { ...DEFAULT_SUBFRAME_THRESHOLDS })).toBe(false);
    expect(subframeSettingsChanged(DEFAULT_SUBFRAME_THRESHOLDS, { ...DEFAULT_SUBFRAME_THRESHOLDS, minStars: 3 })).toBe(true);
  });
});

describe("subframe defaults", () => {
  it("match SubframeWeightConfig::default in core/analysis/subframe.rs", () => {
    expect(DEFAULT_SUBFRAME_SETTINGS).toEqual({
      maxFwhm: 8.0,
      maxEccentricity: 0.7,
      minSnr: 5.0,
      minStars: 5,
      fwhmWeight: 1.0,
      eccentricityWeight: 0.5,
      snrWeight: 1.0,
      noiseWeight: 0.3,
    });
    expect(DEFAULT_SUBFRAME_THRESHOLDS).toEqual({ maxFwhm: 8.0, maxEccentricity: 0.7, minSnr: 5.0, minStars: 5 });
  });
});

describe("wizard subframe results", () => {
  const bins = [{ id: "ha" }, { id: "r" }, { id: "g" }];
  const at = (maxFwhm: number) => ({ ...DEFAULT_SUBFRAME_THRESHOLDS, maxFwhm });

  it("start the sliders from the thresholds of the first analysed channel", () => {
    expect(analysedThresholdsOf(bins, { g: { analysedWith: at(5) }, r: { analysedWith: at(6.5) } })).toEqual(at(6.5));
  });

  it("have no thresholds to start from before an analysis that recorded them", () => {
    expect(analysedThresholdsOf(bins, {})).toBeNull();
    expect(analysedThresholdsOf(bins, { r: {} })).toBeNull();
  });

  it("are stale once the sliders differ from the thresholds they were analysed with", () => {
    expect(subframeResultStale({ analysedWith: at(6.5) }, at(6.5))).toBe(false);
    expect(subframeResultStale({ analysedWith: at(6.5) }, at(5))).toBe(true);
    expect(subframeResultStale({}, at(5))).toBe(false);
  });

  it("name the way out", () => {
    expect(SUBFRAME_STALE_NOTE).toBe("Settings changed since the analysis: Analyze again");
  });
});
