import { describe, expect, it } from "vitest";
import { adHocErrorBoxLines, localSmallButtonLines } from "../../../test/panelRules";
import pixelTableSource from "../PixelTablePanel.tsx?raw";
import measurementLogSource from "../MeasurementLogPanel.tsx?raw";
import catalogSource from "../CatalogPanel.tsx?raw";

const PANELS: [string, string][] = [
  ["PixelTablePanel.tsx", pixelTableSource],
  ["MeasurementLogPanel.tsx", measurementLogSource],
  ["CatalogPanel.tsx", catalogSource],
];

const SHARED_SMALL_BUTTON_IMPORT = /import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/;

describe("shared primitives in the pixel table, log and catalogue panels", () => {
  it.each(PANELS)("keeps no private copy of the small button class in %s", (_name, source) => {
    expect(localSmallButtonLines(source)).toEqual([]);
    expect(source).toMatch(SHARED_SMALL_BUTTON_IMPORT);
  });

  it("uses no one-off button class in the pixel table", () => {
    expect(pixelTableSource).not.toMatch(/^const BUTTON_CLASS\s*=/m);
  });

  it.each(PANELS)("holds no hand-made failure box in %s", (_name, source) => {
    expect(adHocErrorBoxLines(source)).toEqual([]);
  });

  it("confirms the log and the catalogue copies through the shared button", () => {
    for (const source of [measurementLogSource, catalogSource]) {
      expect(source).toMatch(/<CopyCsvButton\b/);
      expect(source).not.toContain("navigator.clipboard.writeText(");
    }
  });
});
