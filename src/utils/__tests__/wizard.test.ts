import { describe, it, expect } from "vitest";
import {
  applyCompositeOp,
  channelExportHistory,
  channelOutputPaths,
  channelStretchInput,
  compositeHistoryLines,
  droppedChannelOutputs,
  EMPTY_COMPOSITE_HISTORY,
  INITIAL_STATE,
  invalidateDownstream,
  resolveExportRgbPaths,
  resolveOutputChannelPath,
  resolveRgbPaths,
  singleChannelBinId,
  starRemovalNote,
  withChannelStage,
  wizardHeaderSourcePath,
  wizardStackName,
  wizardZipChannels,
  type CompositeHistory,
  type WizardState,
} from "../wizard";

function stateWith(files: Record<string, string[]>, extra: Partial<WizardState> = {}): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] })),
    ...extra,
  };
}

const starless = { path: "/out/ha_starless.fits", note: starRemovalNote(4, 3) };
const stretched = { path: "/out/ha_starless_masked_stretch.fits", note: "masked stretch (target bg 0.25)" };

describe("single-channel wizard results", () => {
  it("targets the first filled bin", () => {
    expect(singleChannelBinId(stateWith({ oiii: ["/o.fits"], ha: ["/h.fits"] }))).toBe("ha");
    expect(singleChannelBinId(stateWith({}))).toBeNull();
  });

  it("feeds the starless channel to the next stretch instead of the channel with stars", () => {
    const base = stateWith({ ha: ["/h.fits"] });
    expect(channelStretchInput(base, "ha")).toBe("/h.fits");
    const withStarless = { ...base, channelResults: withChannelStage({}, "ha", "starless", starless) };
    expect(channelStretchInput(withStarless, "ha")).toBe(starless.path);
  });

  it("keeps the starless stage when a stretch lands, and drops the stretch when stars are removed again", () => {
    const afterStretch = withChannelStage(withChannelStage({}, "ha", "starless", starless), "ha", "stretched", stretched);
    expect(afterStretch.ha).toEqual({ starless, stretched });
    const rerun = withChannelStage(afterStretch, "ha", "starless", { path: "/out/ha_starless2.fits", note: "x" });
    expect(rerun.ha.stretched).toBeNull();
  });

  it("exports the processed channel in every slot it fills, never mixing it with the unprocessed channel", () => {
    const s = stateWith({ ha: ["/h.fits"] }, {
      channelResults: withChannelStage(withChannelStage({}, "ha", "starless", starless), "ha", "stretched", stretched),
    });
    expect(resolveOutputChannelPath(s, "ha")).toBe(stretched.path);
    expect(resolveRgbPaths(s, true)).toEqual({ r: stretched.path, g: stretched.path, b: stretched.path });
    expect(resolveRgbPaths(s)).toEqual({ r: "/h.fits", g: "/h.fits", b: "/h.fits" });
  });

  it("exports only the stretched channel when the other filled bins are still linear", () => {
    const hoo = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }, {
      channelResults: withChannelStage({}, "ha", "stretched", stretched),
    });
    expect(resolveRgbPaths(hoo, true)).toEqual({ r: stretched.path, g: "/o.fits", b: "/o.fits" });
    const out = resolveExportRgbPaths(hoo);
    expect(out).toEqual({ r: stretched.path, g: stretched.path, b: stretched.path, monoBinId: "ha" });
    expect(channelExportHistory(hoo, [out.r, out.g, out.b])).toEqual([`Channel ha: ${stretched.note}`]);
  });

  it("still mixes a starless channel with linear channels, since both are linear", () => {
    const hoo = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }, {
      channelResults: withChannelStage({}, "ha", "starless", starless),
    });
    expect(resolveExportRgbPaths(hoo)).toEqual({ r: starless.path, g: "/o.fits", b: "/o.fits", monoBinId: null });
  });

  it("keeps the channel mapping when nothing is stretched or the only filled bin is stretched", () => {
    const linear = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"], sii: ["/s.fits"] });
    expect(resolveExportRgbPaths(linear)).toEqual({ r: "/s.fits", g: "/h.fits", b: "/o.fits", monoBinId: null });
    const single = stateWith({ ha: ["/h.fits"] }, {
      channelResults: withChannelStage({}, "ha", "stretched", stretched),
    });
    expect(resolveExportRgbPaths(single)).toEqual({ r: stretched.path, g: stretched.path, b: stretched.path, monoBinId: null });
  });

  it("exports the processed Hα alone when R, G and B take every colour slot (HaRGB, HaLRGB)", () => {
    const extras: Record<string, string[]>[] = [{}, { l: ["/l.fits"] }];
    for (const extra of extras) {
      const s = stateWith({ ha: ["/h.fits"], r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"], ...extra }, {
        channelResults: withChannelStage({}, "ha", "stretched", stretched),
      });
      expect(singleChannelBinId(s)).toBe("ha");
      const out = resolveExportRgbPaths(s);
      expect(out).toEqual({ r: stretched.path, g: stretched.path, b: stretched.path, monoBinId: "ha" });
      expect(channelExportHistory(s, [out.r, out.g, out.b])).toEqual([`Channel ha: ${stretched.note}`]);
      expect(wizardZipChannels(s)).toEqual([{ name: "channel_ha", path: stretched.path }]);
    }
  });

  it("exports a starless channel that has no colour slot alone instead of dropping it", () => {
    const s = stateWith({ ha: ["/h.fits"], r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] }, {
      channelResults: withChannelStage({}, "ha", "starless", starless),
    });
    expect(resolveExportRgbPaths(s)).toEqual({ r: starless.path, g: starless.path, b: starless.path, monoBinId: "ha" });
  });

  it("exports the single-channel result for every combination of filled bins", () => {
    const ids = INITIAL_STATE.bins.map((b) => b.id);
    for (let mask = 1; mask < 1 << ids.length; mask++) {
      const base = stateWith(Object.fromEntries(ids.filter((_, i) => mask & (1 << i)).map((id) => [id, [`/${id}.fits`]])));
      const target = singleChannelBinId(base);
      expect(target).not.toBeNull();
      const withStretch = resolveExportRgbPaths({ ...base, channelResults: withChannelStage({}, target!, "stretched", stretched) });
      expect([withStretch.r, withStretch.g, withStretch.b]).toEqual([stretched.path, stretched.path, stretched.path]);
      const withStarless = resolveExportRgbPaths({ ...base, channelResults: withChannelStage({}, target!, "starless", starless) });
      const slots = [withStarless.r, withStarless.g, withStarless.b];
      expect(slots).toContain(starless.path);
      expect(slots).not.toContain(`/${target}.fits`);
    }
  });

  it("zips one PNG per colour slot unless the export is a single processed channel", () => {
    const linear = stateWith({ ha: ["/h.fits"], r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] });
    expect(wizardZipChannels(linear)).toEqual([
      { name: "channel_r", path: "/r.fits" },
      { name: "channel_g", path: "/g.fits" },
      { name: "channel_b", path: "/b.fits" },
    ]);
    const blended = { ...linear, compositeReady: true, channelResults: withChannelStage({}, "ha", "stretched", stretched) };
    expect(wizardZipChannels(blended).map((c) => c.path)).toEqual(["/r.fits", "/g.fits", "/b.fits"]);
  });

  it("writes HISTORY only for the channels whose processed output is exported", () => {
    const s = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }, {
      channelResults: withChannelStage({}, "ha", "starless", starless),
    });
    expect(channelExportHistory(s, [starless.path, "/o.fits", "/o.fits"])).toEqual([`Channel ha: ${starless.note}`]);
    expect(channelExportHistory(s, ["/h.fits", "/o.fits", "/o.fits"])).toEqual([]);
    expect(channelOutputPaths(s)).toEqual([starless.path]);
  });

  it("forgets channel results and the composite history when an upstream step changes", () => {
    const s = stateWith({ ha: ["/h.fits"] }, {
      channelResults: withChannelStage({}, "ha", "starless", starless),
      compositeHistory: applyCompositeOp(EMPTY_COMPOSITE_HISTORY, { kind: "blend", preset: "sho" }),
      compositeReady: true,
    });
    const partial = invalidateDownstream(s, "stack");
    expect(partial.channelResults).toEqual({});
    expect(partial.compositeHistory).toEqual(EMPTY_COMPOSITE_HISTORY);
    expect(partial.compositeReady).toBe(false);
  });

  it("names the outputs an upstream change or a rerun drops, so their display records can be forgotten", () => {
    const s = stateWith({ ha: ["/h.fits"] }, {
      channelResults: withChannelStage(withChannelStage({}, "ha", "starless", starless), "ha", "stretched", stretched),
    });
    expect(droppedChannelOutputs(s, { ...s, ...invalidateDownstream(s, "stack") })).toEqual([starless.path, stretched.path]);
    const rerun = { ...s, channelResults: withChannelStage(s.channelResults, "ha", "starless", starless) };
    expect(droppedChannelOutputs(s, rerun)).toEqual([stretched.path]);
    expect(droppedChannelOutputs(s, s)).toEqual([]);
  });
});

