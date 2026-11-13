import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ANALYSIS_SECTION, toolSections, type AnalysisSection, type AnalysisSectionsInput } from "../analysisSections";
import { tableRowAction } from "../fileKinds";

interface ScaffoldRow {
  key: string;
  tool: string;
  panel: string;
}

const SCAFFOLD: readonly ScaffoldRow[] = [
  { key: "tso", tool: "CubeTool.tsx", panel: "TsoLightCurvesPanel.tsx" },
  { key: "forced", tool: "PhotometryTool.tsx", panel: "ForcedPhotometryPanel.tsx" },
  { key: "depth", tool: "PhotometryTool.tsx", panel: "DepthPanel.tsx" },
  { key: "resolution", tool: "PhotometryTool.tsx", panel: "ResolutionPanel.tsx" },
  { key: "compare", tool: "PhotometryTool.tsx", panel: "CatalogComparePanel.tsx" },
  { key: "sed", tool: "CubeTool.tsx", panel: "SedPanel.tsx" },
];

const PANEL_BEFORE_THE_ROUND = "SedPanel.tsx";
const STUB_BODY = /function \w+\(_props: \w+\) \{\s*return null;\s*\}/;
const ANALYSIS_DIR = new URL("../../components/analysis/", import.meta.url);
const SECTIONS: Readonly<Record<string, AnalysisSection>> = ANALYSIS_SECTION;

const EVERYTHING: AnalysisSectionsInput = {
  hasHistogram: true,
  isCube: true,
  isRamp: true,
  isSpectrum: true,
  hasSpectrumDocument: true,
  hasSpectrumSeries: true,
  hasPhotometricPlanes: true,
  showFft: true,
  showDeepZoom: true,
};

const TABLE_ROUTE_KINDS = ["light_curve", "catalogue"] as const;
const ROUTE_PLUMBING = ["/hooks/useTableRouteStore.ts", "/utils/tableRoutes.ts", "/utils/fileKinds.ts", "/App.tsx"];
const SOURCES = import.meta.glob<string>("../../**/*.{ts,tsx}", { query: "?raw", import: "default", eager: true });

function analysisPath(file: string): string {
  return fileURLToPath(new URL(file, ANALYSIS_DIR));
}

function analysisSource(file: string): string {
  return readFileSync(analysisPath(file), "utf8");
}

function withoutExtension(file: string): string {
  return file.replace(/\.tsx$/, "");
}

const kept = SCAFFOLD.filter((row) => Object.hasOwn(SECTIONS, row.key));
const removed = SCAFFOLD.filter((row) => !Object.hasOwn(SECTIONS, row.key));

describe("scaffold of the round", () => {
  it("every section the round added has a panel that draws something", () => {
    for (const { key, panel } of kept) {
      expect(existsSync(analysisPath(panel)), `${key}: ${panel} is missing`).toBe(true);
      expect(STUB_BODY.test(analysisSource(panel)), `${key}: ${panel} still holds the stub body`).toBe(false);
    }
  });

  it("every section the round added is mounted in its tool", () => {
    for (const { key, tool, panel } of kept) {
      const source = analysisSource(tool);
      expect(source, `${key}: ${tool} does not mount its section`).toContain(`ANALYSIS_SECTION.${key}.id`);
      expect(source, `${key}: ${tool} does not import ${panel}`).toContain(withoutExtension(panel));
    }
  });

  it("a removed section leaves nothing behind", () => {
    for (const { key, tool, panel } of removed) {
      if (panel !== PANEL_BEFORE_THE_ROUND) expect(existsSync(analysisPath(panel)), `${key}: ${panel} is still there`).toBe(false);
      expect(analysisSource(tool), `${key}: ${tool} still names ${panel}`).not.toContain(withoutExtension(panel));
    }
  });

  it("the Photometry and Spectra tools list exactly the sections that exist", () => {
    const entries = Object.values(SECTIONS);
    for (const tool of ["photometry", "cube"] as const) {
      for (const section of toolSections(tool, EVERYTHING)) expect(entries, `${tool}: ${section.id}`).toContain(section);
    }
    for (const { key } of kept) {
      const section = SECTIONS[key];
      expect(toolSections(section.tool, EVERYTHING), key).toContain(section);
    }
  });

  it("a table row offers a button only for a panel that reads it", () => {
    const readers = Object.entries(SOURCES).filter(
      ([path]) => !path.includes("/__tests__/") && !ROUTE_PLUMBING.some((plumbing) => path.endsWith(plumbing)),
    );
    for (const kind of TABLE_ROUTE_KINDS) {
      if (tableRowAction({ format: "csv", guess: { kind, reason: "" } }) === null) continue;
      const reads = new RegExp(`useTableRoute\\(\\s*"${kind}"\\s*\\)|tableRouteStore\\.get\\(\\s*"${kind}"\\s*\\)`);
      const reading = readers.filter(([, source]) => reads.test(source)).map(([path]) => path);
      expect(reading, `${kind}: the row button has no panel that reads its route`).not.toHaveLength(0);
    }
  });

  it("the viewer mounts the spectrum component", () => {
    const preview = readFileSync(fileURLToPath(new URL("../../components/PreviewPanel.tsx", import.meta.url)), "utf8");
    expect(preview).toContain("<SpectrumViewer");
  });
});
