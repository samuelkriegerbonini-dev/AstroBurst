import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(fileURLToPath(new URL("../../index.css", import.meta.url)), "utf8");

type Rgb = [number, number, number];

function rootTokens(text: string): Map<string, string> {
  const block = /:root\s*\{([^}]*)\}/.exec(text)?.[1] ?? "";
  const tokens = new Map<string, string>();
  for (const m of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) tokens.set(m[1], m[2].trim());
  return tokens;
}

const tokens = rootTokens(css);

function resolveColor(value: string): string {
  const ref = /^var\((--[\w-]+)\)$/.exec(value.trim());
  if (!ref) return value.trim();
  const next = tokens.get(ref[1]);
  if (next === undefined) throw new Error(`missing token ${ref[1]}`);
  return resolveColor(next);
}

function hexRgb(value: string): Rgb {
  const hex = /^#([0-9a-f]{6})$/i.exec(value);
  if (!hex) throw new Error(`unparsed colour ${value}`);
  const n = parseInt(hex[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function whiteContrast(rgb: Rgb): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  const l = 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2]);
  return 1.05 / (l + 0.05);
}

function brighten(rgb: Rgb, k: number): Rgb {
  return rgb.map((c) => Math.min(255, c * k)) as Rgb;
}

function accentStops(text: string): Map<string, string[]> {
  const out = new Map<string, string[]>();
  const rule = /\.ab-run-btn\[data-accent="([\w-]+)"\][^{]*\{[^}]*?background:\s*linear-gradient\(([^;]*)\)\s*;/g;
  for (const m of text.matchAll(rule)) {
    const args = m[2].split(/,(?![^(]*\))/).map((a) => a.trim());
    out.set(m[1], args.filter((a) => !/deg$/.test(a)).map(resolveColor));
  }
  return out;
}

function hoverBrightness(text: string): number {
  const body = /\.ab-run-btn:not\(\[data-running="true"\]\):not\(\[data-disabled="true"\]\):hover\s*\{([^}]*)\}/.exec(text)?.[1];
  if (body === undefined) throw new Error("no Run button hover rule");
  const k = /filter:\s*brightness\(([\d.]+)\)/.exec(body)?.[1];
  return k === undefined ? 1 : Number(k);
}

const ACCENTS = ["emerald", "sky", "violet", "indigo", "amber", "teal", "cyan", "rose", "blue", "purple"];

function failing(k: number): string[] {
  return [...accentStops(css)].flatMap(([accent, stops]) =>
    stops
      .map((stop) => ({ stop, ratio: whiteContrast(brighten(hexRgb(stop), k)) }))
      .filter(({ ratio }) => ratio < 4.5)
      .map(({ stop, ratio }) => `${accent} ${stop} ${ratio.toFixed(2)}:1`),
  );
}

describe("Run button accents", () => {
  it("compute the WCAG ratio of white text on known colours", () => {
    expect(whiteContrast([0, 0, 0])).toBeCloseTo(21, 5);
    expect(whiteContrast(hexRgb("#767676"))).toBeGreaterThan(4.5);
    expect(whiteContrast(hexRgb("#777777"))).toBeLessThan(4.5);
    expect(brighten([100, 200, 0], 2)).toEqual([200, 255, 0]);
  });

  it("define a two-stop gradient for every accent", () => {
    const stops = accentStops(css);
    expect([...stops.keys()].sort()).toEqual([...ACCENTS].sort());
    for (const accent of ACCENTS) expect(stops.get(accent), accent).toHaveLength(2);
  });

  it("keep white text at 4.5:1 or more on both stops", () => {
    expect(failing(1)).toEqual([]);
  });

  it("keep white text at 4.5:1 or more on both stops under the hover filter", () => {
    expect(hoverBrightness(css)).toBeGreaterThanOrEqual(1);
    expect(failing(hoverBrightness(css))).toEqual([]);
  });
});
