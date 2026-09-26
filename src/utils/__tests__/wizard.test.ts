import { describe, it, expect } from "vitest";
import {
  alignChannelOutcome,
  applyCompositeOp,
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

  it("keeps Export locked until a channel is assigned or a composite exists", () => {
    expect(stepById("export").enabled(empty)).toBe(false);
    expect(stepById("export").enabled(oneFrame)).toBe(true);
    expect(stepById("export").enabled({ ...empty, compositeReady: true })).toBe(true);
  });

  it("gives a reason exactly when a step is locked", () => {
    for (const s of [empty, oneFrame, oneChannelStack, twoChannels, aligned, blended]) {
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

  it("suggests Crop after the first Align when the suggestion reads the state that holds the aligned paths", () => {
    expect(nextEnabledStep("align", twoChannels)).toBe("background");
    expect(nextEnabledStep("align", aligned)).toBe("crop");
    expect(nextEnabledStep("blend", blended)).toBe("colorbalance");
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
