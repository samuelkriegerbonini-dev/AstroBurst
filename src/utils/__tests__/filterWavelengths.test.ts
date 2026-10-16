import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, it, expect } from "vitest";
import { filterCodeAndWavelengthNm, filterToWavelengthNm, filterWavelengthEntries } from "../filterWavelengths";

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

describe("filter table additions", () => {
  it("resolves the NIRISS medium bands", () => {
    expect(filterCodeAndWavelengthNm("F158M")).toEqual({ code: "F158M", nm: 1580 });
    expect(filterCodeAndWavelengthNm("F380M")).toEqual({ code: "F380M", nm: 3800 });
  });

  it("resolves the HST broadband codes at their nominal code wavelengths", () => {
    expect(filterToWavelengthNm("F814W")).toBe(814);
    expect(filterToWavelengthNm("F555W")).toBe(555);
    expect(filterToWavelengthNm("F435W")).toBe(435);
    expect(filterToWavelengthNm("F439W")).toBe(439);
    const hst: [string, number][] = [
      ["F336W", 336], ["F390W", 390], ["F435W", 435], ["F438W", 438], ["F439W", 439],
      ["F450W", 450], ["F475W", 475], ["F555W", 555], ["F606W", 606], ["F625W", 625],
      ["F675W", 675], ["F702W", 702], ["F775W", 775], ["F814W", 814],
    ];
    for (const [code, nm] of hst) {
      expect(filterCodeAndWavelengthNm(code)).toEqual({ code, nm });
    }
  });

  it("resolves the Roman WFI pivot wavelengths", () => {
    const roman: [string, number][] = [
      ["F062", 629], ["F087", 870], ["F106", 1057], ["F129", 1290],
      ["F146", 1438], ["F158", 1575], ["F184", 1839], ["F213", 2123],
    ];
    for (const [code, nm] of roman) {
      expect(filterCodeAndWavelengthNm(code)).toEqual({ code, nm });
    }
    expect(filterCodeAndWavelengthNm("F129")).toEqual({ code: "F129", nm: 1290 });
    expect(filterCodeAndWavelengthNm("f129")).toEqual({ code: "F129", nm: 1290 });
    expect(filterCodeAndWavelengthNm("F129;CLEAR")).toEqual({ code: "F129", nm: 1290 });
    expect(filterCodeAndWavelengthNm("F158")).toEqual({ code: "F158", nm: 1575 });
    expect(filterCodeAndWavelengthNm("F158M")).toEqual({ code: "F158M", nm: 1580 });
  });

  it("matches the Rust mirror entry for entry", () => {
    const rustPath = fileURLToPath(new URL("../../../src-tauri/src/core/metadata/filter_wavelengths.rs", import.meta.url));
    const src = readFileSync(rustPath, "utf8");
    const start = src.indexOf("pub const FILTER_WAVELENGTHS_NM");
    expect(start).toBeGreaterThanOrEqual(0);
    const table = src.slice(start, src.indexOf("];", start));
    const rust: [string, number][] = [...table.matchAll(/\("([A-Z0-9_]+)",\s*(\d+)\)/g)].map((m) => [m[1], Number(m[2])]);
    const ts = filterWavelengthEntries();
    const byKey = (a: [string, number], b: [string, number]) => a[0].localeCompare(b[0]);
    expect(rust).toHaveLength(82);
    expect(ts).toHaveLength(82);
    expect(new Set(rust.map(([k]) => k)).size).toBe(82);
    expect(new Set(ts.map(([k]) => k)).size).toBe(82);
    expect([...rust].sort(byKey)).toEqual([...ts].sort(byKey));
    expect([...ts].sort(byKey)).toEqual([...rust].sort(byKey));
  });
});
