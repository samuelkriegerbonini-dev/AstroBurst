import { describe, it, expect } from "vitest";
import {
  blendRequest,
  channelLevel,
  channelLevels,
  levelMatchDefault,
  levelMatchEnabled,
  levelMatchEntries,
  levelMatchSummary,
  levelMeasureError,
  levelScales,
  levelTargets,
  scaleBlendWeights,
  SPCC_LEVEL_NOTE,
  spccFactorsForLevels,
  spccLevelNote,
  withSpccWb,
  type BackendBlendWeight,
} from "../levelMatch";
import { BLEND_PRESETS, INITIAL_STATE, spccInputs, type BlendWeight, type WizardState } from "../wizard";
import { blendMatrixError } from "../blendWeights";
import { WB_APPLY_MAX, WB_APPLY_MIN, wbFactorsOutOfRange } from "../whiteBalanceRange";
import type { ChannelSource } from "../channelMapping";

function stateWith(files: Record<string, string[]>, extra: Partial<WizardState> = {}): WizardState {
  return {
    ...INITIAL_STATE,
    bins: INITIAL_STATE.bins.map((b) => ({ ...b, files: files[b.id] ?? [] })),
    ...extra,
  };
}

const frame = (path: string, header: Record<string, string> = {}): ChannelSource & { path: string } => ({
  path,
  name: path.split("/").pop(),
  result: { header },
});

const filled = (s: WizardState) => s.bins.filter((b) => b.files.length > 0);

describe("level scales", () => {
  it("levelScales gives k = max / level", () => {
    expect(levelScales([
      { binId: "a", label: "a", level: 1 },
      { binId: "b", label: "b", level: 20 },
    ])).toEqual({ scales: { a: 20, b: 1 } });
  });

  it("levelScales refuses a channel without signal", () => {
    for (const bad of [0, -3, Number.NaN]) {
      const outcome = levelScales([
        { binId: "r", label: "r F200W", level: 2 },
        { binId: "g", label: "g F187N", level: bad },
        { binId: "b", label: "b F090W", level: Number.POSITIVE_INFINITY },
      ]);
      expect("error" in outcome).toBe(true);
      const error = "error" in outcome ? outcome.error : "";
      expect(error).toContain("g F187N");
      expect(error).toContain("Turn Match levels off");
      expect(error).not.toContain("b F090W");
    }
  });

  it("channelLevel is p99.5 minus p50", () => {
    expect(channelLevel({ vmin: 0.1, vmax: 2.6 })).toBeCloseTo(2.5, 12);
  });

  it("scaled weights are not re-rounded", () => {
    const weights: BackendBlendWeight[] = [{ channelIdx: 0, r: 0, g: 0.33, b: 0 }];
    const scaled = scaleBlendWeights(weights, ["a"], { a: 20 });
    expect(scaled[0].g).toBe(0.33 * 20);
    expect(scaled[0].g).not.toBe(6.6);
    expect(scaled[0]).toEqual({ channelIdx: 0, r: 0, g: 0.33 * 20, b: 0 });
    const sho: BackendBlendWeight[] = [
      { channelIdx: 0, r: 0.7, g: 0.3, b: 0 },
      { channelIdx: 1, r: 0, g: 0.15, b: 0.85 },
    ];
    const up = scaleBlendWeights(sho, ["sii", "oiii"], { sii: 1, oiii: 7.5 });
    up.forEach((w, i) => {
      expect(w.r).toBeGreaterThanOrEqual(sho[i].r);
      expect(w.g).toBeGreaterThanOrEqual(sho[i].g);
      expect(w.b).toBeGreaterThanOrEqual(sho[i].b);
    });
    expect(blendMatrixError(up, ["SII", "OIII"])).toBeNull();
  });

  it("no scales leaves the payload unchanged", () => {
    const weights: BackendBlendWeight[] = [
      { channelIdx: 2, r: 1, g: 0, b: 0 },
      { channelIdx: 0, r: 0, g: 0.35, b: 0.5 },
    ];
    const same = scaleBlendWeights(weights, ["ha", "oiii", "sii"], null);
    expect(same).toEqual(weights);
    expect(same).not.toBe(weights);
  });

  it("a channel without a scale keeps its weights", () => {
    const weights: BackendBlendWeight[] = [{ channelIdx: 1, r: 0.5, g: 0, b: 0 }];
    expect(scaleBlendWeights(weights, ["a", "b"], { a: 3 })).toEqual(weights);
  });
});

