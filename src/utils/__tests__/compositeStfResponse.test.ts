import { describe, expect, it } from "vitest";
import { stfTripleOf } from "../compositeStfResponse";

const A = { shadow: 0.1, midtone: 0.3, highlight: 1 };
const B = { shadow: 0.2, midtone: 0.4, highlight: 1 };
const C = { shadow: 0.05, midtone: 0.2, highlight: 0.9 };

describe("stfTripleOf", () => {
  it("returns the per-channel STF and the unlinked flag of a composite response", () => {
    expect(stfTripleOf({ auto_stf: A, stf_r: A, stf_g: B, stf_b: C, stf_linked: false })).toEqual({ r: A, g: B, b: C, linked: false });
  });

  it("returns a linked STF recomputed by LRGB or star removal", () => {
    expect(stfTripleOf({ auto_stf: B, stf_r: B, stf_g: B, stf_b: B, stf_linked: true })).toEqual({ r: B, g: B, b: B, linked: true });
  });

  it("treats per-channel STFs without a flag as linked", () => {
    expect(stfTripleOf({ stf_r: A, stf_g: A, stf_b: A })?.linked).toBe(true);
  });

  it("falls back to the linked auto STF when the response carries only that", () => {
    expect(stfTripleOf({ auto_stf: C })).toEqual({ r: C, g: C, b: C, linked: true });
  });

  it("does not turn an unlinked response that lost a channel into a linked one", () => {
    expect(stfTripleOf({ auto_stf: A, stf_r: A, stf_g: B, stf_linked: false })).toBeNull();
  });

  it("returns null for a response without any STF", () => {
    expect(stfTripleOf({})).toBeNull();
    expect(stfTripleOf(null)).toBeNull();
    expect(stfTripleOf(undefined)).toBeNull();
  });
});
