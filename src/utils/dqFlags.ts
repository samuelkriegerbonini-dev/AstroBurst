import type { DqFlag } from "../shared/types/dq";

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