describe("composite HISTORY", () => {
  const blended = applyCompositeOp(EMPTY_COMPOSITE_HISTORY, { kind: "blend", preset: "sho" });

  it("records the blend and nothing that was only a wizard setting", () => {
    expect(compositeHistoryLines(blended)).toEqual(["Blend: sho"]);
    expect(compositeHistoryLines(blended).some((l) => l.startsWith("Stretch"))).toBe(false);
  });

  it("records white balance and SCNR only when they were applied, and omits neutral factors", () => {
    const neutral = applyCompositeOp(blended, { kind: "colorBalance", mode: "none", r: 1, g: 1, b: 1, scnr: null });
    expect(compositeHistoryLines(neutral)).toEqual(["Blend: sho"]);
    const applied = applyCompositeOp(blended, {
      kind: "colorBalance", mode: "manual", r: 1.2, g: 1, b: 0.9, scnr: { method: "average", amount: 0.8 },
    });
    expect(compositeHistoryLines(applied)).toEqual([
      "Blend: sho",
      "White balance: manual R=1.200 G=1.000 B=0.900",
      "SCNR: average 80%",
    ]);
    expect(compositeHistoryLines(applyCompositeOp(applied, { kind: "resetColorBalance" }))).toEqual(["Blend: sho"]);
  });

  it("keeps LRGB and star removal when colour balance is applied or reset, as the backend keeps them in the data", () => {
    let h: CompositeHistory = applyCompositeOp(blended, { kind: "lrgb", lightness: 1, chrominance: 0.5 });
    h = applyCompositeOp(h, { kind: "starRemoval", sigma: 4, growth: 3 });
    const after = ["LRGB: lightness 100%, chrominance 50%", "Star removal: 4.0 sigma, growth 3.00x FWHM"];
    expect(compositeHistoryLines(h)).toEqual(["Blend: sho", ...after]);
    const balanced = applyCompositeOp(h, { kind: "colorBalance", mode: "auto", r: 1.1, g: 1, b: 1, scnr: null });
    expect(compositeHistoryLines(balanced)).toEqual(["Blend: sho", "White balance: auto R=1.100 G=1.000 B=1.000", ...after]);
    expect(compositeHistoryLines(applyCompositeOp(balanced, { kind: "resetColorBalance" }))).toEqual(["Blend: sho", ...after]);
  });

  it("starts over on a new blend", () => {
    const h = applyCompositeOp(blended, { kind: "lrgb", lightness: 1, chrominance: 1 });
    expect(compositeHistoryLines(applyCompositeOp(h, { kind: "blend", preset: "hoo" }))).toEqual(["Blend: hoo"]);
  });
});

