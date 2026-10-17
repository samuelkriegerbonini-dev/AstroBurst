import { describe, expect, it } from "vitest";
import { allColumns, metadataSections, sRegionText, valueText } from "../logic/metadataView";
import { COLUMNS, row } from "./fixtures";
import type { MastResolved } from "../../../shared/types/mast";

const M16: MastResolved = { query: "M16", canonicalName: "M 16", resolver: "SIMBAD", raDeg: 274.688, decDeg: -13.792, objectType: null, elapsedMs: 870, fromCache: false };

function section(key: string, sections: ReturnType<typeof metadataSections>) {
  const found = sections.find((s) => s.key === key);
  if (!found) throw new Error(key);
  return found;
}

describe("metadataSections", () => {
  const jw = row("jw02739-o001_t001_nircam", { sepArcmin: 0.449, sepVerified: false });
  const sections = metadataSections({ row: jw, resolved: M16, searchPosition: { raDeg: 274.688, decDeg: -13.792 } });

  it("orders identifiers, pointing and observation", () => {
    expect(sections.map((s) => [s.key, s.title])).toEqual([
      ["identifiers", "Identifiers"],
      ["pointing", "Pointing"],
      ["observation", "Observation"],
    ]);
  });

  it("gives copyable identifiers with the programme, observation and target position", () => {
    expect(section("identifiers", sections).rows).toEqual([
      { label: "Dataset", value: "jw02739-o001_t001_nircam", copy: "jw02739-o001_t001_nircam", note: null },
      { label: "Programme / Obs", value: "2739 / 1", copy: "2739.1", note: null },
      { label: "Target", value: "M 16  (targ 274.68800 -13.79200)", copy: "M 16", note: null },
    ]);
  });

  it("shows the search position with its resolver, MAST's separation as reported and the raw s_region", () => {
    expect(section("pointing", sections).rows).toEqual([
      { label: "Search position", value: '274.68800 -13.79200 (SIMBAD "M 16")', copy: "274.68800 -13.79200", note: null },
      { label: "Sep (MAST)", value: "0.449", copy: null, note: "unit and meaning differ by mission" },
      { label: "s_region", value: sRegionText(jw.sRegion ?? ""), copy: jw.sRegion, note: "Draw it on the open image with Show on, above" },
    ]);
  });

  it("puts the projected footprint summary on the s_region row", () => {
    const summary = "1 polygon, 3 vertices, cone 274.7347 -13.8672 r 0.62', overlaps a.fits";
    const shown = metadataSections({ row: jw, resolved: null, searchPosition: null, footprintNote: summary });
    expect(section("pointing", shown).rows.find((r) => r.label === "s_region")?.note).toBe(summary);
  });

  it("converts a verified separation to arcminutes", () => {
    const hst = metadataSections({ row: row("ICK909030", { sepArcmin: 1.2345, sepVerified: true }), resolved: null, searchPosition: null });
    expect(section("pointing", hst).rows[0]).toEqual({ label: "Sep", value: "1.23'", copy: null, note: null });
  });

  it("lists the observation fields that are present", () => {
    expect(section("observation", sections).rows.map((r) => [r.label, r.value])).toEqual([
      ["Instrument", "NIRCAM"],
      ["Exp type", "NRC_IMAGE"],
      ["Filter (row)", "F187N"],
      ["Filters in set", "F090W F187N F200W F335M F444W F444W-F470N"],
      ["Date obs (UTC)", "2022-08-14 08:33:45"],
      ["Exposure", "5797.86 s"],
      ["Access", "PUBLIC"],
    ]);
    const excl = metadataSections({ row: row("x", { access: "PROPRIETARY", releaseDate: "2027-03-02T00:00:00", expType: null, filters: ["F770W"] }), resolved: null, searchPosition: null });
    expect(section("observation", excl).rows.map((r) => [r.label, r.value])).toEqual([
      ["Instrument", "NIRCAM"],
      ["Filter (row)", "F187N"],
      ["Date obs (UTC)", "2022-08-14 08:33:45"],
      ["Exposure", "5797.86 s"],
      ["Access", "PROPRIETARY"],
      ["Public from", "2027-03-02"],
    ]);
  });

  it("names a search position MAST resolved itself and skips missing pointing", () => {
    const none = metadataSections({ row: row("x", { sRegion: null, target: null, program: null, observation: null }), resolved: null, searchPosition: { raDeg: 10, decDeg: -80 } });
    expect(section("pointing", none).rows).toEqual([{ label: "Search position", value: "10.00000 -80.00000", copy: "10.00000 -80.00000", note: null }]);
    expect(section("identifiers", none).rows.map((r) => r.label)).toEqual(["Dataset"]);
  });
});

describe("s_region and column values", () => {
  it("shortens a long s_region and gives its length", () => {
    const long = `POLYGON ICRS 274.7${"0".repeat(2197 - 18)}`;
    expect(sRegionText(long)).toBe("POLYGON ICRS 274.7… (2,197 chars)");
    expect(sRegionText("CIRCLE 1 2 3")).toBe("CIRCLE 1 2 3");
  });

  it("prints every value verbatim", () => {
    expect(valueText(null)).toBe("");
    expect(valueText(undefined)).toBe("");
    expect(valueText(true)).toBe("true");
    expect(valueText(5797.86)).toBe("5797.86");
    expect(valueText("2022-08-14T08:33:45.2960000")).toBe("2022-08-14T08:33:45.2960000");
  });

  it("lists every returned column and filters by name or label", () => {
    const r = row("jw02739-o001_t001_nircam");
    expect(allColumns(r, COLUMNS, "")).toHaveLength(COLUMNS.length);
    expect(allColumns(r, COLUMNS, "").find((c) => c.name === "effexptm")).toEqual({ name: "effexptm", label: "Exp (s)", value: "5797.86" });
    expect(allColumns(r, COLUMNS, "DATE").map((c) => c.name)).toEqual(["date_obs"]);
    expect(allColumns(r, COLUMNS, "prog").map((c) => c.name)).toEqual(["program", "_prog_obs"]);
  });
});
