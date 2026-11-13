import { describe, expect, it } from "vitest";
import {
  WINDOW_NEEDS_BOTH,
  binFromDraft,
  clipFromDraft,
  draftFromBin,
  draftFromMargin,
  draftFromWindow,
  fapText,
  foldFromDraft,
  marginFromDraft,
  periodRangeFromDraft,
  periodStep,
  periodogramAliasWarning,
  periodogramRangeText,
  sameWindow,
  windowFromDraft,
} from "../lightCurveInputs";
import { formatSignificant } from "../formatSignificant";

describe("lightCurveInputs", () => {
  it("turns two typed axis values into an absolute window", () => {
    const typed = windowFromDraft({ start: "0.5068", end: "0.5086" }, 60061);
    expect(typed.apply).toBe(true);
    expect(typed.error).toBeNull();
    expect(typed.hint).toBeNull();
    expect(typed.window!.start).toBeCloseTo(60061.5068, 9);
    expect(typed.window!.end).toBeCloseTo(60061.5086, 9);
    expect(windowFromDraft({ start: "", end: "" }, 60061)).toEqual({ window: null, error: null, apply: true, hint: null });
    expect(windowFromDraft({ start: "0.5", end: "  " }, 60061)).toEqual({ window: null, error: null, apply: true, hint: WINDOW_NEEDS_BOTH });
    expect(windowFromDraft({ start: "", end: "0.5" }, 60061)).toEqual({ window: null, error: null, apply: true, hint: WINDOW_NEEDS_BOTH });
  });

  it("refuses a text that is not a number and a window that ends before it starts", () => {
    const comma = windowFromDraft({ start: "0,5", end: "1" }, 0);
    expect(comma.apply).toBe(false);
    expect(comma.window).toBeNull();
    expect(comma.error).toBe("transit start is not a number");
    const end = windowFromDraft({ start: "1", end: "x" }, 0);
    expect(end.apply).toBe(false);
    expect(end.error).toBe("transit end is not a number");
    const reversed = windowFromDraft({ start: "2", end: "1" }, 0);
    expect(reversed.apply).toBe(false);
    expect(reversed.window).toBeNull();
    expect(reversed.error).toBe("transit end is before transit start");
    expect(windowFromDraft({ start: "1", end: "1" }, 0)).toEqual({ window: { start: 1, end: 1 }, error: null, apply: true, hint: null });
  });

  it("writes a stored window back as axis text without float noise", () => {
    expect(draftFromWindow({ start: 60061.5068, end: 60061.5086 }, 60061)).toEqual({ start: "0.5068", end: "0.5086" });
    expect(draftFromWindow({ start: 2460310.61, end: 2460310.65 }, 2460310)).toEqual({ start: "0.61", end: "0.65" });
    expect(draftFromWindow(null, 60061)).toEqual({ start: "", end: "" });
    const w = { start: 60061.5068, end: 60061.5086 };
    expect(sameWindow(null, null)).toBe(true);
    expect(sameWindow(w, { start: w.start + 5e-10, end: w.end - 5e-10 })).toBe(true);
    expect(sameWindow(null, w)).toBe(false);
    expect(sameWindow(w, null)).toBe(false);
    expect(sameWindow(w, { start: w.start, end: w.end + 1e-6 })).toBe(false);
  });

  it("writes a typed edge back as the shortest text that reads as the same window", () => {
    for (const offset of [0, 60061, 2460310]) {
      const typed = windowFromDraft({ start: "0.5068", end: "0.508596144" }, offset).window!;
      const shown = draftFromWindow(typed, offset);
      expect(shown).toEqual({ start: "0.5068", end: "0.508596144" });
      expect(windowFromDraft(shown, offset).window).toEqual(typed);
      expect(windowFromDraft({ ...shown, start: "0.5067" }, offset).window!.end).toBe(typed.end);
      const long = windowFromDraft({ start: "0.5", end: "0.50859614412345678" }, offset).window!;
      const longShown = draftFromWindow(long, offset);
      expect(windowFromDraft(longShown, offset).window).toEqual(long);
      expect(longShown.end.length).toBeLessThanOrEqual("0.50859614412345678".length);
      expect(longShown.start).toBe("0.5");
    }
  });

  it("reads the margin in minutes, or in points on an index axis", () => {
    expect(marginFromDraft("", false)).toEqual({ marginDays: 0, error: null, apply: true });
    const minutes = marginFromDraft("7.2", false);
    expect(minutes.apply).toBe(true);
    expect(minutes.error).toBeNull();
    expect(minutes.marginDays).toBeCloseTo(0.005, 12);
    expect(marginFromDraft("3", true)).toEqual({ marginDays: 3, error: null, apply: true });
    const refused = { marginDays: 0, error: "margin is not a number of at least 0", apply: false };
    expect(marginFromDraft("-1", false)).toEqual(refused);
    expect(marginFromDraft("abc", false)).toEqual(refused);
    expect(draftFromMargin(0.005, false)).toBe("7.2");
    expect(draftFromMargin(0, false)).toBe("");
    expect(draftFromMargin(3, true)).toBe("3");
  });

  it("reads the bin kind and size", () => {
    expect(binFromDraft("none", "7")).toEqual({ bin: { kind: "none" }, error: null, apply: true });
    expect(binFromDraft("points", "10")).toEqual({ bin: { kind: "points", points: 10 }, error: null, apply: true });
    expect(binFromDraft("points", "")).toEqual({ bin: { kind: "points", points: 5 }, error: null, apply: true });
    const wholeError = { bin: { kind: "none" }, error: "bin size must be a whole number of at least 2", apply: false };
    expect(binFromDraft("points", "1")).toEqual(wholeError);
    expect(binFromDraft("points", "2.5")).toEqual(wholeError);
    expect(binFromDraft("points", "x")).toEqual(wholeError);
    expect(binFromDraft("minutes", "2.5")).toEqual({ bin: { kind: "minutes", minutes: 2.5 }, error: null, apply: true });
    expect(binFromDraft("minutes", "")).toEqual({ bin: { kind: "minutes", minutes: 5 }, error: null, apply: true });
    expect(binFromDraft("minutes", "0")).toEqual({
      bin: { kind: "none" },
      error: "bin size must be a number of minutes above 0",
      apply: false,
    });
    expect(draftFromBin({ kind: "minutes", minutes: 2.5 })).toEqual({ kind: "minutes", size: "2.5" });
    expect(draftFromBin({ kind: "points", points: 10 })).toEqual({ kind: "points", size: "10" });
    expect(draftFromBin({ kind: "none" })).toEqual({ kind: "none", size: "" });
  });

  it("reads the clip sigma and its running window", () => {
    expect(clipFromDraft("", "")).toEqual({ clipSigma: null, clipWindowPoints: 11, error: null, apply: true });
    expect(clipFromDraft("5", "7")).toEqual({ clipSigma: 5, clipWindowPoints: 7, error: null, apply: true });
    const sigmaRefused = clipFromDraft("0", "");
    expect(sigmaRefused.apply).toBe(false);
    expect(sigmaRefused.error).toBe("clip sigma must be a number above 0");
    const windowError = "clip window must be a whole number of at least 3";
    expect(clipFromDraft("5", "2").apply).toBe(false);
    expect(clipFromDraft("5", "2").error).toBe(windowError);
    expect(clipFromDraft("5", "7.5").apply).toBe(false);
    expect(clipFromDraft("5", "7.5").error).toBe(windowError);
    expect(clipFromDraft("0", "2").error).toBe("clip sigma must be a number above 0");
  });

  it("turns a period range into a frequency range", () => {
    expect(periodRangeFromDraft("", "")).toEqual({ minFrequency: null, maxFrequency: null, error: null });
    const both = periodRangeFromDraft("0.5", "10");
    expect(both.error).toBeNull();
    expect(both.maxFrequency).toBe(2);
    expect(both.minFrequency).toBeCloseTo(0.1, 15);
    expect(periodRangeFromDraft("2", "")).toEqual({ minFrequency: null, maxFrequency: 0.5, error: null });
    expect(periodRangeFromDraft("", "4")).toEqual({ minFrequency: 0.25, maxFrequency: null, error: null });
    const refused = (error: string) => ({ minFrequency: null, maxFrequency: null, error });
    expect(periodRangeFromDraft("0", "")).toEqual(refused("shortest period must be a number of days above 0"));
    expect(periodRangeFromDraft("x", "y")).toEqual(refused("shortest period must be a number of days above 0"));
    expect(periodRangeFromDraft("1", "x")).toEqual(refused("longest period must be a number of days above 0"));
    expect(periodRangeFromDraft("", "-3")).toEqual(refused("longest period must be a number of days above 0"));
    expect(periodRangeFromDraft("5", "5")).toEqual(refused("the shortest period must be below the longest"));
    expect(periodRangeFromDraft("6", "5")).toEqual(refused("the shortest period must be below the longest"));
  });

  it("reads the fold period and epoch", () => {
    expect(foldFromDraft("", "", 60061)).toEqual({ fold: null, error: null, apply: true });
    const typed = foldFromDraft("1.4079405", "0.5077", 60061);
    expect(typed.apply).toBe(true);
    expect(typed.error).toBeNull();
    expect(typed.fold!.periodDays).toBe(1.4079405);
    expect(typed.fold!.t0).toBeCloseTo(60061.5077, 9);
    expect(foldFromDraft("0", "0.5", 0)).toEqual({ fold: null, error: "fold period must be a number of days above 0", apply: false });
    expect(foldFromDraft("", "0.5", 0)).toEqual({ fold: null, error: "fold period must be a number of days above 0", apply: false });
    expect(foldFromDraft("2", "", 0)).toEqual({ fold: null, error: "the fold needs an epoch", apply: false });
    expect(foldFromDraft("2", "x", 0)).toEqual({ fold: null, error: "fold epoch is not a number", apply: false });
    expect(foldFromDraft("2", "-0.25", 10)).toEqual({ fold: { periodDays: 2, t0: 9.75 }, error: null, apply: true });
  });

  it("prints a false-alarm probability and the period step", () => {
    expect(fapText(6.64610294915559e-14)).toBe("6.6e-14");
    expect(fapText(0.3746095275854542)).toBe("0.37");
    expect(fapText(0.00012)).toBe("0.00012");
    expect(fapText(0.0000456)).toBe("4.6e-5");
    expect(fapText(0.0001)).toBe("0.0001");
    expect(fapText(0)).toBe("0");
    expect(fapText(null)).toBe("--");
    const step = periodStep(2.895261402175141, 0.013815678256184037);
    expect(step).toBeCloseTo(2.895261402175141 ** 2 * 0.013815678256184037, 15);
    expect(step).toBeCloseTo(0.11581045608700564, 15);
    expect(formatSignificant(step, 2)).toBe("0.12");
  });

  it("states the searched period range and why the automatic shortest period stops where it does", () => {
    const regular = {
      first_frequency: 0.5858166,
      last_frequency: 534.2647,
      upper_limit: "cadence_nyquist" as const,
      cadence_days: 0.0009353528221254237,
      best_frequency: 4.1,
    };
    expect(periodogramRangeText(regular)).toBe(
      "Periods searched: 0.001872 to 1.707 d. The shortest is twice the regular cadence of 0.0009354 d (Nyquist frequency 534.6 per day): shorter periods only repeat aliases of the sampling. Type a shortest period to search below it.",
    );
    expect(periodogramRangeText({ ...regular, upper_limit: "five_mean_nyquist", cadence_days: null, last_frequency: 6.907839128092019 })).toBe(
      "Periods searched: 0.1448 to 1.707 d. The shortest is set by five times the mean Nyquist frequency (points / 2 baseline).",
    );
    expect(periodogramRangeText({ ...regular, upper_limit: "given", first_frequency: 0.1, last_frequency: 2 })).toBe("Periods searched: 0.5 to 10 d.");
    expect(periodogramAliasWarning(regular)).toBeNull();
    expect(periodogramAliasWarning({ ...regular, upper_limit: "given", last_frequency: 2680, best_frequency: 2134.13 })).toBe(
      "The highest peak lies above 534.6 per day, the Nyquist frequency of the regular cadence: it can be an alias of the sampling, not a period of the source.",
    );
    expect(periodogramAliasWarning({ ...regular, cadence_days: null, best_frequency: 2134.13 })).toBeNull();
  });
});
