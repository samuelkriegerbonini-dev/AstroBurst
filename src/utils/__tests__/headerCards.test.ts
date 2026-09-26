import { describe, expect, it } from "vitest";
import { groupHeaderCards, headerCardLine } from "../headerCards";

const ORDER = ["observation", "instrument", "image", "wcs", "processing", "other"];

const CARDS = [
  { key: "SIMPLE", value: "T" },
  { key: "NAXIS1", value: "800" },
  { key: "TELESCOP", value: "'HST'" },
  { key: "HISTORY", value: "MASKFILE= 'f8213081u.r0h'" },
  { key: "CRVAL1", value: "83.8" },
  { key: "HISTORY", value: "BIASFILE= 'e2112084u.r2h'" },
  { key: "DATE-OBS", value: "'1995-03-04'" },
  { key: "COMMENT", value: "first note" },
  { key: "HISTORY", value: "FLATFILE= 'f4i1559cu.r4h'" },
  { key: "BITPIX", value: "-32" },
];

const CATEGORIES = {
  observation: { "DATE-OBS": "'1995-03-04'", TELESCOP: "'HST'" },
  instrument: {},
  image: { BITPIX: "-32", NAXIS1: "800" },
  wcs: { CRVAL1: "83.8" },
  processing: { COMMENT: "first note", HISTORY: "FLATFILE= 'f4i1559cu.r4h'" },
  other: {},
};

describe("groupHeaderCards", () => {
  it("keeps every repeated HISTORY card, in header order, under Processing", () => {
    const groups = groupHeaderCards(CARDS, CATEGORIES, ORDER, "");
    const processing = groups.find((g) => g.category === "processing");
    expect(processing?.rows.map((r) => r.value)).toEqual([
      "MASKFILE= 'f8213081u.r0h'",
      "BIASFILE= 'e2112084u.r2h'",
      "first note",
      "FLATFILE= 'f4i1559cu.r4h'",
    ]);
  });

  it("lists categories in the display order and keys in header order, not alphabetically", () => {
    const groups = groupHeaderCards(CARDS, CATEGORIES, ORDER, "");
    expect(groups.map((g) => g.category)).toEqual(["observation", "image", "wcs", "processing", "other"]);
    expect(groups[0].rows.map((r) => r.key)).toEqual(["TELESCOP", "DATE-OBS"]);
    expect(groups[1].rows.map((r) => r.key)).toEqual(["NAXIS1", "BITPIX"]);
  });

  it("gives every row its card index so repeated keys stay distinct", () => {
    const rows = groupHeaderCards(CARDS, CATEGORIES, ORDER, "").flatMap((g) => g.rows);
    expect(rows).toHaveLength(CARDS.length);
    expect(new Set(rows.map((r) => r.index)).size).toBe(CARDS.length);
  });

  it("puts a key the backend did not categorise under Other", () => {
    const other = groupHeaderCards(CARDS, CATEGORIES, ORDER, "").find((g) => g.category === "other");
    expect(other?.rows.map((r) => r.key)).toEqual(["SIMPLE"]);
  });

  it("filters on key or value, case-insensitively, and drops empty categories", () => {
    const groups = groupHeaderCards(CARDS, CATEGORIES, ORDER, "  flatFILE ");
    expect(groups).toHaveLength(1);
    expect(groups[0].rows.map((r) => r.value)).toEqual(["FLATFILE= 'f4i1559cu.r4h'"]);
    expect(groupHeaderCards(CARDS, CATEGORIES, ORDER, "naxis").flatMap((g) => g.rows.map((r) => r.key))).toEqual(["NAXIS1"]);
  });
});

describe("headerCardLine", () => {
  it("writes value cards with an equals sign", () => {
    expect(headerCardLine("OBJECT", "'M31'", 8)).toBe("OBJECT   = 'M31'");
    expect(headerCardLine("OBJECT", "M31")).toBe("OBJECT = M31");
  });

  it("writes commentary cards without an equals sign, as FITS does", () => {
    expect(headerCardLine("HISTORY", "MASKFILE= 'x.r0h'", 8)).toBe("HISTORY MASKFILE= 'x.r0h'");
    expect(headerCardLine("COMMENT", "first note")).toBe("COMMENT first note");
  });
});
