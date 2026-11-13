import { describe, it, expect } from "vitest";
import {
  decodeDqBits,
  formatDqBits,
  toggleBit,
  hasBit,
  rawBitFlags,
  listedDqFlags,
  emptyFlagsText,
  isNamedFlagSet,
  excludedFlagNames,
  exclusionLabel,
  exclusionTitle,
  exclusionSummary,
  UNNAMED_FLAGS_TITLE,
} from "../dqFlags";
import type { DqFlag } from "../../shared/types/dq";

const FLAGS: DqFlag[] = [
  { bit: 1, name: "DO_NOT_USE" },
  { bit: 2, name: "SATURATED" },
  { bit: 4, name: "JUMP_DET" },
  { bit: 2147483648, name: "REFERENCE_PIXEL" },
];

describe("decodeDqBits", () => {
  it("lists set bits ascending by name", () => {
    expect(decodeDqBits(3, FLAGS)).toEqual(["DO_NOT_USE", "SATURATED"]);
  });

  it("returns an empty list for 0", () => {
    expect(decodeDqBits(0, FLAGS)).toEqual([]);
  });

  it("handles bit 31 unsigned-safely", () => {
    expect(decodeDqBits(2147483648, FLAGS)).toEqual(["REFERENCE_PIXEL"]);
    expect(decodeDqBits(2147483649, FLAGS)).toEqual(["DO_NOT_USE", "REFERENCE_PIXEL"]);
    expect(decodeDqBits(-2147483648, FLAGS)).toEqual(["REFERENCE_PIXEL"]);
  });

  it("names unknown bits BIT<n>", () => {
    expect(decodeDqBits(1 << 20, FLAGS)).toEqual(["BIT20"]);
    expect(decodeDqBits((1 << 20) | 2, FLAGS)).toEqual(["SATURATED", "BIT20"]);
  });
});

describe("formatDqBits", () => {
  it("formats 0 as GOOD", () => {
    expect(formatDqBits(0, FLAGS)).toBe("0: GOOD");
  });

  it("joins names with a pipe", () => {
    expect(formatDqBits(3, FLAGS)).toBe("3: DO_NOT_USE | SATURATED");
    expect(formatDqBits(2147483648, FLAGS)).toBe("2147483648: REFERENCE_PIXEL");
  });
});

describe("rawBitFlags / listedDqFlags", () => {
  it("names each set bit BIT n in ascending order, unsigned-safe for bit 31", () => {
    expect(rawBitFlags(0b101)).toEqual([
      { bit: 1, name: "BIT 0" },
      { bit: 4, name: "BIT 2" },
    ]);
    expect(rawBitFlags(0)).toEqual([]);
    expect(rawBitFlags(-2147483648)).toEqual([{ bit: 2147483648, name: "BIT 31" }]);
    expect(rawBitFlags(4294967295)).toHaveLength(32);
  });

  it("lists raw bits for an unknown table and the named flags otherwise", () => {
    expect(listedDqFlags({ table: "unknown", flags: [], bits_present: 513 })).toEqual([
      { bit: 1, name: "BIT 0" },
      { bit: 512, name: "BIT 9" },
    ]);
    expect(listedDqFlags({ table: "jwst", flags: FLAGS, bits_present: null })).toBe(FLAGS);
    expect(listedDqFlags({ table: "unknown", flags: [], bits_present: null })).toEqual([]);
  });

  it("lists the names a header gives even though the table stays unknown", () => {
    const named = [{ bit: 1, name: "BAD" }, { bit: 2, name: "SAT" }];
    expect(listedDqFlags({ table: "unknown", flags: named, bits_present: null })).toEqual(named);
  });

  it("says why a list is empty", () => {
    expect(emptyFlagsText({ flags: [], bits_present: 0 })).toBe("no bits set in this plane");
    expect(emptyFlagsText({ flags: [], bits_present: null })).toBe("flag bits could not be read from this plane");
    expect(emptyFlagsText({ flags: [], bits_present: 5 })).toBeNull();
    expect(emptyFlagsText({ flags: FLAGS, bits_present: null })).toBeNull();
  });
});

