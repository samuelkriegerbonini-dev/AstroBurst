import { describe, it, expect } from "vitest";
import { filterCodeAndWavelengthNm, filterToWavelengthNm } from "../filterWavelengths";

describe("filterCodeAndWavelengthNm", () => {
  it("resolves a direct hit", () => {
    expect(filterCodeAndWavelengthNm("F444W")).toEqual({ code: "F444W", nm: 4440 });
  });

  it("is case-insensitive and trims whitespace", () => {
    expect(filterCodeAndWavelengthNm(" ha ")).toEqual({ code: "HA", nm: 656 });
    expect(filterCodeAndWavelengthNm("f090w")).toEqual({ code: "F090W", nm: 900 });
  });

  it("falls back to the first known token of a compound name", () => {
    expect(filterCodeAndWavelengthNm("CLEAR/F090W")).toEqual({ code: "F090W", nm: 900 });
    expect(filterCodeAndWavelengthNm("F444W;CLEAR")).toEqual({ code: "F444W", nm: 4440 });
  });

  it("returns null for CLEAR alone and for unknown filters", () => {
    expect(filterCodeAndWavelengthNm("CLEAR")).toBeNull();
    expect(filterCodeAndWavelengthNm("CLEAR/CLEAR")).toBeNull();
    expect(filterCodeAndWavelengthNm("XYZ123")).toBeNull();
  });

  it("returns null for null, undefined and empty input", () => {
    expect(filterCodeAndWavelengthNm(null)).toBeNull();
    expect(filterCodeAndWavelengthNm(undefined)).toBeNull();
    expect(filterCodeAndWavelengthNm("")).toBeNull();
  });
});

describe("filterToWavelengthNm", () => {
  it("delegates to filterCodeAndWavelengthNm and returns only the wavelength", () => {
    expect(filterToWavelengthNm("F444W")).toBe(4440);
    expect(filterToWavelengthNm("CLEAR/F090W")).toBe(900);
    expect(filterToWavelengthNm("CLEAR")).toBeNull();
    expect(filterToWavelengthNm(undefined)).toBeNull();
  });
});
