import { describe, expect, it } from "vitest";
import { STF_STATE_LABEL, stfStateOf } from "../analysisTarget";
import { IDENTITY_STF } from "../displayMemory";

const AUTO_673NMOS = { shadow: 0.004321, midtone: 0.008467, highlight: 1 };

describe("stfStateOf (SF-01)", () => {
  it("is auto when the STF is the auto STF", () => {
    expect(stfStateOf({ ...AUTO_673NMOS }, AUTO_673NMOS)).toBe("auto");
  });

  it("is auto when every parameter is within 1e-6 of the auto STF", () => {
    const near = { shadow: 0.004321 + 9e-7, midtone: 0.008467 - 9e-7, highlight: 1 - 9e-7 };
    expect(stfStateOf(near, AUTO_673NMOS)).toBe("auto");
  });

  it("is manual as soon as one parameter is more than 1e-6 off the auto STF", () => {
    expect(stfStateOf({ ...AUTO_673NMOS, shadow: 0.004323 }, AUTO_673NMOS)).toBe("manual");
    expect(stfStateOf({ ...AUTO_673NMOS, midtone: 0.0121 }, AUTO_673NMOS)).toBe("manual");
    expect(stfStateOf({ ...AUTO_673NMOS, highlight: 0.95 }, AUTO_673NMOS)).toBe("manual");
  });

  it("is linear for Reset's 0 / 0.5 / 1, exactly or within 1e-6", () => {
    expect(stfStateOf(IDENTITY_STF, AUTO_673NMOS)).toBe("linear");
    expect(stfStateOf({ shadow: 0, midtone: 0.5, highlight: 1 }, AUTO_673NMOS)).toBe("linear");
    expect(stfStateOf({ shadow: 5e-7, midtone: 0.5 + 5e-7, highlight: 1 - 5e-7 }, AUTO_673NMOS)).toBe("linear");
  });

  it("is manual for a typed STF that is neither the auto STF nor 0 / 0.5 / 1", () => {
    expect(stfStateOf({ shadow: 0.01, midtone: 0.25, highlight: 0.9 }, AUTO_673NMOS)).toBe("manual");
    expect(stfStateOf({ shadow: 0, midtone: 0.5, highlight: 0.99 }, AUTO_673NMOS)).toBe("manual");
  });

  it("falls back to linear or manual when the histogram has no auto STF", () => {
    expect(stfStateOf(IDENTITY_STF, null)).toBe("linear");
    expect(stfStateOf(IDENTITY_STF, undefined)).toBe("linear");
    expect(stfStateOf(AUTO_673NMOS, null)).toBe("manual");
  });

  it("calls an auto STF that is itself 0 / 0.5 / 1 auto, so Auto stays pressed after a click on it", () => {
    expect(stfStateOf(IDENTITY_STF, { shadow: 0, midtone: 0.5, highlight: 1 })).toBe("auto");
  });
});

describe("STF_STATE_LABEL (SF-01)", () => {
  it("names the three states as the readout shows them", () => {
    expect(STF_STATE_LABEL).toEqual({ auto: "Auto STF", manual: "Manual STF", linear: "Linear" });
  });
});