describe("blend request", () => {
  it("blendRequest matches today's payload", () => {
    const s = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"], sii: ["/s.fits"] });
    const active: BlendWeight[] = [
      ...BLEND_PRESETS.sho.weights,
      { channelId: "r", r: 0, g: 0, b: 0 },
      { channelId: "l", r: 0.5, g: 0, b: 0 },
    ];
    expect(blendRequest(s, filled(s), active)).toEqual({
      channelOrder: ["ha", "oiii", "sii"],
      paths: ["/h.fits", "/o.fits", "/s.fits"],
      weights: [
        { channelIdx: 2, r: 1, g: 0, b: 0 },
        { channelIdx: 0, r: 0, g: 1, b: 0 },
        { channelIdx: 1, r: 0, g: 0, b: 1 },
      ],
    });
  });

  it("blendRequest uses the latest stage of each channel", () => {
    const s = stateWith({ ha: ["/h.fits"], oiii: ["/o.fits"] }, {
      alignedPaths: { ha: "__wizard_ch_ha_aligned", oiii: "__wizard_ch_oiii_aligned" },
      backgroundPaths: { oiii: "/out/oiii_bg.fits" },
    });
    const active: BlendWeight[] = [
      { channelId: "ha", r: 1, g: 0, b: 0 },
      { channelId: "oiii", r: 0, g: 0.5, b: 0.5 },
    ];
    expect(blendRequest(s, filled(s), active).paths).toEqual(["__wizard_ch_ha_aligned", "/out/oiii_bg.fits"]);
  });

  it("levelTargets measures each used path once", () => {
    const s = stateWith({ ha: ["/x.fits"], oiii: ["/x.fits"], sii: ["/s.fits"], r: ["/r.fits"] });
    const active: BlendWeight[] = [
      { channelId: "sii", r: 1, g: 0, b: 0 },
      { channelId: "ha", r: 0, g: 1, b: 0 },
      { channelId: "oiii", r: 0, g: 0, b: 1 },
      { channelId: "r", r: 0, g: 0, b: 0 },
    ];
    const request = blendRequest(s, filled(s), active);
    expect(request.channelOrder).toEqual(["ha", "oiii", "sii", "r"]);
    expect(levelTargets(request)).toEqual([
      { binId: "ha", path: "/x.fits" },
      { binId: "sii", path: "/s.fits" },
    ]);
    expect(channelLevels(request, { "/x.fits": 2, "/s.fits": 4 }, (id) => `${id}!`)).toEqual([
      { binId: "ha", label: "ha!", level: 2 },
      { binId: "oiii", label: "oiii!", level: 2 },
      { binId: "sii", label: "sii!", level: 4 },
    ]);
  });

  it("a used channel without a measured level is refused", () => {
    const request = { channelOrder: ["a", "b"], paths: ["/a", "/b"], weights: [{ channelIdx: 0, r: 1, g: 0, b: 0 }, { channelIdx: 1, r: 0, g: 1, b: 1 }] };
    const outcome = levelScales(channelLevels(request, { "/a": 3 }, (id) => id));
    expect("error" in outcome && outcome.error.startsWith("Match levels needs signal above the sky in every channel: b ")).toBe(true);
  });
});

