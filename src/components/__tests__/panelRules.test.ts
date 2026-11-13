import { describe, expect, it } from "vitest";
import { adHocErrorBoxLines, localSmallButtonLines } from "../../test/panelRules";

describe("panel source rules", () => {
  it("flags a hand-made red failure box and spares the shared alert, a verdict chip and plain red text", () => {
    const sample = [
      '<div className="text-[10px] text-red-400 bg-red-900/20 border border-red-800/30 rounded px-2.5 py-1.5 break-words">',
      '<p className="text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20">{error}</p>',
      'const ERROR_CLASS = "text-[10px] text-red-400/80 px-2 py-1 rounded bg-red-900/15 border border-red-800/20";',
      "<ErrorAlert compact message={error} />",
      '<div role="alert" className="text-[10px] text-red-400">',
      ': "text-red-300 bg-red-900/20 border-red-700/30"',
      '<div className={ERROR_ALERT_COMPACT_CLASS} role="alert">',
    ].join("\r\n");
    expect(adHocErrorBoxLines(sample)).toEqual([1, 2, 3]);
    expect(adHocErrorBoxLines("")).toEqual([]);
  });

  it("flags a private copy of the small button class and spares its users", () => {
    const sample = [
      "const SMALL_BUTTON_CLASS =",
      '  "flex items-center gap-1 px-2 py-1 rounded text-[10px]";',
      "const LOG_BUTTON_CLASS = `${SMALL_BUTTON_CLASS} inline-flex`;",
      'import { ErrorAlert, SMALL_BUTTON_CLASS } from "../ui";',
    ].join("\n");
    expect(localSmallButtonLines(sample)).toEqual([1]);
  });
});

const PANEL_SOURCES = import.meta.glob<string>(["../analysis/**/*.tsx", "../regions/**/*.tsx"], { query: "?raw", import: "default", eager: true });

function panelFile(key: string): string {
  return key.replace(/^\.\.\//, "");
}

function sitesOf(lines: (source: string) => number[]): string[] {
  return Object.entries(PANEL_SOURCES).flatMap(([key, source]) => lines(source).map((line) => `${panelFile(key)}:${line}`));
}

describe("every analysis and regions panel", () => {
  it("holds no hand-made failure box", () => {
    expect(sitesOf(adHocErrorBoxLines)).toEqual([]);
  });

  it("keeps no private copy of the small button class", () => {
    expect(sitesOf(localSmallButtonLines)).toEqual([]);
  });

  it("confirms every table copy through the shared button", () => {
    const files = [
      "analysis/MeasurementLogPanel.tsx",
      "analysis/TimeSeriesPanel.tsx",
      "analysis/PhotometryTablePanel.tsx",
      "analysis/CatalogPanel.tsx",
      "analysis/PvPanel.tsx",
      "analysis/TransitFitSection.tsx",
      "regions/RegionsPanel.tsx",
    ];
    const sources = new Map(Object.entries(PANEL_SOURCES).map(([key, source]) => [panelFile(key), source]));
    expect(files.filter((file) => !/<CopyCsvButton\b/.test(sources.get(file) ?? ""))).toEqual([]);
  });
});
