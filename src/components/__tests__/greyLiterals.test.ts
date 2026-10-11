import { describe, expect, it } from "vitest";

const SOURCES = import.meta.glob<string>(
  [
    "../ui/Slider.tsx",
    "../header/HduSelectorPanel.tsx",
    "../header/HeaderExplorerPanel.tsx",
    "../preview/DqControls.tsx",
    "../CubeFrameNav.tsx",
    "../analysis/LineFitInspectPlot.tsx",
    "../analysis/FFTPanel.tsx",
    "../compose/steps/BackgroundStep.tsx",
    "../processing/WaveletPanel.tsx",
    "../stacking/SubframeSelectorPanel.tsx",
    "../processing/GeometryPanel.tsx",
  ],
  { query: "?raw", import: "default", eager: true },
);

const GREY_HEX = /#(71717a|52525b)\b/i;
const ZINC_700_TEXT = /(^|[\s"'`:])text-zinc-700\b/;
const DECORATIVE_ICON = /<(ChevronRight|GripVertical|ArrowRight|Grid3X3)\b/;

function greyLiteralLines(name: string, text: string): string[] {
  return text.split(/\r?\n/).flatMap((line, i) => {
    const informationalZinc = ZINC_700_TEXT.test(line) && !DECORATIVE_ICON.test(line);
    return GREY_HEX.test(line) || informationalZinc ? [`${name}:${i + 1}: ${line.trim()}`] : [];
  });
}

describe("grey literals in the token-lead files", () => {
  it("scans every listed file", () => {
    expect(Object.keys(SOURCES)).toHaveLength(11);
  });

  it("use the --ab-text-3/--ab-text-4 tokens instead of #71717a, #52525b or informational text-zinc-700", () => {
    const offenders = Object.entries(SOURCES).flatMap(([name, text]) => greyLiteralLines(name, text));
    expect(offenders).toEqual([]);
  });

  it("flags each literal form and spares a decorative icon", () => {
    const sample = [
      'style={{ color: "#71717A" }}',
      "const UNIT = \"#52525b\";",
      '<span className="text-[8px] text-zinc-700 font-mono">',
      '<input className="placeholder:text-zinc-700" />',
      '<ChevronRight size={10} className="text-zinc-700" />',
      '<div className="hover:bg-zinc-700/40 border-zinc-700/50">',
    ].join("\n");
    expect(greyLiteralLines("sample", sample).map((l) => l.split(":")[1])).toEqual(["1", "2", "3", "4"]);
  });
});
