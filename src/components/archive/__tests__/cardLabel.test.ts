import { describe, expect, it } from "vitest";
import {
  cardAccessibleName,
  dateText,
  exclusiveText,
  exposureText,
  filterRangeText,
  instrumentText,
  isPublic,
  levelBadge,
} from "../logic/cardLabel";
import { row } from "./fixtures";

describe("cardAccessibleName", () => {
  it("says what the card shows", () => {
    expect(cardAccessibleName(row("jw02739-o001_t001_nircam"), "JWST")).toBe("M 16, JWST NIRCam, level 3, 6 filters, 2022-08-14, public");
  });

  it("names a single filter, an exposure level and exclusive access", () => {
    const r = row("jw01234001001_02101_00001", {
      levelClass: "exposure",
      level: "2b",
      instrument: "MIRI",
      filters: ["F770W"],
      access: "EXCLUSIVE",
      releaseDate: "2027-03-02T00:00:00",
    });
    expect(cardAccessibleName(r, "JWST")).toBe("M 16, JWST MIRI, level 2b, filter F770W, 2022-08-14, exclusive until 2027-03-02");
  });

  it("falls back to the dataset id and skips missing parts", () => {
    const r = row("ICK909030", { target: null, levelClass: "unknown", level: null, instrument: "WFC3", filters: [], dateObs: null, access: null });
    expect(cardAccessibleName(r, "HST")).toBe("ICK909030, HST WFC3");
  });
});

describe("card text helpers", () => {
  it("shows the filter range by wavelength", () => {
    expect(filterRangeText(["F090W", "F187N", "F200W", "F335M", "F444W", "F444W-F470N"])).toBe("F090W..F470N");
    expect(filterRangeText(["F1130W", "F770W", "F1000W"])).toBe("F770W..F1130W");
    expect(filterRangeText(["F502N"])).toBe("F502N");
    expect(filterRangeText(["F200W", "F090W"])).toBe("F090W, F200W");
    expect(filterRangeText([])).toBe("");
    expect(filterRangeText(["CLEAR", "PRISM", "F070LP"])).toBe("F070LP, CLEAR, PRISM");
  });

  it("shows exposure in hours, minutes or seconds", () => {
    expect(exposureText(5797.86)).toBe("1.6 h");
    expect(exposureText(4320)).toBe("1.2 h");
    expect(exposureText(708.6)).toBe("12 min");
    expect(exposureText(45.2)).toBe("45 s");
    expect(exposureText(null)).toBe("");
  });

  it("keeps only the date of an observation time", () => {
    expect(dateText("2022-08-14T08:33:45.2960000")).toBe("2022-08-14");
    expect(dateText("2022-08-14 08:33:45")).toBe("2022-08-14");
    expect(dateText(null)).toBe("");
  });

  it("formats instruments the way the missions write them", () => {
    expect(instrumentText("NIRCAM")).toBe("NIRCam");
    expect(instrumentText("NIRSPEC")).toBe("NIRSpec");
    expect(instrumentText("MIRI")).toBe("MIRI");
    expect(instrumentText("WFC3")).toBe("WFC3");
    expect(instrumentText(null)).toBe("");
  });

  it("badges the level class", () => {
    expect(levelBadge("combined")).toBe("L3");
    expect(levelBadge("exposure")).toBe("L1-2");
    expect(levelBadge("unknown")).toBeNull();
  });

  it("words exclusive access", () => {
    expect(isPublic(row("a"))).toBe(true);
    expect(isPublic(row("a", { access: "public" }))).toBe(true);
    expect(isPublic(row("a", { access: null }))).toBe(true);
    expect(exclusiveText(row("a", { access: "EXCLUSIVE", releaseDate: "2027-03-02T00:00:00" }))).toBe("Exclusive until 2027-03-02");
    expect(exclusiveText(row("a", { access: "EXCLUSIVE", releaseDate: null }))).toBe("Exclusive access");
    expect(exclusiveText(row("a"))).toBeNull();
  });

  it("treats a release date still to come as exclusive when MAST gives no access column", () => {
    const today = Date.UTC(2026, 9, 16, 12);
    const hst = (releaseDate: string | null) => row("ICK909030", { access: null, releaseDate });
    expect(isPublic(hst("2026-10-17T00:00:00"), today)).toBe(false);
    expect(isPublic(hst("2026-10-16T18:56:50.210000"), today)).toBe(true);
    expect(isPublic(hst("2007-05-18T18:56:50.210000"), today)).toBe(true);
    expect(isPublic(hst(null), today)).toBe(true);
    expect(isPublic(hst("not a date"), today)).toBe(true);
    expect(isPublic(row("a", { access: "PUBLIC", releaseDate: "2999-01-01T00:00:00" }), today)).toBe(true);
    expect(isPublic(hst("2999-01-01T00:00:00"))).toBe(false);
    expect(exclusiveText(hst("2999-01-01T00:00:00"))).toBe("Exclusive until 2999-01-01");
    expect(cardAccessibleName(hst("2999-01-01T00:00:00"), "HST")).toMatch(/exclusive until 2999-01-01$/);
    expect(cardAccessibleName(hst("2007-05-18T18:56:50.210000"), "HST")).not.toMatch(/exclusive|public/);
  });
});
