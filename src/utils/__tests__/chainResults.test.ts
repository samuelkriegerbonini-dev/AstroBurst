import { describe, it, expect } from "vitest";
import { EMPTY_CHAIN, chainHoldsOutput, withStep } from "../processingChain";
import type { ChainEntry } from "../../shared/types/preview";

function entry(fitsPath: string): ChainEntry {
  return { fitsPath, previewUrl: null, dimensions: null };
}

const BG1 = "C:/out/M31_bg_1.fits";
const BG2 = "C:/out/M31_bg_2.fits";
const DN = "C:/out/M31_denoise.fits";

describe("chainHoldsOutput", () => {
  it("drops the denoise result once background is re-run upstream", () => {
    const afterDenoise = withStep(withStep(EMPTY_CHAIN, "background", entry(BG1)), "denoise", entry(DN));
    expect(chainHoldsOutput(afterDenoise, "denoise", DN)).toBe(true);
    const rerun = withStep(afterDenoise, "background", entry(BG2));
    expect(chainHoldsOutput(rerun, "denoise", DN)).toBe(false);
    expect(chainHoldsOutput(rerun, "background", BG1)).toBe(false);
    expect(chainHoldsOutput(rerun, "background", BG2)).toBe(true);
  });

  it("drops every panel result after Reset processing chain", () => {
    for (const [step, path] of [["background", BG1], ["denoise", DN]] as const) {
      expect(chainHoldsOutput(EMPTY_CHAIN, step, path)).toBe(false);
    }
  });

  it("drops the arcsinh result when masked stretch replaces it", () => {
    const stretched = withStep(EMPTY_CHAIN, "stretch", entry("C:/out/s.fits"));
    const masked = withStep(stretched, "maskedStretch", entry("C:/out/m.fits"));
    expect(chainHoldsOutput(masked, "stretch", "C:/out/s.fits")).toBe(false);
    expect(chainHoldsOutput(masked, "maskedStretch", "C:\\OUT\\m.fits")).toBe(true);
  });

  it("keeps results that have no FITS output to compare, such as composite runs", () => {
    expect(chainHoldsOutput(EMPTY_CHAIN, "localContrast", undefined)).toBe(true);
    expect(chainHoldsOutput(EMPTY_CHAIN, "localContrast", null)).toBe(true);
  });
});
