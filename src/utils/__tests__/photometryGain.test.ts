import { describe, expect, it } from "vitest";
import { GAIN_TITLE, gainCaption, gainPrefillText, nextGainText } from "../photometryGain";
import type { GainModel } from "../../services/analysis";

function model(overrides: Partial<GainModel> = {}): GainModel {
  return {
    gain_e_per_adu: null,
    source: null,
    gain_card: null,
    unit_class: "counts",
    ncombine: null,
    combine_method: null,
    drizzle_scale: null,
    combine_scaled: false,
    effective_gain: null,
    fallback_gain: null,
    poisson_route: "unavailable",
    note: "no gain in the header",
    ...overrides,
  };
}

function headerGain(gain: number, overrides: Partial<GainModel> = {}): GainModel {
  return model({
    gain_e_per_adu: gain,
    source: "EGAIN",
    effective_gain: gain,
    poisson_route: "header_gain",
    note: null,
    ...overrides,
  });
}

const ERR_PLANE = model({ poisson_route: "err_plane", unit_class: "calibrated", note: "Poisson noise from the ERR plane" });
const ERR_CAPTION = "Poisson noise from the ERR plane; gain not used";

const WFPC2_502NMOS = headerGain(7, {
  source: "ATODGAIN",
  ncombine: 2,
  note: "NCOMBINE=2 in the header: gain not scaled because the combine method is unknown; enter 2 × 7 = 14 e-/ADU for a mean stack",
});

describe("GAIN_TITLE", () => {
  it("is the tooltip the CDP selects the gain field by", () => {
    expect(GAIN_TITLE).toBe("Gain in e-/ADU; prefilled from the header when known");
  });
});

describe("gainPrefillText", () => {
  it("prefills nothing without a header gain route", () => {
    expect(gainPrefillText(null)).toBe("");
    expect(gainPrefillText(model({ poisson_route: "err_plane", fallback_gain: 200 }))).toBe("");
    expect(gainPrefillText(model())).toBe("");
  });

  it("writes the effective gain with up to four significant digits and no trailing zeros", () => {
    expect(gainPrefillText(headerGain(0.25))).toBe("0.25");
    expect(gainPrefillText(headerGain(4))).toBe("4");
    expect(gainPrefillText(headerGain(1100))).toBe("1100");
    expect(gainPrefillText(headerGain(0.25, { effective_gain: (2 * 16 * 0.25) / Math.PI }))).toBe("2.546");
  });

  it("prefills nothing for a zero or non-finite effective gain", () => {
    expect(gainPrefillText(headerGain(0))).toBe("");
    expect(gainPrefillText(headerGain(Number.NaN))).toBe("");
    expect(gainPrefillText(headerGain(4, { effective_gain: null }))).toBe("");
  });
});

describe("nextGainText", () => {
  it("prefills an empty field", () => {
    expect(nextGainText("", "", headerGain(4))).toEqual({ text: "4", prefill: "4" });
  });

  it("lets a prefilled value follow the new file", () => {
    expect(nextGainText("4", "4", headerGain(7))).toEqual({ text: "7", prefill: "7" });
  });

  it("keeps a value the user typed", () => {
    expect(nextGainText("3.2", "4", headerGain(7))).toEqual({ text: "3.2", prefill: "7" });
    expect(nextGainText("3.2", "4", null)).toEqual({ text: "3.2", prefill: "" });
  });

  it("follows the C-S5 sequence: 502nmos, typed 3.5 kept on f200w, cleared, rate file, 502nmos again", () => {
    const first = nextGainText("", "", WFPC2_502NMOS);
    expect(first).toEqual({ text: "7", prefill: "7" });
    const typed = nextGainText("3.5", first.prefill, ERR_PLANE);
    expect(typed).toEqual({ text: "3.5", prefill: "" });
    const rate = nextGainText("", typed.prefill, ERR_PLANE);
    expect(rate).toEqual({ text: "", prefill: "" });
    expect(nextGainText(rate.text, rate.prefill, WFPC2_502NMOS)).toEqual({ text: "7", prefill: "7" });
  });

  it("clears a prefilled value when the next file has no header gain", () => {
    expect(nextGainText("7", "7", ERR_PLANE)).toEqual({ text: "", prefill: "" });
  });
});