describe("Match levels default", () => {
  const nircam = (codes: [string, string, string]) => {
    const [b, g, r] = codes.map((c) => `/n/${c.toLowerCase()}.fits`);
    return {
      state: stateWith({ b: [b], g: [g], r: [r] }),
      files: [frame(b, { FILTER: codes[0] }), frame(g, { FILTER: codes[1] }), frame(r, { FILTER: codes[2] })],
    };
  };

  it("default is on with a narrowband filter", () => {
    const withNarrow = nircam(["F090W", "F187N", "F200W"]);
    expect(levelMatchDefault(withNarrow.state, withNarrow.files)).toBe(true);

    const sho = stateWith({ ha: ["/d/656.fits"], oiii: ["/d/502.fits"], sii: ["/d/673.fits"] });
    const shoFiles = [
      frame("/d/656.fits", { FILTNAM1: "F656N" }),
      frame("/d/502.fits", { FILTNAM1: "F502N" }),
      frame("/d/673.fits", { FILTNAM1: "F673N" }),
    ];
    expect(levelMatchDefault(sho, shoFiles)).toBe(true);

    const broad = nircam(["F090W", "F150W", "F200W"]);
    expect(levelMatchDefault(broad.state, broad.files)).toBe(false);

    const amateur = stateWith({ r: ["/a/red.fits"], g: ["/a/green.fits"], b: ["/a/blue.fits"] });
    const amateurFiles = [
      frame("/a/red.fits", { FILTER: "Red" }),
      frame("/a/green.fits", { FILTER: "Green" }),
      frame("/a/blue.fits", { FILTER: "Blue" }),
    ];
    expect(levelMatchDefault(amateur, amateurFiles)).toBe(false);

    expect(levelMatchDefault({ ...broad.state, blendPreset: "sho" }, broad.files)).toBe(false);
  });

  it("reads narrowband detections and PUPIL tokens", () => {
    const wfpc2 = stateWith({ r: ["/d/673nmos.fits"], g: ["/d/656nmos.fits"], b: ["/d/502nmos.fits"] });
    const wheel = [
      frame("/d/673nmos.fits", { FILTER: "33" }),
      frame("/d/656nmos.fits", { FILTER: "31" }),
      frame("/d/502nmos.fits", { FILTER: "23" }),
    ];
    expect(levelMatchDefault(wfpc2, wheel)).toBe(false);
    expect(levelMatchDefault(wfpc2, wheel, [{ path: "/d/656nmos.fits", filter: "Hα (656nm)" }])).toBe(true);

    const pupil = "/n/jw_f444w-f470n_i2d.fits";
    const s = stateWith({ r: [pupil], g: ["/n/f200w.fits"], b: ["/n/f090w.fits"] });
    const files = [
      frame(pupil, { FILTER: "F444W", PUPIL: "F470N" }),
      frame("/n/f200w.fits", { FILTER: "F200W" }),
      frame("/n/f090w.fits", { FILTER: "F090W" }),
    ];
    expect(levelMatchDefault(s, files)).toBe(true);
  });

  it("explicit toggle wins", () => {
    const withNarrow = nircam(["F090W", "F187N", "F200W"]);
    expect(levelMatchEnabled(withNarrow.state, withNarrow.files)).toBe(true);
    expect(levelMatchEnabled({ ...withNarrow.state, levelMatch: false }, withNarrow.files)).toBe(false);
    const broad = nircam(["F090W", "F150W", "F200W"]);
    expect(levelMatchEnabled(broad.state, broad.files)).toBe(false);
    expect(levelMatchEnabled({ ...broad.state, levelMatch: true }, broad.files)).toBe(true);
  });
});

