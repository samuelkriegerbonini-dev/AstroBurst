import { describe, it, expect } from "vitest";
import { createElement } from "react";
import { prerender } from "react-dom/static";
import { CompositeProvider } from "../../context/CompositeContext";
import { ComposeWizardProvider, useComposeWizardContext } from "../../context/ComposeWizardContext";
import {
  alignChannelOutcome,
  alignedRunFromChannels,
  alignMatchSummary,
  alignOverlayBinIds,
  alignOverlayColours,
  alignOverlayRequest,
  alignPreviewFrame,
  alignPreviewLegend,
  alignBlinkFrame,
  alignBlinkIndex,
  alignDisplayedOnLoad,
  alignRunOutcome,
  alignViewerState,
  ALIGN_IMAGE_LOAD_ERROR,
  applyCompositeOp,
  AUTO_WB_MANUAL_HINT,
  autoWbErrorText,
  ALIGN_INPUTS_CHANGED,
  alignInputs,
  alignRunFinish,
  BACKGROUND_UNALIGNED_NOTICE,
  backgroundAlignNotice,
  discardsNotice,
  effectiveBinFiles,
  fullyExcludedBins,
  nextAlignRun,
  nextAlignedRunState,
  rerunDiscards,
  resolveChannelPath,
  scnrAutoEnable,
  sameAlignInputs,
  sameAlignedRun,
  unalignedBins,
  unstackedBins,
  withExcludedFiles,
  formatAlignOffset,
  autoStfBlockedReason,
  binMenuKeyAction,
  binMenuPlacement,
  channelExportHistory,
  channelOutputPaths,
  channelStretchInput,
  compositeHistoryLines,
  droppedChannelOutputs,
  EMPTY_COMPOSITE_HISTORY,
  exportBlockedReason,
  exportWcsWarning,
  INITIAL_STATE,
  invalidateDownstream,
  levelMatchLine,
  nextEnabledStep,
  spccBlockReason,
  spccInputs,
  STEPS,
  WCS_MISSING_WARNING,
  wizardHasProgress,
  resolveExportRgbPaths,
  resolveOutputChannelPath,
  resolveRgbPaths,
  singleChannelBinId,
  starRemovalNote,
  withChannelStage,
  wizardChannelExport,
  wizardHeaderSourcePath,
  wizardStackName,
  wizardZipChannels,
  type CompositeHistory,
  type WizardState,
} from "../wizard";
import type { ChannelSource } from "../channelMapping";

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

  it("carries one level-match line per channel", () => {
    const h = applyCompositeOp(EMPTY_COMPOSITE_HISTORY, {
      kind: "blend",
      preset: "auto_wavelength",
      levels: [{ channel: "b F090W", scale: 4.7123 }, { channel: "g F187N", scale: 1 }],
    });
    const lines = ["Blend: auto_wavelength", "Level match b F090W: x4.712", "Level match g F187N: x1.000"];
    expect(compositeHistoryLines(h)).toEqual(lines);
    const balanced = applyCompositeOp(h, { kind: "colorBalance", mode: "manual", r: 1.2, g: 1, b: 1, scnr: null });
    expect(compositeHistoryLines(balanced)).toEqual([...lines, "White balance: manual R=1.200 G=1.000 B=1.000"]);
    const later = applyCompositeOp(applyCompositeOp(balanced, { kind: "lrgb", lightness: 1, chrominance: 1 }), { kind: "resetColorBalance" });
    expect(compositeHistoryLines(later)).toEqual([...lines, "LRGB: lightness 100%, chrominance 100%"]);
    const longest = levelMatchLine({ channel: "oiii F1000W", scale: 12345.678 });
    for (const line of [...compositeHistoryLines(balanced), longest]) {
      expect(line.length).toBeLessThanOrEqual(70);
      expect(line).toMatch(/^[\x20-\x7e]*$/);
    }
  });

  it("writes today's history for a blend without levels", () => {
    expect(compositeHistoryLines(applyCompositeOp(EMPTY_COMPOSITE_HISTORY, { kind: "blend", preset: "sho", levels: null }))).toEqual(["Blend: sho"]);
    expect(compositeHistoryLines(applyCompositeOp(EMPTY_COMPOSITE_HISTORY, { kind: "blend", preset: "sho", levels: [] }))).toEqual(["Blend: sho"]);
  });

  it("drops the level scales when a step before Blend is invalidated", () => {
    const s = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }, { blendLevelScales: { r: 2 } });
    expect(invalidateDownstream(s, "align").blendLevelScales).toBeNull();
    expect(invalidateDownstream(s, "channels").blendLevelScales).toBeNull();
    expect(invalidateDownstream(s, "blend").blendLevelScales).toBeUndefined();
  });
});

