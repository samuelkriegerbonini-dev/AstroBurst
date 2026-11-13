import type { DqFlag, DqFlagTable } from "../shared/types/dq";

export function rawBitFlags(bitsPresent: number): DqFlag[] {
  const value = bitsPresent >>> 0;
  const flags: DqFlag[] = [];
  for (let n = 0; n < 32; n++) {
    const bit = (1 << n) >>> 0;
    if ((value & bit) !== 0) flags.push({ bit, name: `BIT ${n}` });
  }
  return flags;
}

type ListedFlagSet = Pick<DqFlagTable, "flags"> & { table?: DqFlagTable["table"]; bits_present: number | null };
type FlagSetView = Pick<DqFlagTable, "flags" | "exclusion_mask" | "label"> & { bits_present: number | null; exclusion_edited?: boolean };

const ALL_BITS = 4294967295;
const MAX_LISTED_NAMES = 3;

export const UNNAMED_FLAGS_TITLE = "Flag layer without bit names: raw bits are listed and nothing is excluded until bits are chosen";
export const FLAG_LIST_LEGEND = "Ticked: drawn on the image. excl: left out of histogram, statistics and photometry while the box below is ticked.";
export const EXCLUDED_FLAG_TAG = "excl";
export const EXCLUDED_FLAG_TITLE = "In the exclusion set: left out of histogram, statistics and photometry while the box below is ticked";

export function isNamedFlagSet(table: Pick<DqFlagTable, "flags">): boolean {
  return table.flags.length > 0;
}

export function listedDqFlags(table: ListedFlagSet): DqFlag[] {
  return isNamedFlagSet(table) ? table.flags : rawBitFlags(table.bits_present ?? 0);
}

export function emptyFlagsText(table: ListedFlagSet): string | null {
  if (listedDqFlags(table).length > 0) return null;
  return table.bits_present === null ? "flag bits could not be read from this plane" : "no bits set in this plane";
}

const excludedBits = (table: FlagSetView) => table.exclusion_mask >>> 0;
const excludesEveryPixel = (table: FlagSetView) => !isNamedFlagSet(table) && excludedBits(table) === ALL_BITS;

export function excludedFlagNames(table: FlagSetView): string[] {
  const mask = excludedBits(table);
  const named = table.flags.filter((f) => hasBit(mask, f.bit));
  const namedBits = named.reduce((bits, f) => (bits | f.bit) >>> 0, 0);
  const unnamed = rawBitFlags((mask & ~namedBits) >>> 0);
  return [...named, ...unnamed].map((f) => f.name);
}

export function exclusionLabel(table: FlagSetView): string {
  if (excludedBits(table) === 0) return "exclude flags from stats (none chosen: excludes nothing)";
  if (excludesEveryPixel(table)) return "exclude every non-zero pixel from stats";
  const names = excludedFlagNames(table);
  return names.length > MAX_LISTED_NAMES ? `exclude ${names.length} flags from stats` : `exclude ${names.join(", ")} from stats`;
}

export function exclusionTitle(table: FlagSetView): string {
  if (excludedBits(table) === 0) {
    return isNamedFlagSet(table)
      ? `No flag of ${table.label} is excluded: tick flags and press "Exclude the checked flags"`
      : UNNAMED_FLAGS_TITLE;
  }
  if (excludesEveryPixel(table)) return "Every pixel whose mask value is not zero is ignored by histogram, statistics and photometry";
  return `Ignored by histogram, statistics and photometry: ${excludedFlagNames(table).join(", ")}`;
}

export function exclusionSummary(table: FlagSetView): string {
  if (excludedBits(table) === 0) return `Excluded now: nothing${table.exclusion_edited === true ? " (chosen)" : ""}`;
  const what = excludesEveryPixel(table) ? "every non-zero pixel" : excludedFlagNames(table).join(", ");
  return `Excluded now: ${what} ${table.exclusion_edited === true ? "(chosen)" : "(default)"}`;
}

export function hasBit(mask: number, bit: number): boolean {
  return ((mask >>> 0) & (bit >>> 0)) !== 0;
}

export function toggleBit(mask: number, bit: number): number {
  return ((mask >>> 0) ^ (bit >>> 0)) >>> 0;
}

export function decodeDqBits(bits: number, flags: DqFlag[]): string[] {
  const value = bits >>> 0;
  const names: string[] = [];
  for (let n = 0; n < 32; n++) {
    const bit = (1 << n) >>> 0;
    if ((value & bit) === 0) continue;
    const flag = flags.find((f) => (f.bit >>> 0) === bit);
    names.push(flag ? flag.name : `BIT${n}`);
  }
  return names;
}

export function formatDqBits(bits: number, flags: DqFlag[]): string {
  const value = bits >>> 0;
  if (value === 0) return "0: GOOD";
  return `${value}: ${decodeDqBits(value, flags).join(" | ")}`;
}