const AFW = {
  label: "MP_ cards (afw mask planes)",
  flags: [
    { bit: 1, name: "BAD" },
    { bit: 2, name: "SAT" },
    { bit: 8, name: "CR" },
    { bit: 32, name: "DETECTED" },
    { bit: 256, name: "NO_DATA" },
  ],
  bits_present: null,
};

describe("exclusion texts", () => {
  it("names the excluded flags", () => {
    const set = { ...AFW, exclusion_mask: 267 };
    expect(isNamedFlagSet(set)).toBe(true);
    expect(excludedFlagNames(set)).toEqual(["BAD", "SAT", "CR", "NO_DATA"]);
    expect(exclusionLabel(set)).toBe("exclude 4 flags from stats");
    expect(exclusionTitle(set)).toBe("Ignored by histogram, statistics and photometry: BAD, SAT, CR, NO_DATA");
    expect(exclusionSummary(set)).toBe("Excluded now: BAD, SAT, CR, NO_DATA (default)");
    expect(exclusionLabel({ ...AFW, exclusion_mask: 3 })).toBe("exclude BAD, SAT from stats");
    expect(excludedFlagNames({ ...AFW, exclusion_mask: 2 | 1024 })).toEqual(["SAT", "BIT 10"]);
    const none = { ...AFW, exclusion_mask: 0 };
    expect(excludedFlagNames(none)).toEqual([]);
    expect(exclusionLabel(none)).toBe("exclude flags from stats (none chosen: excludes nothing)");
    expect(exclusionTitle(none)).toBe('No flag of MP_ cards (afw mask planes) is excluded: tick flags and press "Exclude the checked flags"');
    expect(exclusionSummary(none)).toBe("Excluded now: nothing");
  });

  it("names raw bits and the every-pixel set of a layer without names", () => {
    const raw = { label: "unknown (raw bits)", flags: [], bits_present: 25301342, exclusion_mask: 0 };
    expect(isNamedFlagSet(raw)).toBe(false);
    expect(exclusionTitle(raw)).toBe(UNNAMED_FLAGS_TITLE);
    const chosen = { ...raw, exclusion_mask: (1 << 4) | (1 << 23), exclusion_edited: true };
    expect(excludedFlagNames(chosen)).toEqual(["BIT 4", "BIT 23"]);
    expect(exclusionLabel(chosen)).toBe("exclude BIT 4, BIT 23 from stats");
    expect(exclusionSummary(chosen)).toBe("Excluded now: BIT 4, BIT 23 (chosen)");
    const des = { label: "DES coadd mask (non-zero = no good data)", flags: [], bits_present: 0, exclusion_mask: 4294967295 };
    expect(exclusionLabel(des)).toBe("exclude every non-zero pixel from stats");
    expect(exclusionTitle(des)).toBe("Every pixel whose mask value is not zero is ignored by histogram, statistics and photometry");
    expect(exclusionSummary(des)).toBe("Excluded now: every non-zero pixel (default)");
  });

  it("keeps the jwst label", () => {
    expect(exclusionLabel({ label: "jwst", flags: [{ bit: 1, name: "DO_NOT_USE" }, { bit: 2, name: "SATURATED" }], bits_present: null, exclusion_mask: 1 })).toBe(
      "exclude DO_NOT_USE from stats",
    );
    const hst = Array.from({ length: 14 }, (_, n) => ({ bit: 2 ** n, name: `HST_${n}` }));
    expect(exclusionLabel({ label: "hst", flags: hst, bits_present: null, exclusion_mask: 16383 })).toBe("exclude 14 flags from stats");
  });
});

describe("toggleBit / hasBit", () => {
  it("toggles low bits", () => {
    expect(toggleBit(0, 1)).toBe(1);
    expect(toggleBit(7, 2)).toBe(5);
    expect(hasBit(7, 2)).toBe(true);
    expect(hasBit(5, 2)).toBe(false);
  });

  it("stays unsigned for bit 31", () => {
    const m = toggleBit(0, 2147483648);
    expect(m).toBe(2147483648);
    expect(hasBit(m, 2147483648)).toBe(true);
    expect(toggleBit(m, 2147483648)).toBe(0);
    expect(toggleBit(4294967295, 1)).toBe(4294967294);
    expect(hasBit(4294967295, 2147483648)).toBe(true);
  });
});
