import { describe, expect, it } from "vitest";
import { cfaPatternFromHeader, cosmeticCfaValue } from "../cfaPattern";

describe("cfaPatternFromHeader", () => {
  it("reads BAYERPAT with its FITS quotes stripped", () => {
    expect(cfaPatternFromHeader({ BAYERPAT: "'RGGB'" })).toBe("RGGB");
    expect(cfaPatternFromHeader({ BAYERPAT: " 'bggr    ' " })).toBe("BGGR");
  });

  it("falls back to COLORTYP", () => {
    expect(cfaPatternFromHeader({ COLORTYP: "GRBG" })).toBe("GRBG");
    expect(cfaPatternFromHeader({ COLORTYP: "'GBRG'" })).toBe("GBRG");
  });

  it("returns null without a Bayer card or for a non-Bayer value", () => {
    expect(cfaPatternFromHeader({})).toBeNull();
    expect(cfaPatternFromHeader({ COLORTYP: "RGB" })).toBeNull();
    expect(cfaPatternFromHeader(null)).toBeNull();
    expect(cfaPatternFromHeader(undefined)).toBeNull();
  });

  it("reads BAYERPAT first, like the backend's cfa_pattern", () => {
    expect(cfaPatternFromHeader({ BAYERPAT: "RGGB", COLORTYP: "GRBG" })).toBe("RGGB");
    expect(cfaPatternFromHeader({ BAYERPAT: "TRUE", COLORTYP: "GRBG" })).toBeNull();
  });
});

describe("cosmeticCfaValue", () => {
  it("leaves the CFA choice to the backend until the toggle is touched", () => {
    expect(cosmeticCfaValue(false, true)).toBeNull();
    expect(cosmeticCfaValue(false, false)).toBeNull();
  });

  it("sends the explicit toggle state once the user touched it", () => {
    expect(cosmeticCfaValue(true, true)).toBe(true);
    expect(cosmeticCfaValue(true, false)).toBe(false);
  });
});
