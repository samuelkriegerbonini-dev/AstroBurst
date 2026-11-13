import { describe, expect, it } from "vitest";
import pvPanelSource from "../PvPanel.tsx?raw";
import rampPanelSource from "../RampPanel.tsx?raw";
import rampComparePanelSource from "../RampComparePanel.tsx?raw";
import contourPanelSource from "../ContourPanel.tsx?raw";
import targetsPanelSource from "../TargetsPanel.tsx?raw";
import { adHocErrorBoxLines, localSmallButtonLines } from "../../../test/panelRules";

type Rgb = [number, number, number];

const PANELS: Array<[string, string]> = [
  ["PvPanel.tsx", pvPanelSource],
  ["RampPanel.tsx", rampPanelSource],
  ["RampComparePanel.tsx", rampComparePanelSource],
  ["ContourPanel.tsx", contourPanelSource],
  ["TargetsPanel.tsx", targetsPanelSource],
];

const PANEL_BACKGROUND: Rgb = [11, 11, 26];

function compositedConstant(source: string, name: string): Rgb {
  const match = new RegExp(`const ${name} = "rgba\\((\\d+),(\\d+),(\\d+),([\\d.]+)\\)";`).exec(source);
  if (!match) throw new Error(`no rgba literal for ${name}`);
  const alpha = Number(match[4]);
  return [1, 2, 3].map((i, k) => Number(match[i]) * alpha + PANEL_BACKGROUND[k] * (1 - alpha)) as Rgb;
}

function luminance([r, g, b]: Rgb): number {
  const lin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

describe("panels of the general analysis area", () => {
  it.each(PANELS)("keeps no private copy of the small button class in %s", (_name, source) => {
    expect(localSmallButtonLines(source)).toEqual([]);
    expect(source).toMatch(/import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/);
  });

  it.each(PANELS)("holds no hand-made failure box in %s", (_name, source) => {
    expect(adHocErrorBoxLines(source)).toEqual([]);
  });

  it("draws the ramp groups left out at 3:1 or more against the panel and apart from the raw groups", () => {
    const excluded = compositedConstant(rampPanelSource, "EXCLUDED_COLOR");
    const raw = compositedConstant(rampPanelSource, "RAW_COLOR");
    expect(contrast(excluded, PANEL_BACKGROUND)).toBeGreaterThanOrEqual(3);
    expect(contrast(excluded, raw)).toBeGreaterThanOrEqual(1.5);
  });
});