describe("Match levels labels and messages", () => {
  const s = stateWith({ b: ["/n/f090w.fits"], g: ["/n/f187n.fits"], r: ["/n/f200w.fits"] });
  const files = [
    frame("/n/f090w.fits", { FILTER: "F090W" }),
    frame("/n/f187n.fits", { FILTER: "F187N" }),
    frame("/n/f200w.fits", { FILTER: "F200W" }),
  ];

  it("labels use bin id and filter code", () => {
    expect(levelMatchEntries(s, files, { b: 20, g: 1, r: 20 / 1.2 })).toEqual([
      { channel: "r F200W", scale: 20 / 1.2 },
      { channel: "g F187N", scale: 1 },
      { channel: "b F090W", scale: 20 },
    ]);
    const amateur = stateWith({ r: ["/a/red.fits"], g: ["/a/green.fits"] });
    expect(levelMatchEntries(amateur, [frame("/a/red.fits", { FILTER: "Red" })], { r: 2 })).toEqual([
      { channel: "r", scale: 2 },
    ]);
  });

  it("summarises the scales in one line", () => {
    expect(levelMatchSummary(levelMatchEntries(s, files, { b: 20, g: 1, r: 20 / 1.2 }))).toBe(
      "Level match: r F200W x16.67, g F187N x1.000, b F090W x20.00",
    );
  });

  it("levelMeasureError", () => {
    expect(levelMeasureError("g F187N", "__wizard_ch_g_aligned", "not in cache")).toBe(
      "g F187N is no longer in memory; run Align again (or the Crop or BG step that produced it).",
    );
    expect(levelMeasureError("X", "/n/f187n.fits", "msg")).toBe("Measuring levels for X failed: msg");
  });
});