describe("gainCaption", () => {
  it("is null before a model is known", () => {
    expect(gainCaption(null, "")).toBeNull();
    expect(gainCaption(null, "3.2")).toBeNull();
  });

  it("names the keyword of an unscaled header gain", () => {
    expect(gainCaption(headerGain(0.25), "0.25")).toBe("gain 0.25 e-/ADU from EGAIN");
    expect(gainCaption(headerGain(0.25), "")).toBe("gain 0.25 e-/ADU from EGAIN");
  });

  it("spells out the mean stack scaling", () => {
    const mean = headerGain(0.25, { ncombine: 16, combine_method: "mean", combine_scaled: true, effective_gain: 4 });
    expect(gainCaption(mean, "4")).toBe("gain 4 e-/ADU = 16 × 0.25 (EGAIN), mean stack (NCOMBINE=16)");
  });

  it("spells out the median stack scaling", () => {
    const median = headerGain(0.25, {
      ncombine: 16,
      combine_method: "median",
      combine_scaled: true,
      effective_gain: (2 * 16 * 0.25) / Math.PI,
    });
    expect(gainCaption(median, "2.546")).toBe("gain 2.546 e-/ADU = 2 × 16 × 0.25 (EGAIN) / π, median stack (NCOMBINE=16)");
  });

  it("spells out the drizzle scaling", () => {
    const drizzle = headerGain(0.25, {
      ncombine: 16,
      combine_method: "mean",
      drizzle_scale: 2,
      combine_scaled: true,
      effective_gain: 1,
    });
    expect(gainCaption(drizzle, "1")).toBe(
      "gain 1 e-/ADU = 16 × 0.25 (EGAIN) / 2², drizzle (NCOMBINE=16); noise is correlated between output pixels, so the Poisson term is approximate",
    );
  });

  it("adds the backend note to an unscaled extreme-value combine", () => {
    const min = headerGain(0.25, {
      ncombine: 16,
      combine_method: "min",
      note: "min combine of 16 frames: the gain is not scaled (an extreme-value combine has no Poisson scaling)",
    });
    expect(gainCaption(min, "0.25")).toBe(
      "gain 0.25 e-/ADU from EGAIN; min combine of 16 frames: the gain is not scaled (an extreme-value combine has no Poisson scaling)",
    );
  });

  it("names ATODGAIN and the unknown NCOMBINE on the WFPC2 mosaic (P13)", () => {
    expect(gainCaption(WFPC2_502NMOS, "7")).toBe(
      "gain 7 e-/ADU from ATODGAIN; NCOMBINE=2 in the header: gain not scaled because the combine method is unknown; enter 2 × 7 = 14 e-/ADU for a mean stack",
    );
  });

  it("says the ERR plane carries the Poisson noise", () => {
    expect(gainCaption(ERR_PLANE, "")).toBe(ERR_CAPTION);
  });

  it("names the header gain kept as the fallback where ERR is not finite", () => {
    const fallback = model({ poisson_route: "err_plane", fallback_gain: 0.25, source: "EGAIN", gain_e_per_adu: 0.25 });
    expect(gainCaption(fallback, "")).toBe(`${ERR_CAPTION} (header gain 0.25 e-/ADU used only where ERR is not finite)`);
  });

  it("shows the backend note when no gain applies", () => {
    const calibrated = model({ unit_class: "calibrated", note: "JWST/Roman calibrated units (MJy/sr): no gain applies" });
    expect(gainCaption(calibrated, "")).toBe("JWST/Roman calibrated units (MJy/sr): no gain applies");
  });

  it("reports a GAIN card as present but not applied", () => {
    expect(gainCaption(model({ gain_card: 120 }), "")).toBe(
      "no gain in the header; GAIN=120 present (camera gain setting on ZWO/QHY CMOS; e-/ADU on IRAF/NOAO/LCO pipelines) — not applied; enter the e-/ADU value if that is what it is",
    );
  });

  it("leaves the GAIN sentence out when an e-/ADU keyword was found", () => {
    expect(gainCaption(headerGain(0.25, { gain_card: 120 }), "0.25")).toBe("gain 0.25 e-/ADU from EGAIN");
  });

  it("prefixes a field value that differs from the header model", () => {
    const caption = gainCaption(headerGain(7), "3.2");
    expect(caption!.startsWith("field value 3.2 differs from the header model — ")).toBe(true);
    expect(caption).toBe("field value 3.2 differs from the header model — gain 7 e-/ADU from EGAIN");
  });

  it("prefixes a value carried over to a file whose Poisson noise comes from the ERR plane (C-S5-3)", () => {
    expect(gainCaption(ERR_PLANE, "3.5")).toBe(`field value 3.5 differs from the header model — ${ERR_CAPTION}`);
  });
});