describe("wizardChannelExport", () => {
  it("sends three non-null slots", () => {
    const cases = [
      stateWith({ ha: ["/h.fits"] }),
      stateWith({ ha: ["/h.fits"] }, { channelResults: withChannelStage({}, "ha", "stretched", stretched) }),
      stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }),
      stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"], sii: ["/s.fits"] }),
      stateWith({ ha: ["/h.fits"], r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] }, {
        channelResults: withChannelStage({}, "ha", "stretched", stretched),
      }),
    ];
    for (const s of cases) {
      const out = wizardChannelExport(s);
      expect(out).not.toBeNull();
      expect([out?.r, out?.g, out?.b].every((p) => typeof p === "string" && p.length > 0)).toBe(true);
      expect(out).toEqual(resolveExportRgbPaths(s));
    }
    expect(wizardChannelExport(cases[4])?.monoBinId).toBe("ha");
    expect(wizardChannelExport(stateWith({}))).toBeNull();
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

const stepById = (id: string) => {
  const step = STEPS.find((s) => s.id === id);
  if (!step) throw new Error(`no step ${id}`);
  return step;
};

describe("wizard step gates", () => {
  const empty = stateWith({});
  const oneFrame = stateWith({ ha: ["/h.fits"] });
  const oneChannelStack = stateWith({ ha: ["/h1.fits", "/h2.fits"] });
  const twoChannels = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] });
  const aligned = { ...twoChannels, alignedPaths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" } };
  const blended = { ...aligned, compositeReady: true };
  const twoUnstacked = stateWith({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o1.fits", "/o2.fits"] });
  const halfStacked = { ...twoUnstacked, stackedPaths: { ha: "/out/stacked_ha_1.fits" } };
  const keptOne = { ...twoUnstacked, excludedFiles: { ha: ["/h1.fits"], oiii: ["/o2.fits"] } };
  const allExcluded = stateWith({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"] }, { excludedFiles: { ha: ["/h1.fits", "/h2.fits"] } });
  const allExcludedOfThree = { ...allExcluded, bins: stateWith({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"], sii: ["/s.fits"] }).bins };
  const R1_STEPS = ["align", "background", "blend", "colorbalance", "stretch", "export"];
  const FIXTURES = [
    empty, oneFrame, oneChannelStack, twoChannels, aligned, blended, twoUnstacked, halfStacked, keptOne, allExcluded, allExcludedOfThree,
  ];

  it("keeps Export locked until a channel is assigned or a composite exists", () => {
    expect(stepById("export").enabled(empty)).toBe(false);
    expect(stepById("export").enabled(oneFrame)).toBe(true);
    expect(stepById("export").enabled({ ...empty, compositeReady: true })).toBe(true);
  });

  it("gives a reason exactly when a step is locked", () => {
    for (const s of FIXTURES) {
      for (const step of STEPS) {
        expect(step.blockedReason(s) === null, `${step.id}`).toBe(step.enabled(s));
      }
    }
  });

  it("names what each locked step is waiting for", () => {
    expect(stepById("stack").blockedReason(empty)).toBe("assign frames in step 1");
    expect(stepById("stack").blockedReason(oneFrame)).toBe("needs a channel with 2+ frames");
    expect(stepById("align").blockedReason(oneChannelStack)).toBe("assign at least 2 channels");
    expect(stepById("crop").blockedReason(oneFrame)).toBe("assign at least 2 channels");
    expect(stepById("crop").blockedReason(twoChannels)).toBe("run Align first");
    expect(stepById("blend").blockedReason(oneFrame)).toBe("assign at least 2 channels");
    expect(stepById("colorbalance").blockedReason(oneFrame)).toBe("assign at least 2 channels");
    expect(stepById("adjust").blockedReason(oneFrame)).toBe("assign at least 2 channels, then run Blend");
    expect(stepById("adjust").blockedReason(twoChannels)).toBe("run Blend first");
    expect(stepById("background").blockedReason(empty)).toBe("assign frames in step 1");
    expect(stepById("stretch").blockedReason(empty)).toBe("assign frames in step 1");
    expect(stepById("export").blockedReason(empty)).toBe("assign frames in step 1");
  });

  it("asks to stack a single channel with 2+ frames before BG, Stretch and Export", () => {
    const reason = "stack Hα first (2+ frames), or keep one file per channel";
    expect(stepById("background").blockedReason(oneChannelStack)).toBe(reason);
    expect(stepById("stretch").blockedReason(oneChannelStack)).toBe(reason);
    expect(stepById("export").blockedReason(oneChannelStack)).toBe(reason);
    expect(stepById("align").blockedReason(oneChannelStack)).toBe("assign at least 2 channels");
    expect(stepById("blend").blockedReason(oneChannelStack)).toBe("assign at least 2 channels");
  });

  it("names every unstacked channel in the gates of Align, BG, Blend, Color, Stretch and Export", () => {
    const reason = "stack Hα and OIII first (2+ frames each), or keep one file per channel";
    for (const id of R1_STEPS) {
      expect(stepById(id).blockedReason(twoUnstacked), id).toBe(reason);
    }
    const three = stateWith({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o1.fits", "/o2.fits"], sii: ["/s1.fits", "/s2.fits"] });
    expect(stepById("align").blockedReason(three)).toBe(
      "stack Hα, OIII and SII first (2+ frames each), or keep one file per channel",
    );
    expect(stepById("align").blockedReason(halfStacked)).toBe("stack OIII first (2+ frames), or keep one file per channel");
  });

  it("opens the later steps once every channel is stacked or keeps a single effective file", () => {
    const stacked = { ...twoUnstacked, stackedPaths: { ha: "/out/stacked_ha_1.fits", oiii: "/out/stacked_oiii_1.fits" } };
    for (const s of [stacked, keptOne]) {
      for (const id of R1_STEPS) {
        expect(stepById(id).blockedReason(s), id).toBeNull();
      }
    }
  });

  it("names a channel whose frames are all excluded in the gates of Align, BG, Blend, Color, Stretch and Export", () => {
    for (const s of [allExcluded, allExcludedOfThree]) {
      for (const id of R1_STEPS) {
        expect(stepById(id).blockedReason(s), id).toBe("all Hα frames are excluded; re-include one in Stack");
      }
    }
    const two = { ...allExcludedOfThree, excludedFiles: { ha: ["/h1.fits", "/h2.fits"], sii: ["/s.fits"] } };
    expect(stepById("align").blockedReason(two)).toBe("all Hα and SII frames are excluded; re-include one per channel in Stack");
    const alsoUnstacked = stateWith(
      { ha: ["/h1.fits", "/h2.fits"], oiii: ["/o1.fits", "/o2.fits"] },
      { excludedFiles: { ha: ["/h1.fits", "/h2.fits"] } },
    );
    expect(stepById("align").blockedReason(alsoUnstacked)).toBe("all Hα frames are excluded; re-include one in Stack");
    const lone = stateWith({ ha: ["/h1.fits", "/h2.fits"] }, { excludedFiles: { ha: ["/h1.fits", "/h2.fits"] } });
    for (const id of ["background", "stretch", "export"]) {
      expect(stepById(id).blockedReason(lone), id).toBe("all Hα frames are excluded; re-include one in Stack");
    }
  });

  it("keeps a stacked channel usable when its frames are excluded afterwards", () => {
    const stacked = { ...allExcluded, stackedPaths: { ha: "/out/stacked_ha_1.fits" } };
    expect(fullyExcludedBins(stacked)).toEqual([]);
    expect(fullyExcludedBins(allExcluded).map((b) => b.id)).toEqual(["ha"]);
    for (const id of R1_STEPS) {
      expect(stepById(id).blockedReason(stacked), id).toBeNull();
    }
  });

  it("opens Align, BG and Blend only when every filled channel has an input", () => {
    for (const s of FIXTURES) {
      const filled = s.bins.filter((b) => b.files.length > 0).length;
      for (const id of ["align", "background", "blend"]) {
        if (stepById(id).enabled(s)) expect(alignInputs(s), id).toHaveLength(filled);
      }
    }
  });

  it("leaves the composite steps open once Blend has built the composite", () => {
    const composite = { ...twoUnstacked, compositeReady: true };
    for (const id of ["colorbalance", "stretch", "export"]) {
      expect(stepById(id).blockedReason(composite), id).toBeNull();
    }
    expect(stepById("blend").blockedReason(composite)).not.toBeNull();
  });

  it("suggests only Stack while a channel with 2+ frames is unstacked", () => {
    expect(nextEnabledStep("channels", twoUnstacked)).toBe("stack");
    expect(nextEnabledStep("stack", twoUnstacked)).toBeNull();
    expect(nextEnabledStep("stack", halfStacked)).toBeNull();
  });

  it("suggests Crop after the first Align when the suggestion reads the state that holds the aligned paths", () => {
    expect(nextEnabledStep("align", twoChannels)).toBe("background");
    expect(nextEnabledStep("align", aligned)).toBe("crop");
    expect(nextEnabledStep("blend", blended)).toBe("colorbalance");
  });
});

describe("unstacked channels and excluded files", () => {
  it("counts the files left after exclusions and ignores stacked bins", () => {
    const s = stateWith(
      { ha: ["/h1.fits", "/h2.fits", "/h3.fits"], oiii: ["/o1.fits", "/o2.fits"], sii: ["/s1.fits", "/s2.fits"], r: ["/r.fits"] },
      { excludedFiles: { ha: ["/h1.fits", "/h3.fits"] }, stackedPaths: { sii: "/out/stacked_sii_1.fits" } },
    );
    expect(effectiveBinFiles(s, s.bins.find((b) => b.id === "ha")!)).toEqual(["/h2.fits"]);
    expect(unstackedBins(s).map((b) => b.id)).toEqual(["oiii"]);
  });

  it("never resolves a channel to an excluded file", () => {
    const s = stateWith({ ha: ["/h1.fits", "/h2.fits"] }, { excludedFiles: { ha: ["/h1.fits"] } });
    expect(resolveChannelPath(s, "ha")).toBe("/h2.fits");
    expect(resolveChannelPath({ ...s, excludedFiles: { ha: ["/h1.fits", "/h2.fits"] } }, "ha")).toBeNull();
    expect(resolveChannelPath({ ...s, stackedPaths: { ha: "/out/stacked_ha_1.fits" } }, "ha")).toBe("/out/stacked_ha_1.fits");
  });

  it("reads the header of the file the channel uses, not an excluded one", () => {
    const s = stateWith(
      { ha: ["/raw/ha_1.fits", "/raw/ha_2.fits"], oiii: ["/raw/o_1.fits"] },
      { excludedFiles: { ha: ["/raw/ha_1.fits"] }, alignedPaths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" } },
    );
    expect(wizardHeaderSourcePath(s, [])).toBe("/raw/ha_2.fits");
  });

  it("drops the results built on a channel whose input an exclusion changes, and keeps them otherwise", () => {
    const base = stateWith(
      { ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"] },
      {
        excludedFiles: { ha: ["/h1.fits"] },
        alignedPaths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" },
        backgroundPaths: { ha: "__wizard_ch_ha_bg" },
        compositeReady: true,
        completedSteps: { channels: true, stack: true, align: true, background: true, blend: true },
      },
    );
    const switched = withExcludedFiles(base, "ha", ["/h2.fits"]);
    expect(switched.excludedFiles.ha).toEqual(["/h2.fits"]);
    expect(switched.alignedPaths).toEqual({});
    expect(switched.backgroundPaths).toEqual({});
    expect(switched.compositeReady).toBe(false);
    expect(switched.completedSteps).toEqual({ channels: true, stack: true });
    const same = withExcludedFiles(base, "ha", ["/h1.fits"]);
    expect(same.alignedPaths).toBe(base.alignedPaths);
    expect(same.compositeReady).toBe(true);
    const stacked = { ...base, stackedPaths: { ha: "/out/stacked_ha_1.fits" } };
    expect(withExcludedFiles(stacked, "ha", ["/h2.fits"]).alignedPaths).toBe(base.alignedPaths);
    expect(withExcludedFiles(stacked, "ha", []).alignedPaths).toBe(base.alignedPaths);
  });

  it("drops the composite when re-including a frame makes the channel need a stack, although its input file is the same", () => {
    const base = stateWith(
      { ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"] },
      {
        excludedFiles: { ha: ["/h2.fits"] },
        alignedPaths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" },
        compositeReady: true,
        completedSteps: { channels: true, align: true, blend: true },
      },
    );
    const reincluded = withExcludedFiles(base, "ha", []);
    expect(resolveChannelPath(reincluded, "ha", "stacked")).toBe(resolveChannelPath(base, "ha", "stacked"));
    expect(reincluded.compositeReady).toBe(false);
    expect(reincluded.alignedPaths).toEqual({});
    expect(reincluded.completedSteps).toEqual({ channels: true });
    for (const id of ["align", "background", "blend", "colorbalance", "stretch", "export"]) {
      expect(STEPS.find((s) => s.id === id)!.blockedReason(reincluded), id).toBe(
        "stack Hα first (2+ frames), or keep one file per channel",
      );
    }
  });

  it("drops the results built on a channel when every one of its frames is excluded", () => {
    const base = stateWith(
      { ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"] },
      {
        excludedFiles: { ha: ["/h2.fits"] },
        alignedPaths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" },
        compositeReady: true,
      },
    );
    const none = withExcludedFiles(base, "ha", ["/h1.fits", "/h2.fits"]);
    expect(resolveChannelPath(none, "ha")).toBeNull();
    expect(none.alignedPaths).toEqual({});
    expect(none.compositeReady).toBe(false);
  });
});

describe("dependency warnings between Align, Crop, BG and Blend", () => {
  const two = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] });
  const alignedKeys = { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" };

  it("flags every filled channel without an aligned key once 2+ channels are filled, whatever its later stage", () => {
    expect(unalignedBins(stateWith({ ha: ["/h.fits"] }))).toEqual([]);
    expect(unalignedBins(two).map((b) => b.id)).toEqual(["ha", "oiii"]);
    const afterBg = { ...two, backgroundPaths: { ha: "__wizard_ch_ha_bg", oiii: "__wizard_ch_oiii_bg" } };
    expect(unalignedBins(afterBg).map((b) => b.id)).toEqual(["ha", "oiii"]);
    expect(unalignedBins({ ...afterBg, alignedPaths: alignedKeys })).toEqual([]);
    expect(unalignedBins({ ...two, alignedPaths: { ha: alignedKeys.ha } }).map((b) => b.id)).toEqual(["oiii"]);
  });

  it("warns in BG only when 2+ channels are filled and Align was skipped", () => {
    expect(backgroundAlignNotice(stateWith({ ha: ["/h.fits"] }))).toBeNull();
    expect(backgroundAlignNotice(two)).toBe(BACKGROUND_UNALIGNED_NOTICE);
    expect(backgroundAlignNotice({ ...two, alignedPaths: alignedKeys })).toBeNull();
  });

  it("lists the later results a new Align or Crop would clear, in step order", () => {
    const full = {
      ...two,
      alignedPaths: alignedKeys,
      croppedPaths: { ha: "__wizard_ch_ha_cropped", oiii: "__wizard_ch_oiii_cropped" },
      backgroundPaths: { ha: "__wizard_ch_ha_bg" },
      compositeReady: true,
      channelResults: withChannelStage({}, "ha", "starless", starless),
      completedSteps: { channels: true, align: true, crop: true, background: true, blend: true, colorbalance: true, adjust: true },
    };
    expect(rerunDiscards(full, "align")).toEqual(["Crop", "BG", "Blend", "Color", "Stretch", "Adjust"]);
    expect(rerunDiscards(full, "crop")).toEqual(["BG", "Blend", "Color", "Stretch", "Adjust"]);
    expect(rerunDiscards({ ...two, backgroundPaths: { ha: "__wizard_ch_ha_bg" } }, "align")).toEqual(["BG"]);
    expect(rerunDiscards(two, "align")).toEqual([]);
    expect(rerunDiscards(full, "unknown")).toEqual([]);
  });

  it("names only what invalidateDownstream actually clears", () => {
    const s = { ...two, backgroundPaths: { ha: "__wizard_ch_ha_bg" }, compositeReady: true };
    const after = { ...s, ...invalidateDownstream(s, "background") };
    expect(rerunDiscards(s, "background")).toEqual(["Blend"]);
    expect(after.backgroundPaths).toEqual(s.backgroundPaths);
    expect(after.compositeReady).toBe(false);
  });

  it("writes one short line, or nothing when there is nothing to discard", () => {
    expect(discardsNotice("Running Align", ["BG", "Blend"])).toBe("Running Align discards: BG, Blend");
    expect(discardsNotice("Apply or Skip", [])).toBeNull();
  });
});

describe("Align result that lands after its inputs changed", () => {
  const base = stateWith({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"] }, { stackedPaths: { ha: "/out/stacked_ha_1.fits" } });
  const result = { channels: [] };

  it("takes the stacked or single-file input of every filled bin, with its bin id", () => {
    expect(alignInputs(base)).toEqual([
      { binId: "ha", path: "/out/stacked_ha_1.fits" },
      { binId: "oiii", path: "/o.fits" },
    ]);
    const bg = { ...base, backgroundPaths: { ha: "__wizard_ch_ha_bg" }, alignedPaths: { ha: "__wizard_ch_ha_aligned" } };
    expect(alignInputs(bg)).toEqual(alignInputs(base));
  });

  it("compares bin ids and paths in order", () => {
    const a = alignInputs(base);
    expect(sameAlignInputs(a, a.map((x) => ({ ...x })))).toBe(true);
    expect(sameAlignInputs(a, [{ ...a[0], binId: "r" }, a[1]])).toBe(false);
    expect(sameAlignInputs(a, [a[1], a[0]])).toBe(false);
    expect(sameAlignInputs(a, a.slice(0, 1))).toBe(false);
  });

  it("stores the keys of a run whose inputs did not change", () => {
    const started = alignInputs(base);
    expect(alignRunFinish(started, alignInputs({ ...base }), result, "")).toEqual({
      record: { running: false, inputs: ["/out/stacked_ha_1.fits", "/o.fits"], result, error: "" },
      store: true,
    });
    expect(alignRunFinish(started, alignInputs(base), null, "boom")).toEqual({
      record: { running: false, inputs: ["/out/stacked_ha_1.fits", "/o.fits"], result: null, error: "boom" },
      store: false,
    });
  });

  it("discards a run when a new stack or new bins changed its inputs mid-run, and says why", () => {
    const started = alignInputs(base);
    const restacked = { ...base, ...invalidateDownstream(base, "stack"), stackedPaths: { ha: "/out/stacked_ha_2.fits" } };
    const outcome = alignRunFinish(started, alignInputs(restacked), result, "");
    expect(outcome.store).toBe(false);
    expect(outcome.record).toEqual({
      running: false, inputs: ["/out/stacked_ha_2.fits", "/o.fits"], result: null, error: ALIGN_INPUTS_CHANGED,
    });
    const shown = alignRunOutcome(outcome.record, alignInputs(restacked).map((c) => c.path), false);
    expect(shown).toEqual({ result: null, error: "Inputs changed while aligning; run Align again" });
    const rebinned = stateWith({ ha: ["/h1.fits", "/h2.fits"], sii: ["/o.fits"] }, { stackedPaths: base.stackedPaths });
    expect(alignRunFinish(started, alignInputs(rebinned), result, "").store).toBe(false);
    expect(alignRunFinish(started, alignInputs(stateWith({})), result, "boom").record.error).toBe(ALIGN_INPUTS_CHANGED);
  });

  it("discards a run when an exclusion changes the file a channel uses", () => {
    const single = stateWith({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"] }, { excludedFiles: { ha: ["/h1.fits"] } });
    const started = alignInputs(single);
    const switched = withExcludedFiles(single, "ha", ["/h2.fits"]);
    expect(alignRunFinish(started, alignInputs(switched), result, "").store).toBe(false);
  });
});

describe("Align run record across a reset", () => {
  const running = { running: true, inputs: ["/a", "/b"], result: null, error: "" };
  const finished = { running: false, inputs: ["/a", "/b"], result: { ok: 1 }, error: "" };

  it("records the finished run when it is still the current one", () => {
    const started = nextAlignRun(null, { type: "start", run: running });
    expect(nextAlignRun(started, { type: "finish", started: running, run: finished })).toBe(finished);
  });

  it("clears the record on reset, also while a run is in flight, and ignores that run's late finish", () => {
    const started = nextAlignRun(null, { type: "start", run: running });
    const reset = nextAlignRun(started, { type: "reset" });
    expect(reset).toBeNull();
    expect(nextAlignRun(reset, { type: "finish", started: running, run: finished })).toBeNull();
  });

  it("keeps a newer run when an older one finishes", () => {
    const newer = { running: true, inputs: ["/c", "/d"], result: null, error: "" };
    const afterReset = nextAlignRun(nextAlignRun(null, { type: "start", run: running }), { type: "reset" });
    const current = nextAlignRun(afterReset, { type: "start", run: newer });
    expect(nextAlignRun(current, { type: "finish", started: running, run: finished })).toBe(newer);
  });
});

type WizardContextValue = ReturnType<typeof useComposeWizardContext>;

async function mountWizard(): Promise<WizardContextValue> {
  const seen: WizardContextValue[] = [];
  function Capture(): null {
    seen.push(useComposeWizardContext());
    return null;
  }
  const tree = createElement(CompositeProvider, {
    children: createElement(ComposeWizardProvider, { children: createElement(Capture) }),
  });
  await prerender(tree);
  return seen[0];
}

describe("wizard provider", () => {
  const binsOf = (files: Record<string, string[]>) => stateWith(files).bins;
  const analysis = { subframes: [], total: 2, accepted: 0, rejected: 2, elapsed_ms: 1 };
  const running = { running: true, inputs: ["/a", "/b"], result: null, error: "" };
  const finished = { ...running, running: false };

  it("reads actions dispatched before the next render through getState", async () => {
    const ctx = await mountWizard();
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h.fits"], oiii: ["/o.fits"] }) });
    ctx.dispatch({ type: "SET_STACKED", channelId: "ha", path: "/out/stacked_ha_1.fits" });
    expect(alignInputs(ctx.getState())).toEqual([
      { binId: "ha", path: "/out/stacked_ha_1.fits" },
      { binId: "oiii", path: "/o.fits" },
    ]);
  });

  it("forgets the exclusions and the analysis of a channel whose frames change, and keeps the others", async () => {
    const ctx = await mountWizard();
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o1.fits", "/o2.fits"] }) });
    ctx.dispatch({ type: "SET_SUBFRAME_RESULT", binId: "ha", result: analysis });
    ctx.dispatch({ type: "SET_SUBFRAME_RESULT", binId: "oiii", result: analysis });
    ctx.dispatch({ type: "SET_EXCLUDED_FILES", binId: "ha", files: ["/h1.fits"] });
    ctx.dispatch({ type: "SET_EXCLUDED_FILES", binId: "oiii", files: ["/o2.fits"] });
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h1.fits"], oiii: ["/o1.fits", "/o2.fits"] }) });
    const s = ctx.getState();
    expect(s.excludedFiles).toEqual({ oiii: ["/o2.fits"] });
    expect(Object.keys(s.subframeResults)).toEqual(["oiii"]);
    expect(resolveChannelPath(s, "ha")).toBe("/h1.fits");
    expect(STEPS.find((step) => step.id === "align")!.blockedReason(s)).toBeNull();
  });

  it("keeps the exclusions when the channels are set again with the same frames", async () => {
    const ctx = await mountWizard();
    const bins = binsOf({ ha: ["/h1.fits", "/h2.fits"], oiii: ["/o.fits"] });
    ctx.dispatch({ type: "SET_BINS", bins });
    ctx.dispatch({ type: "SET_EXCLUDED_FILES", binId: "ha", files: ["/h1.fits"] });
    ctx.dispatch({ type: "SET_BINS", bins: bins.map((b) => ({ ...b })) });
    expect(ctx.getState().excludedFiles).toEqual({ ha: ["/h1.fits"] });
  });

  it("gives a new channel set the Match levels default again", async () => {
    const ctx = await mountWizard();
    const bins = binsOf({ ha: ["/h.fits"], oiii: ["/o.fits"] });
    ctx.dispatch({ type: "SET_BINS", bins });
    ctx.dispatch({ type: "UPDATE", partial: { levelMatch: false, blendLevelScales: { ha: 1, oiii: 3 } } });
    ctx.dispatch({ type: "SET_BINS", bins: bins.map((b) => ({ ...b })) });
    expect(ctx.getState().levelMatch).toBe(false);
    expect(ctx.getState().blendLevelScales).toEqual({ ha: 1, oiii: 3 });
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ ha: ["/h.fits"], sii: ["/s.fits"] }) });
    expect(ctx.getState().levelMatch).toBeNull();
    expect(ctx.getState().blendLevelScales).toBeNull();
  });

  it("re-derives the SPCC factors when a later Blend changes the level-match scales", async () => {
    const ctx = await mountWizard();
    const wb = () => {
      const s = ctx.getState();
      return [s.wbR, s.wbG, s.wbB];
    };
    ctx.dispatch({ type: "SET_BINS", bins: binsOf({ r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] }) });
    ctx.dispatch({ type: "UPDATE", partial: { blendLevelScales: null } });
    ctx.dispatch({ type: "UPDATE", partial: { wbMode: "spcc", spccFactors: { r: 1.1, g: 1, b: 0.9 } } });
    expect(wb()).toEqual([1.1, 1, 0.9]);
    ctx.dispatch({ type: "UPDATE", partial: { blendLevelScales: { r: 1, g: 1.4, b: 2.1 } } });
    expect(wb()[0]).toBe(1.1);
    expect(wb()[1]).toBeCloseTo(1 / 1.4, 12);
    expect(wb()[2]).toBeCloseTo(0.9 / 2.1, 12);
    ctx.dispatch({ type: "UPDATE", partial: { blendLevelScales: null } });
    expect(wb()).toEqual([1.1, 1, 0.9]);
    ctx.dispatch({ type: "SET_WB", mode: "manual", r: 1.3, g: 1, b: 0.8 });
    ctx.dispatch({ type: "UPDATE", partial: { blendLevelScales: { r: 2, g: 1, b: 1 } } });
    expect(wb()).toEqual([1.3, 1, 0.8]);
    ctx.dispatch({ type: "SET_WB", mode: "spcc", r: 1.3, g: 1, b: 0.8 });
    expect(wb()[0]).toBeCloseTo(0.55, 12);
    expect(wb().slice(1)).toEqual([1, 0.9]);
  });

  it("drops the Align run on RESET, so the late finish of a run in flight is ignored", async () => {
    const live = await mountWizard();
    live.startAlignRun(running);
    expect(live.finishAlignRun(running, finished)).toBe(true);
    const reset = await mountWizard();
    reset.startAlignRun(running);
    reset.dispatch({ type: "RESET" });
    expect(reset.finishAlignRun(running, finished)).toBe(false);
  });
});

describe("wizardHasProgress", () => {
  it("asks before a reset only when frames are assigned or a step is complete", () => {
    expect(wizardHasProgress(stateWith({}))).toBe(false);
    expect(wizardHasProgress(stateWith({}, { completedSteps: { stack: false } }))).toBe(false);
    expect(wizardHasProgress(stateWith({ ha: ["/h.fits"] }))).toBe(true);
    expect(wizardHasProgress(stateWith({}, { completedSteps: { export: true } }))).toBe(true);
  });
});

describe("SPCC gate", () => {
  const frame = (path: string, filter?: string): ChannelSource & { path: string } => ({
    path,
    name: path.split("/").pop(),
    result: { header: filter ? { FILTER: filter } : {} },
  });

  it("measures the R, G and B bins and reports which bin each input comes from", () => {
    const s = stateWith({ r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"], ha: ["/h.fits"] });
    expect(spccInputs(s)).toEqual({
      r: { binId: "r", path: "/r.fits" },
      g: { binId: "g", path: "/g.fits" },
      b: { binId: "b", path: "/b.fits" },
    });
  });

  it("allows broadband RGB, also next to an Hα channel (HaRGB)", () => {
    const files = [frame("/r.fits", "Red"), frame("/g.fits", "Green"), frame("/b.fits", "Blue"), frame("/h.fits", "Ha")];
    expect(spccBlockReason(stateWith({ r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] }), files)).toBeNull();
    expect(spccBlockReason(stateWith({ r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"], ha: ["/h.fits"] }), files)).toBeNull();
  });

  it("refuses inputs that fall back to the Hα, OIII or SII bins", () => {
    const sho = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"], sii: ["/s.fits"] });
    expect(spccInputs(sho).r).toEqual({ binId: "ha", path: "/h.fits" });
    expect(spccBlockReason(sho, [])).toBe(
      "SPCC models broadband R/G/B filters only; Hα, OIII, SII would be measured as R, G, B.",
    );
    const mixed = stateWith({ r: ["/r.fits"], g: ["/g.fits"], sii: ["/s.fits"] });
    expect(spccBlockReason(mixed, [frame("/r.fits", "Red"), frame("/g.fits", "Green")])).toBe(
      "SPCC models broadband R/G/B filters only; SII would be measured as B.",
    );
  });

  it("refuses narrowband frames dropped into the R, G or B bins, by header, by the header scan or by file name", () => {
    const hst = stateWith({ r: ["/d/f673n.fits"], g: ["/d/f656n.fits"], b: ["/d/f502n.fits"] });
    const byHeader = [frame("/d/f673n.fits", "F673N"), frame("/d/f656n.fits", "F656N"), frame("/d/f502n.fits", "F502N")];
    expect(spccBlockReason(hst, byHeader)).toBe(
      "SPCC models broadband R/G/B filters only; f673n in R is a narrowband frame (F673N).",
    );
    const wfpc2 = stateWith({ r: ["/d/673nmos.fits"], g: ["/d/656nmos.fits"], b: ["/d/502nmos.fits"] });
    const wheel = [frame("/d/673nmos.fits", "33"), frame("/d/656nmos.fits", "31"), frame("/d/502nmos.fits", "23")];
    expect(spccBlockReason(wfpc2, wheel)).toBeNull();
    expect(spccBlockReason(wfpc2, wheel, [{ path: "/d/656nmos.fits", filter: "Hα (656nm)" }])).toBe(
      "SPCC models broadband R/G/B filters only; 656nmos in G is a narrowband frame (Hα (656nm)).",
    );
    const named = stateWith({ r: ["/d/m42_Ha_001.fits"], g: ["/g.fits"], b: ["/b.fits"] });
    expect(spccBlockReason(named, [frame("/d/m42_Ha_001.fits"), frame("/g.fits", "Green"), frame("/b.fits", "Blue")])).toBe(
      "SPCC models broadband R/G/B filters only; m42_Ha_001 in R is a narrowband frame (Hα).",
    );
    const jwst = stateWith({ r: ["/n/f470n.fits"], g: ["/n/f200w.fits"], b: ["/n/f090w.fits"] });
    expect(spccBlockReason(jwst, [frame("/n/f470n.fits", "F470N"), frame("/n/f200w.fits", "F200W"), frame("/n/f090w.fits", "F090W")])).toBe(
      "SPCC models broadband R/G/B filters only; f470n in R is a narrowband frame (F470N).",
    );
  });

  it("says nothing while an input is missing, which the step reports on its own", () => {
    expect(spccBlockReason(stateWith({ r: ["/r.fits"] }), [frame("/r.fits", "Red")])).toBeNull();
  });

  it("blocks a medium-band frame by its code", () => {
    const s = stateWith({ r: ["/n/f410m.fits"], g: ["/n/f200w.fits"], b: ["/n/f090w.fits"] });
    const files = [frame("/n/f410m.fits", "F410M"), frame("/n/f200w.fits", "F200W"), frame("/n/f090w.fits", "F090W")];
    expect(spccBlockReason(s, files)).toBe(
      "SPCC models broadband R/G/B filters only; f410m in R is a medium-band frame (F410M).",
    );
  });

  it("blocks an unmapped medium band by the pattern", () => {
    const s = stateWith({ r: ["/h/f547m.fits"], g: ["/h/f606w.fits"], b: ["/h/f435w.fits"] });
    const files = [frame("/h/f547m.fits", "F547M"), frame("/h/f606w.fits", "F606W"), frame("/h/f435w.fits", "F435W")];
    expect(spccBlockReason(s, files)).toBe(
      "SPCC models broadband R/G/B filters only; f547m in R is a medium-band frame (F547M).",
    );
  });

  it("blocks an infrared broadband by wavelength and names it", () => {
    const s = stateWith({ r: ["/h/f814w.fits"], g: ["/h/f606w.fits"], b: ["/n/f090w.fits"] });
    const files = [frame("/h/f814w.fits", "F814W"), frame("/h/f606w.fits", "F606W"), frame("/n/f090w.fits", "F090W")];
    expect(spccBlockReason(s, files)).toBe(
      "SPCC models visible light (380-830 nm) only; f090w in B is F090W at 900 nm.",
    );
  });

  it("names the first infrared channel of a NIRCam set", () => {
    const s = stateWith({ r: ["/n/f444w.fits"], g: ["/n/f200w.fits"], b: ["/n/f090w.fits"] });
    const files = [frame("/n/f444w.fits", "F444W"), frame("/n/f200w.fits", "F200W"), frame("/n/f090w.fits", "F090W")];
    expect(spccBlockReason(s, files)).toBe(
      "SPCC models visible light (380-830 nm) only; f444w in R is F444W at 4440 nm.",
    );
  });

  it("reads a narrowband PUPIL", () => {
    const pupil = "/n/jw_f444w-f470n_i2d.fits";
    const s = stateWith({ r: [pupil], g: ["/n/f200w.fits"], b: ["/n/f090w.fits"] });
    const files = [
      { path: pupil, name: "jw_f444w-f470n_i2d.fits", result: { header: { FILTER: "F444W", PUPIL: "F470N" } } },
      frame("/n/f200w.fits", "F200W"),
      frame("/n/f090w.fits", "F090W"),
    ];
    expect(spccBlockReason(s, files)).toBe(
      "SPCC models broadband R/G/B filters only; jw_f444w-f470n_i2d in R is a narrowband frame (F470N).",
    );
  });

  it("reports a narrowband frame before an infrared one, as today", () => {
    const s = stateWith({ r: ["/n/r4_f200w.fits"], g: ["/n/r4_f187n.fits"], b: ["/n/r4_f090w.fits"] });
    const files = [frame("/n/r4_f200w.fits", "F200W"), frame("/n/r4_f187n.fits", "F187N"), frame("/n/r4_f090w.fits", "F090W")];
    expect(spccBlockReason(s, files)).toBe(
      "SPCC models broadband R/G/B filters only; r4_f187n in G is a narrowband frame (F187N).",
    );
    const mixed = stateWith({ r: ["/n/f444w.fits"], g: ["/n/f410m.fits"], b: ["/n/f090w.fits"] });
    expect(spccBlockReason(mixed, [frame("/n/f444w.fits", "F444W"), frame("/n/f410m.fits", "F410M"), frame("/n/f090w.fits", "F090W")])).toBe(
      "SPCC models broadband R/G/B filters only; f410m in G is a medium-band frame (F410M).",
    );
  });

  it("keeps visible broadband and unmapped codes allowed", () => {
    const hst = stateWith({ r: ["/h/f814w.fits"], g: ["/h/f606w.fits"], b: ["/h/f435w.fits"] });
    expect(spccBlockReason(hst, [frame("/h/f814w.fits", "F814W"), frame("/h/f606w.fits", "F606W"), frame("/h/f435w.fits", "F435W")])).toBeNull();
    const wide = stateWith({ r: ["/h/f814w.fits"], g: ["/h/f606w.fits"], b: ["/n/f070w.fits"] });
    expect(spccBlockReason(wide, [frame("/h/f814w.fits", "F814W"), frame("/h/f606w.fits", "F606W"), frame("/n/f070w.fits", "F070W")])).toBeNull();
  });
});

describe("scnrAutoEnable", () => {
  it("switches SCNR on only for broadband data that SPCC can model", () => {
    expect(scnrAutoEnable(false, null)).toBe(true);
    expect(scnrAutoEnable(false, "x")).toBe(false);
    expect(scnrAutoEnable(true, null)).toBe(false);
  });
});

describe("autoWbErrorText", () => {
  const rust =
    "Auto white balance needs a sky level clearly above zero, but the median of channel G is within 3 sigma of zero (the sky was probably subtracted). Use SPCC or manual factors.";

  it("suggests manual factors instead of SPCC when SPCC cannot run", () => {
    const text = autoWbErrorText(rust, false);
    expect(text.endsWith(AUTO_WB_MANUAL_HINT)).toBe(true);
    expect(text.endsWith("Use manual factors; SPCC does not model these filters.")).toBe(true);
    expect(text).not.toContain("SPCC or");
  });

  it("keeps the backend text when SPCC is allowed or the hint is absent", () => {
    expect(autoWbErrorText(rust, true)).toBe(rust);
    expect(autoWbErrorText("Auto WB: composite cache is empty", false)).toBe("Auto WB: composite cache is empty");
  });
});

describe("alignChannelOutcome", () => {
  it("tags a channel left at identity and suggests the other method", () => {
    expect(alignChannelOutcome({ registered: false, method_used: "phase_correlation_identity" }, "phase_correlation", false)).toEqual({
      unregistered: "not registered — low-confidence match, channel left unshifted; try Star-based Affine",
      usedMethod: null,
    });
    expect(alignChannelOutcome({ registered: false, method_used: "identity" }, "affine", false).unregistered).toBe(
      "not registered — low-confidence match, channel left unshifted; try Phase Correlation",
    );
  });

  it("reads the method name when the backend does not send the registered flag", () => {
    expect(alignChannelOutcome({ method_used: "identity" }, "affine", false).unregistered).not.toBeNull();
    expect(alignChannelOutcome({ method_used: "phase_correlation_identity" }, "phase_correlation", false).unregistered).not.toBeNull();
    expect(alignChannelOutcome({ method_used: "phase_correlation" }, "phase_correlation", false).unregistered).toBeNull();
  });

  it("names the method actually used when it differs from the requested one", () => {
    expect(alignChannelOutcome({ registered: true, method_used: "rigid" }, "affine", false)).toEqual({ unregistered: null, usedMethod: "rigid" });
    expect(alignChannelOutcome({ registered: true, method_used: "phase_correlation" }, "affine", false).usedMethod).toBe("phase correlation");
    expect(alignChannelOutcome({ registered: true, method_used: "affine" }, "affine", false).usedMethod).toBeNull();
  });

  it("never tags the reference channel", () => {
    expect(alignChannelOutcome({}, "phase_correlation", true)).toEqual({ unregistered: null, usedMethod: null });
    expect(alignChannelOutcome(undefined, "phase_correlation", false)).toEqual({ unregistered: null, usedMethod: null });
  });
});

describe("formatAlignOffset", () => {
  it("reads the backend offset as [dy, dx] and labels both axes with sign and unit", () => {
    expect(formatAlignOffset([-9, 14])).toBe("Δx +14.0 px  Δy −9.0 px");
    expect(formatAlignOffset([2.26, -0.34])).toBe("Δx −0.3 px  Δy +2.3 px");
  });

  it("prints no sign for an offset that rounds to zero, including negative zero", () => {
    expect(formatAlignOffset([0, 0])).toBe("Δx 0.0 px  Δy 0.0 px");
    expect(formatAlignOffset([-0.04, 0.04])).toBe("Δx 0.0 px  Δy 0.0 px");
    expect(formatAlignOffset([-0, -0])).toBe("Δx 0.0 px  Δy 0.0 px");
  });

  it("says n/a for a non-finite component instead of printing NaN", () => {
    expect(formatAlignOffset([Number.NaN, 3])).toBe("Δx +3.0 px  Δy n/a");
  });
});

describe("alignMatchSummary", () => {
  it("shows the phase-correlation confidence as a peak SNR", () => {
    expect(alignMatchSummary({ confidence: 12.345, matched_stars: 0 })).toBe("SNR 12.3");
    expect(alignMatchSummary({ confidence: 3.2 })).toBe("SNR 3.2");
  });

  it("shows star matches for the affine method and never a constant confidence", () => {
    expect(alignMatchSummary({ confidence: 1, matched_stars: 40, inliers: 35, residual_px: 0.412 })).toBe("35/40 stars, 0.41 px");
    expect(alignMatchSummary({ confidence: 1, matched_stars: 8, inliers: 6 })).toBe("6/8 stars");
  });

  it("shows nothing for the reference or an identity fallback without confidence", () => {
    expect(alignMatchSummary(undefined)).toBeNull();
    expect(alignMatchSummary({})).toBeNull();
    expect(alignMatchSummary({ confidence: 0, matched_stars: 0 })).toBeNull();
    expect(alignMatchSummary({ confidence: Number.NaN, matched_stars: 0 })).toBeNull();
  });
});

describe("Align overlay channels", () => {
  it("overlays every channel when there are at most three", () => {
    expect(alignOverlayBinIds(["ha", "oiii"])).toEqual(["ha", "oiii"]);
    expect(alignOverlayBinIds(["ha", "oiii", "sii"], ["sii"])).toEqual(["ha", "oiii", "sii"]);
  });

  it("keeps the reference and takes the next two channels in bin order by default", () => {
    expect(alignOverlayBinIds(["r", "g", "b", "l"])).toEqual(["r", "g", "b"]);
    expect(alignOverlayBinIds(["ha", "oiii", "sii", "r", "g"])).toEqual(["ha", "oiii", "sii"]);
  });

  it("uses the chosen channels in the order given, ignoring the reference, duplicates and unknown ids", () => {
    expect(alignOverlayBinIds(["r", "g", "b", "l"], ["l", "g"])).toEqual(["r", "l", "g"]);
    expect(alignOverlayBinIds(["r", "g", "b", "l"], ["r", "l", "l", "x"])).toEqual(["r", "l", "g"]);
  });

  it("builds a run only when every channel Align used has an aligned key", () => {
    const channels = [{ binId: "ha", path: "/s/ha.fits" }, { binId: "oiii", path: "/raw/o_1.fits" }];
    const aligned = { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" };
    expect(alignedRunFromChannels(channels, aligned)).toEqual({
      binIds: ["ha", "oiii"],
      aligned,
      inputs: { ha: "/s/ha.fits", oiii: "/raw/o_1.fits" },
    });
    expect(alignedRunFromChannels(channels, { ha: aligned.ha })).toBeNull();
    expect(alignedRunFromChannels(channels.slice(0, 1), aligned)).toBeNull();
  });

  it("compares the aligned keys with the exact inputs Align used, channel for channel", () => {
    const binIds = ["r", "g", "b", "l"];
    const run = {
      binIds,
      aligned: Object.fromEntries(binIds.map((id) => [id, `__wizard_ch_${id}_aligned`])),
      inputs: Object.fromEntries(binIds.map((id) => [id, `/in/${id}.fits`])),
    };
    expect(alignOverlayRequest(run)).toEqual({
      binIds: ["r", "g", "b"],
      afterKeys: ["__wizard_ch_r_aligned", "__wizard_ch_g_aligned", "__wizard_ch_b_aligned"],
      beforePaths: ["/in/r.fits", "/in/g.fits", "/in/b.fits"],
    });
    expect(alignOverlayRequest(run, ["l", "b"])?.beforePaths).toEqual(["/in/r.fits", "/in/l.fits", "/in/b.fits"]);
    expect(alignOverlayRequest({ ...run, inputs: { r: "/in/r.fits" } })).toBeNull();
  });
});

describe("Align overlay legend", () => {
  it("names the colour of each channel as the overlay renders it", () => {
    expect(alignOverlayColours(["Hα"])).toBe("grey = Hα (ref)");
    expect(alignOverlayColours(["Hα", "OIII"])).toBe("R = Hα (ref) · G+B = OIII");
    expect(alignOverlayColours(["Hα", "OIII", "SII"])).toBe("R = Hα (ref) · G = OIII · B = SII");
  });

  it("explains what white, colour fringes and the checkerboard mean after Align", () => {
    expect(alignPreviewLegend(["Hα", "OIII"], "after")).toBe(
      "R = Hα (ref) · G+B = OIII · white = aligned, coloured fringes = residual offset, checkerboard = no data",
    );
  });

  it("calls the fringes of the inputs the original offset", () => {
    expect(alignPreviewLegend(["R", "G", "B"], "before")).toBe(
      "R = R (ref) · G = G · B = B · inputs before Align: coloured fringes = original offset, checkerboard = no data",
    );
  });

  it("names the two blinked frames, clamping the chosen channel to the overlay", () => {
    expect(alignPreviewLegend(["Hα", "OIII", "SII"], "blink", 2)).toBe(
      "blinking Hα (ref) and SII after Align · stars that jump = residual offset · checkerboard = no data",
    );
    expect(alignPreviewLegend(["Hα", "OIII"], "blink", 5)).toContain("blinking Hα (ref) and OIII");
    expect(alignPreviewLegend(["Hα", "OIII"], "blink", 0)).toContain("and OIII");
  });
});

describe("Aligned run follows the wizard state", () => {
  const channels = [{ binId: "ha", path: "/s/ha.fits" }, { binId: "oiii", path: "/raw/o_1.fits" }];
  const aligned = { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" };

  it("keeps the same run object while neither the aligned keys nor the inputs change", () => {
    const first = nextAlignedRunState(null, channels, aligned, false);
    expect(first.run?.binIds).toEqual(["ha", "oiii"]);
    expect(nextAlignedRunState(first, channels.map((c) => ({ ...c })), aligned, false)).toBe(first);
  });

  it("starts a new run when Align stores its keys again under the same names", () => {
    const first = nextAlignedRunState(null, channels, aligned, false);
    const again = nextAlignedRunState(first, channels, { ...aligned }, false);
    expect(again).not.toBe(first);
    expect(again.run).not.toBe(first.run);
    expect(again.run).toEqual(first.run);
  });

  it("hides the run while Align is running and shows the stored keys once it settles", () => {
    const first = nextAlignedRunState(null, channels, aligned, false);
    const running = nextAlignedRunState(first, channels, aligned, true);
    expect(running.run).toBeNull();
    const settled = nextAlignedRunState(running, channels, { ...aligned }, false);
    expect(settled.run?.aligned).toEqual(aligned);
  });

  it("drops the run when a new stack clears the aligned keys and follows a changed input", () => {
    const first = nextAlignedRunState(null, channels, aligned, false);
    expect(nextAlignedRunState(first, channels, {}, false).run).toBeNull();
    const restacked = [{ binId: "ha", path: "/s/ha_2.fits" }, channels[1]];
    expect(nextAlignedRunState(first, restacked, aligned, false).run?.inputs.ha).toBe("/s/ha_2.fits");
  });

  it("compares runs by bins, keys and inputs", () => {
    const run = { binIds: ["ha", "oiii"], aligned, inputs: { ha: "/a", oiii: "/b" } };
    expect(sameAlignedRun(run, { ...run, aligned: { ...aligned } })).toBe(true);
    expect(sameAlignedRun(run, { ...run, inputs: { ha: "/a", oiii: "/c" } })).toBe(false);
    expect(sameAlignedRun(run, { ...run, binIds: ["oiii", "ha"] })).toBe(false);
    expect(sameAlignedRun(null, null)).toBe(true);
    expect(sameAlignedRun(run, null)).toBe(false);
  });
});

describe("Align preview frame", () => {
  const names = { ha: "Hα", oiii: "OIII", sii: "SII" };
  const after = { previewUrl: "after.png", frameUrls: ["f0.png", "f1.png", "f2.png"], binIds: ["ha", "oiii", "sii"] };
  const before = { previewUrl: "before.png", frameUrls: [], binIds: ["ha", "sii"] };
  const still = { on: false, showOther: false, index: 1 };

  it("shows the overlay of the chosen view with the labels of the request that produced it", () => {
    expect(alignPreviewFrame("after", still, after, before, names)).toMatchObject({
      view: "after", src: "after.png", label: "After Align", labels: ["Hα", "OIII", "SII"],
    });
    expect(alignPreviewFrame("before", still, after, before, names)).toMatchObject({
      view: "before", src: "before.png", label: "Before Align", labels: ["Hα", "SII"],
    });
  });

  it("pairs blink frame i with bin i and alternates it with the reference", () => {
    expect(alignPreviewFrame("after", { on: true, showOther: true, index: 2 }, after, before, names)).toMatchObject({
      view: "blink", src: "f2.png", label: "SII", blinkIndex: 2,
    });
    expect(alignPreviewFrame("before", { on: true, showOther: false, index: 2 }, after, before, names)).toMatchObject({
      view: "blink", src: "f0.png", label: "Hα (ref)",
    });
  });

  it("clamps the blinked channel to the frames that have a bin", () => {
    const twoBins = { ...after, binIds: ["ha", "oiii"] };
    expect(alignPreviewFrame("after", { on: true, showOther: true, index: 5 }, twoBins, null, names)).toMatchObject({
      src: "f1.png", label: "OIII", blinkIndex: 1,
    });
    expect(alignPreviewFrame("after", { on: true, showOther: true, index: 0 }, after, null, names).blinkIndex).toBe(1);
  });

  it("does not blink without two frames with a bin and shows nothing before a render arrives", () => {
    const single = { ...after, frameUrls: ["f0.png"] };
    expect(alignPreviewFrame("after", { on: true, showOther: true, index: 1 }, single, null, names)).toMatchObject({
      view: "after", src: "after.png", canBlink: false,
    });
    expect(alignPreviewFrame("before", still, after, null, names)).toMatchObject({ src: "", labels: [] });
  });

  it("names a bin without a label by its id", () => {
    expect(alignPreviewFrame("after", { on: true, showOther: true, index: 1 }, after, null, { ha: "Hα" }).label).toBe("oiii");
  });
});

describe("Align blink frames", () => {
  const frames = ["f0.png", "f1.png", "f2.png"];

  it("pairs frame i with the bin id at index i of the request that produced the frames", () => {
    expect(alignBlinkFrame(frames, ["ha", "oiii", "sii"], 1, true)).toEqual({ src: "f1.png", binId: "oiii", index: 1 });
    expect(alignBlinkFrame(frames, ["ha", "oiii", "sii"], 2, true)).toEqual({ src: "f2.png", binId: "sii", index: 2 });
    expect(alignBlinkFrame(frames, ["r", "l", "b"], 2, true)).toEqual({ src: "f2.png", binId: "b", index: 2 });
  });

  it("shows the reference frame between the blinks whatever channel is chosen", () => {
    expect(alignBlinkFrame(frames, ["ha", "oiii", "sii"], 2, false)).toEqual({ src: "f0.png", binId: "ha", index: 0 });
  });

  it("only pairs the frames that have a bin and clamps the choice to them", () => {
    expect(alignBlinkFrame(frames, ["ha", "oiii"], 2, true)).toEqual({ src: "f1.png", binId: "oiii", index: 1 });
    expect(alignBlinkFrame(["f0.png", "f1.png"], ["ha", "oiii", "sii"], 2, true)).toEqual({ src: "f1.png", binId: "oiii", index: 1 });
    expect(alignBlinkIndex(0, 3)).toBe(1);
    expect(alignBlinkIndex(7, 3)).toBe(2);
  });

  it("has nothing to blink with fewer than two pairs", () => {
    expect(alignBlinkFrame(["f0.png"], ["ha", "oiii"], 1, true)).toBeNull();
    expect(alignBlinkFrame(frames, ["ha"], 1, true)).toBeNull();
    expect(alignBlinkFrame([], [], 1, false)).toBeNull();
  });
});

describe("Align viewer keeps the last image", () => {
  const names = { ha: "Hα", oiii: "OIII" };
  const afterShown = { previewUrl: "after.png", frameUrls: ["f0.png", "f1.png"], binIds: ["ha", "oiii"] };
  const beforeShown = { previewUrl: "before.png", frameUrls: [], binIds: ["ha", "oiii"] };
  const still = { on: false, showOther: false, index: 1 };
  const ready = { hasRequest: true, loading: false, error: "", brokenSrc: "" };
  const afterFrame = alignPreviewFrame("after", still, afterShown, beforeShown, names);

  it("shows the requested image and describes it once it has loaded", () => {
    const viewer = alignViewerState(afterFrame, null, ready);
    expect(viewer).toMatchObject({ src: "after.png", pending: false, error: "" });
    expect(viewer.shown?.label).toBe("After Align");
    expect(alignDisplayedOnLoad("after.png", afterFrame, null)).toBe(afterFrame);
  });

  it("keeps the shown image mounted under a spinner while the requested view has not rendered yet", () => {
    const beforePending = alignPreviewFrame("before", still, afterShown, null, names);
    const viewer = alignViewerState(beforePending, afterFrame, { ...ready, loading: true });
    expect(viewer).toMatchObject({ src: "after.png", pending: true, error: "" });
    expect(viewer.shown?.label).toBe("After Align");
  });

  it("names the image on screen, not the requested one, until the requested image has loaded", () => {
    const beforeFrame = alignPreviewFrame("before", still, afterShown, beforeShown, names);
    const switching = alignViewerState(beforeFrame, afterFrame, ready);
    expect(switching.src).toBe("before.png");
    expect(switching.shown?.label).toBe("After Align");
    const loaded = alignDisplayedOnLoad("before.png", beforeFrame, afterFrame);
    expect(alignViewerState(beforeFrame, loaded, ready).shown?.label).toBe("Before Align");
  });

  it("ignores a load event of an image that is no longer requested", () => {
    const beforeFrame = alignPreviewFrame("before", still, afterShown, beforeShown, names);
    expect(alignDisplayedOnLoad("after.png", beforeFrame, afterFrame)).toBe(afterFrame);
    expect(alignDisplayedOnLoad("", beforeFrame, null)).toBeNull();
  });

  it("follows a blink frame with the label and legend of the same frame", () => {
    const blinkOther = alignPreviewFrame("after", { on: true, showOther: true, index: 1 }, afterShown, null, names);
    const shownRef = alignPreviewFrame("after", { on: true, showOther: false, index: 1 }, afterShown, null, names);
    const viewer = alignViewerState(blinkOther, shownRef, ready);
    expect(viewer.src).toBe("f1.png");
    expect(viewer.shown?.label).toBe("Hα (ref)");
    const loaded = alignDisplayedOnLoad("f1.png", blinkOther, shownRef);
    expect(alignViewerState(blinkOther, loaded, ready).shown).toMatchObject({ label: "OIII", view: "blink", blinkIndex: 1 });
  });

  it("keeps the old image of the same view under a spinner while a new render is pending", () => {
    expect(alignViewerState(afterFrame, afterFrame, { ...ready, loading: true })).toMatchObject({
      src: "after.png", pending: true,
    });
  });

  it("shows the error of the requested view instead of an older image", () => {
    const beforeFailed = alignPreviewFrame("before", still, afterShown, null, names);
    expect(alignViewerState(beforeFailed, afterFrame, { ...ready, error: "boom" })).toEqual({
      src: "", pending: false, shown: null, error: "boom",
    });
  });

  it("reports an image that failed to load instead of naming it", () => {
    expect(alignViewerState(afterFrame, null, { ...ready, brokenSrc: "after.png" })).toEqual({
      src: "", pending: false, shown: null, error: ALIGN_IMAGE_LOAD_ERROR,
    });
  });

  it("clears the viewer when there is nothing to render, before the first image or while Align runs", () => {
    const empty = alignPreviewFrame("after", still, null, null, names);
    expect(alignViewerState(empty, null, { ...ready, loading: true })).toEqual({
      src: "", pending: false, shown: null, error: "",
    });
    expect(alignViewerState(afterFrame, afterFrame, { ...ready, hasRequest: false })).toEqual({
      src: "", pending: false, shown: null, error: "",
    });
  });
});

describe("Align run across remounts of the step", () => {
  const channels = [{ binId: "ha", path: "/s/ha.fits" }, { binId: "oiii", path: "/s/oiii.fits" }];
  const inputs = channels.map((c) => c.path);
  const aligned = { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" };
  const result = { offsets: [[0, 0], [3, -2]] };

  it("hides the stored keys from a step mounted while Align is still running", () => {
    const remounted = nextAlignedRunState(null, channels, aligned, true);
    expect(remounted.run).toBeNull();
    const settled = nextAlignedRunState(remounted, channels, { ...aligned }, false);
    expect(settled.run?.aligned).toEqual(aligned);
  });

  it("shows no outcome while the run is in flight", () => {
    expect(alignRunOutcome({ running: true, inputs, result: null, error: "" }, inputs, true)).toEqual({
      result: null, error: "",
    });
    expect(alignRunOutcome(null, inputs, true)).toEqual({ result: null, error: "" });
  });

  it("gives a remounted step the outcome of the run that finished meanwhile", () => {
    expect(alignRunOutcome({ running: false, inputs, result, error: "" }, inputs, true)).toEqual({ result, error: "" });
    expect(alignRunOutcome({ running: false, inputs, result: null, error: "boom" }, [...inputs], false)).toEqual({
      result: null, error: "boom",
    });
  });

  it("drops the outcome once the inputs change or the aligned keys are cleared", () => {
    expect(alignRunOutcome({ running: false, inputs, result, error: "" }, ["/s/ha_2.fits", inputs[1]], true)).toEqual({
      result: null, error: "",
    });
    expect(alignRunOutcome({ running: false, inputs, result, error: "" }, inputs.slice(0, 1), true).result).toBeNull();
    expect(alignRunOutcome({ running: false, inputs, result, error: "" }, inputs, false).result).toBeNull();
  });
});

describe("binMenuPlacement", () => {
  const viewport = { width: 1280, height: 800 };

  it("opens below the button, aligned to its left edge", () => {
    expect(binMenuPlacement({ left: 400, top: 100, bottom: 116 }, viewport)).toEqual({ left: 400, top: 120, bottom: null });
  });

  it("shifts left so a right-hand bin's menu stays inside the window", () => {
    expect(binMenuPlacement({ left: 1200, top: 100, bottom: 116 }, viewport).left).toBe(1280 - 200 - 8);
    expect(binMenuPlacement({ left: 2, top: 100, bottom: 116 }, viewport).left).toBe(8);
  });

  it("opens upwards when there is no room below", () => {
    expect(binMenuPlacement({ left: 400, top: 700, bottom: 716 }, viewport)).toEqual({ left: 400, top: null, bottom: 800 - 700 + 4 });
  });
});

describe("binMenuKeyAction", () => {
  it("closes on Escape and returns focus to the Select button, consuming the key", () => {
    expect(binMenuKeyAction("Escape", 2, 5)).toEqual({ type: "close", preventDefault: true });
    expect(binMenuKeyAction("Escape", -1, 0)).toEqual({ type: "close", preventDefault: true });
  });

  it("closes on Tab and lets the browser move focus on from the Select button", () => {
    expect(binMenuKeyAction("Tab", 0, 5)).toEqual({ type: "close", preventDefault: false });
  });

  it("moves between items with the arrow keys, wrapping at both ends", () => {
    expect(binMenuKeyAction("ArrowDown", 0, 3)).toEqual({ type: "focus", index: 1 });
    expect(binMenuKeyAction("ArrowDown", 2, 3)).toEqual({ type: "focus", index: 0 });
    expect(binMenuKeyAction("ArrowUp", 0, 3)).toEqual({ type: "focus", index: 2 });
    expect(binMenuKeyAction("ArrowUp", 2, 3)).toEqual({ type: "focus", index: 1 });
  });

  it("enters the list from the menu itself when no item has focus", () => {
    expect(binMenuKeyAction("ArrowDown", -1, 3)).toEqual({ type: "focus", index: 0 });
    expect(binMenuKeyAction("ArrowUp", -1, 3)).toEqual({ type: "focus", index: 2 });
  });

  it("jumps to the first and last item with Home and End", () => {
    expect(binMenuKeyAction("Home", 1, 4)).toEqual({ type: "focus", index: 0 });
    expect(binMenuKeyAction("End", 1, 4)).toEqual({ type: "focus", index: 3 });
  });

  it("ignores navigation in an empty menu and leaves other keys to the item", () => {
    expect(binMenuKeyAction("ArrowDown", -1, 0)).toBeNull();
    expect(binMenuKeyAction("End", -1, 0)).toBeNull();
    expect(binMenuKeyAction("Enter", 1, 3)).toBeNull();
    expect(binMenuKeyAction(" ", 1, 3)).toBeNull();
  });
});

describe("exportWcsWarning", () => {
  it("keeps the header-source warning and adds one when the backend wrote no celestial WCS", () => {
    expect(exportWcsWarning({ wcs_written: false }, null)).toBe(WCS_MISSING_WARNING);
    expect(exportWcsWarning({ wcs_written: false }, "header source unreadable")).toBe("header source unreadable");
    expect(exportWcsWarning({ wcs_written: true }, null)).toBeNull();
    expect(exportWcsWarning({ output_path: "/x.fits" }, null)).toBeNull();
  });
});

describe("export and Auto STF requirements", () => {
  it("blocks Export while no channel path resolves", () => {
    expect(exportBlockedReason(stateWith({}))).toBe("Assign at least one channel in step 1");
    expect(exportBlockedReason(stateWith({ ha: ["/h.fits"] }))).toBeNull();
    expect(exportBlockedReason(stateWith({}, { compositeReady: true }))).toBeNull();
  });

  it("explains that Auto STF needs a blended composite", () => {
    expect(autoStfBlockedReason(stateWith({ ha: ["/h.fits"] }))).toBe(
      "Auto STF needs a blended composite, and Blend needs at least 2 channels.",
    );
    expect(autoStfBlockedReason(stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }))).toBe(
      "Auto STF needs a blended composite: run Blend first.",
    );
    expect(autoStfBlockedReason(stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }, { compositeReady: true }))).toBeNull();
  });
});