describe("SPCC factors after Match levels", () => {
  it("SPCC factor is divided by the scale of its input channel", () => {
    const s = stateWith({ r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] });
    const out = spccFactorsForLevels({ r: 1.2, g: 1, b: 0.9 }, spccInputs(s), { r: 1, g: 1, b: 4 });
    expect(out.r).toBeCloseTo(1.2, 12);
    expect(out.g).toBe(1);
    expect(out.b).toBeCloseTo(0.225, 12);
    expect(spccFactorsForLevels({ r: 1.2, g: 1, b: 0.9 }, spccInputs(s), null)).toEqual({ r: 1.2, g: 1, b: 0.9 });
  });

  it("a plane without an input keeps its factor", () => {
    const s = stateWith({ r: ["/r.fits"], g: ["/g.fits"] });
    expect(spccFactorsForLevels({ r: 2, g: 1, b: 0.5 }, spccInputs(s), { r: 4, g: 1 })).toEqual({ r: 0.5, g: 1, b: 0.5 });
  });

  const rgb = { r: ["/r.fits"], g: ["/g.fits"], b: ["/b.fits"] };
  const wbOf = (s: WizardState) => [s.wbR, s.wbG, s.wbB];

  it("withSpccWb divides the stored SPCC solution by the scales of the current Blend", () => {
    const solved = withSpccWb(stateWith(rgb, { wbMode: "spcc", spccFactors: { r: 1.1, g: 1, b: 0.9 } }));
    expect(wbOf(solved)).toEqual([1.1, 1, 0.9]);
    const matched = withSpccWb({ ...solved, blendLevelScales: { r: 1, g: 1.4, b: 2.1 } });
    expect(matched.wbR).toBe(1.1);
    expect(matched.wbG).toBeCloseTo(1 / 1.4, 12);
    expect(matched.wbB).toBeCloseTo(0.9 / 2.1, 12);
    expect(matched.spccFactors).toEqual({ r: 1.1, g: 1, b: 0.9 });
    expect(wbOf(withSpccWb({ ...matched, blendLevelScales: null }))).toEqual([1.1, 1, 0.9]);
  });

  it("withSpccWb returns the same state when nothing has to change", () => {
    const factors = { r: 1.1, g: 1, b: 0.9 };
    const base = stateWith(rgb, { wbR: 1.3, wbG: 1, wbB: 0.7, blendLevelScales: { r: 2, g: 1, b: 1 } });
    const manual = { ...base, wbMode: "manual" as const, spccFactors: factors };
    expect(withSpccWb(manual)).toBe(manual);
    const unsolved = { ...base, wbMode: "spcc" as const };
    expect(withSpccWb(unsolved)).toBe(unsolved);
    const synced = stateWith(rgb, { wbMode: "spcc", spccFactors: factors, wbR: 1.1, wbG: 1, wbB: 0.9 });
    expect(withSpccWb(synced)).toBe(synced);
  });

  const scalesOf = (levels: Record<"r" | "g" | "b", number>) => {
    const outcome = levelScales((["r", "g", "b"] as const).map((binId) => ({ binId, label: binId, level: levels[binId] })));
    if ("error" in outcome) throw new Error(outcome.error);
    return outcome.scales;
  };
  const inApplyRange = (f: { r: number; g: number; b: number }) => {
    expect(wbFactorsOutOfRange(f.r, f.g, f.b)).toEqual([]);
    expect(Math.min(f.r, f.g, f.b)).toBeGreaterThanOrEqual(WB_APPLY_MIN);
    expect(Math.max(f.r, f.g, f.b)).toBeLessThanOrEqual(WB_APPLY_MAX);
  };

  it("SPCC factors stay inside the apply range at a 1:200 level ratio and keep their colour ratios", () => {
    const inputs = spccInputs(stateWith(rgb));
    const solved = { r: 1.2, g: 1, b: 0.9 };

    const blueFaint = scalesOf({ r: 200, g: 200, b: 1 });
    expect(blueFaint).toEqual({ r: 1, g: 1, b: 200 });
    const b = spccFactorsForLevels(solved, inputs, blueFaint);
    inApplyRange(b);
    expect(b.r / b.g).toBeCloseTo(1.2, 12);
    expect(b.b / b.g).toBeCloseTo(0.9 / 200, 12);

    const greenFaint = scalesOf({ r: 200, g: 1, b: 200 });
    expect(greenFaint).toEqual({ r: 1, g: 200, b: 1 });
    const g = spccFactorsForLevels(solved, inputs, greenFaint);
    inApplyRange(g);
    expect(g.r / g.g).toBeCloseTo(1.2 * 200, 9);
    expect(g.b / g.g).toBeCloseTo(0.9 * 200, 9);
  });

  it("withSpccWb applies factors inside the apply range at a 1:200 level ratio", () => {
    const solved = stateWith(rgb, { wbMode: "spcc", spccFactors: { r: 1.2, g: 1, b: 0.9 } });
    const matched = withSpccWb({ ...solved, blendLevelScales: scalesOf({ r: 200, g: 200, b: 1 }) });
    inApplyRange({ r: matched.wbR, g: matched.wbG, b: matched.wbB });
    expect(matched.wbB / matched.wbG).toBeCloseTo(0.9 / 200, 12);
    expect(matched.spccFactors).toEqual({ r: 1.2, g: 1, b: 0.9 });
  });

  it("keeps the colour ratios when no common scale fits the apply range", () => {
    const out = spccFactorsForLevels({ r: 1.2, g: 1, b: 0.9 }, spccInputs(stateWith(rgb)), { r: 1, g: 1, b: 1e5 });
    expect(out.r / out.g).toBeCloseTo(1.2, 12);
    expect(out.b / out.g).toBeCloseTo(0.9e-5, 15);
    expect(wbFactorsOutOfRange(out.r, out.g, out.b)).not.toEqual([]);
  });

  it("the SPCC note names the common rescale when the factors had to be brought into range", () => {
    const solved = stateWith(rgb, { wbMode: "spcc", spccFactors: { r: 1.2, g: 1, b: 0.9 } });
    expect(spccLevelNote(solved)).toBeNull();
    expect(spccLevelNote({ ...solved, blendLevelScales: { r: 1, g: 1.4, b: 2.1 } })).toBe(SPCC_LEVEL_NOTE);
    expect(SPCC_LEVEL_NOTE).toBe("SPCC factors are divided by the Blend level-match scales.");
    expect(spccLevelNote({ ...solved, blendLevelScales: scalesOf({ r: 200, g: 200, b: 1 }) })).toBe(
      "SPCC factors are divided by the Blend level-match scales, then all three are scaled x13.61 to stay within [0.01, 100]; the colour ratios are unchanged.",
    );
    expect(spccLevelNote({ ...solved, wbMode: "manual", blendLevelScales: { r: 1, g: 1, b: 200 } })).toBeNull();
    expect(spccLevelNote({ ...solved, spccFactors: null, blendLevelScales: { r: 1, g: 1, b: 200 } })).toBeNull();
  });
});
