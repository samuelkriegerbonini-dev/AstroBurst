import { describe, it, expect } from "vitest";
import { decodeDqBits, formatDqBits, toggleBit, hasBit, rawBitFlags, listedDqFlags } from "../dqFlags";
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