describe("wizardStackName", () => {
  it("gives stack and drizzle, and every rerun, a different output name", () => {
    const names = [
      wizardStackName("ha", false, 1),
      wizardStackName("ha", true, 1),
      wizardStackName("ha", false, 2),
      wizardStackName("oiii", false, 1),
    ];
    expect(new Set(names).size).toBe(4);
    expect(names).not.toContain("stacked_ha");
  });
});

describe("wizardHeaderSourcePath", () => {
  const aligned = { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" };

  it("reads the WCS from the alignment reference, the first filled bin, when the channels are cache keys", () => {
    const s = stateWith({ ha: ["/raw/ha_1.fits", "/raw/ha_2.fits"], oiii: ["/raw/o_1.fits"] }, { alignedPaths: aligned });
    expect(wizardHeaderSourcePath(s, [])).toBe("/raw/ha_1.fits");
  });

  it("prefers the stacked FITS of the reference bin, whose header describes the stacked grid", () => {
    const s = stateWith(
      { ha: ["/raw/ha_1.fits", "/raw/ha_2.fits"], oiii: ["/raw/o_1.fits"] },
      { alignedPaths: aligned, stackedPaths: { ha: "/out/ha_stack2.fits" } },
    );
    expect(wizardHeaderSourcePath(s, [])).toBe("/out/ha_stack2.fits");
  });

  it("gives no header source after a crop, since the source WCS no longer matches the grid", () => {
    const s = stateWith({ ha: ["/raw/ha_1.fits"], oiii: ["/raw/o_1.fits"] }, {
      alignedPaths: aligned,
      croppedPaths: { ha: "__wizard_ch_ha_cropped", oiii: "__wizard_ch_oiii_cropped" },
    });
    expect(wizardHeaderSourcePath(s, [])).toBeNull();
  });

  it("gives no header source for unaligned channels of several bins, which Blend resamples to a common grid", () => {
    expect(wizardHeaderSourcePath(stateWith({ ha: ["/raw/ha_1.fits"], oiii: ["/raw/o_1.fits"] }), [])).toBeNull();
  });

  it("uses the processed bin of a single-channel export", () => {
    const s = stateWith({ ha: ["/raw/ha_1.fits"], oiii: ["/raw/o_1.fits"] });
    expect(wizardHeaderSourcePath(s, ["/out/ha_bg.fits"], "ha")).toBe("/raw/ha_1.fits");
  });

  it("keeps the channel's own header for a stretched channel, whose values no longer carry the source calibration", () => {
    const s = stateWith({ ha: ["/raw/ha_1.fits"] }, { channelResults: { ha: { starless: null, stretched } } });
    expect(wizardHeaderSourcePath(s, [stretched.path, stretched.path, stretched.path], "ha")).toBeNull();
  });
});
