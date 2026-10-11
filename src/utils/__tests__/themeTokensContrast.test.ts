import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(fileURLToPath(new URL("../../index.css", import.meta.url)), "utf8");

type Rgba = [number, number, number, number];

function rootTokens(text: string): Map<string, string> {
  const block = /:root\s*\{([^}]*)\}/.exec(text)?.[1] ?? "";
  const tokens = new Map<string, string>();
  for (const m of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) tokens.set(m[1], m[2].trim());
  return tokens;
}

function resolve(tokens: Map<string, string>, name: string): string {
  const value = tokens.get(name);
  if (value === undefined) throw new Error(`missing token ${name}`);
  const ref = /^var\((--[\w-]+)\)$/.exec(value);
  return ref ? resolve(tokens, ref[1]) : value;
}

function parseColor(value: string): Rgba {
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
  }
  const fn = /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/i.exec(value);
  if (fn) return [Number(fn[1]), Number(fn[2]), Number(fn[3]), fn[4] === undefined ? 1 : Number(fn[4])];
  throw new Error(`unparsed colour ${value}`);
}

function over(top: Rgba, under: Rgba): Rgba {
  const a = top[3];
  return [0, 1, 2].map((i) => top[i] * a + under[i] * (1 - a)).concat(1) as Rgba;
}

function luminance([r, g, b]: Rgba): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a: Rgba, b: Rgba): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const tokens = rootTokens(css);
const color = (name: string) => parseColor(resolve(tokens, name));

function backgrounds(): Array<[string, Rgba]> {
  const deep = color("--ab-deep");
  const panel = over(color("--ab-bg-panel"), deep);
  return [
    ["deep", deep],
    ["panel", panel],
    ["surface", over(color("--ab-surface"), deep)],
    ["raised", over(color("--ab-bg-raised"), deep)],
    ["raised on panel", over(color("--ab-bg-raised"), panel)],
    ["hover on panel", over(color("--ab-bg-hover"), panel)],
    ["active on panel", over(color("--ab-bg-active"), panel)],
  ];
}

describe("muted text tokens", () => {
  it("are read from the :root block of index.css", () => {
    expect(tokens.get("--ab-text-3")).toMatch(/^#[0-9a-f]{6}$/i);
    expect(tokens.get("--ab-text-4")).toMatch(/^#[0-9a-f]{6}$/i);
  });

  it("parse the colour helpers on known WCAG pairs", () => {
    expect(contrast(parseColor("#ffffff"), parseColor("#000000"))).toBeCloseTo(21, 5);
    expect(contrast(parseColor("#71717a"), parseColor("#0a0a0f"))).toBeLessThan(4.5);
    expect(over(parseColor("rgba(255, 255, 255, 0.5)"), parseColor("#000000"))).toEqual([127.5, 127.5, 127.5, 1]);
  });

  it.each(["--ab-text-3", "--ab-text-4"])("%s reaches 4.5:1 on every panel background token", (name) => {
    const text = color(name);
    expect(text[3]).toBe(1);
    const failing = backgrounds()
      .map(([label, bg]) => [label, Math.round(contrast(text, bg) * 100) / 100] as const)
      .filter(([, ratio]) => ratio < 4.5);
    expect(failing).toEqual([]);
  });
});
