import { DOCK_ANCHORS, type DockAnchor, type DockLayout, type DockToolId } from "./dockLayout";

export const DEEP_ZOOM_MIN_SIDE = 4096;

export function deepZoomAvailable(width: number | undefined, height: number | undefined): boolean {
  return (width ?? 0) > DEEP_ZOOM_MIN_SIDE || (height ?? 0) > DEEP_ZOOM_MIN_SIDE;
}

export type AnalysisToolId = Extract<DockToolId, "image" | "astrometry" | "photometry" | "cube" | "log">;

export const ANALYSIS_TOOL_IDS: readonly AnalysisToolId[] = ["image", "astrometry", "photometry", "cube", "log"];

export const CUBE_TOOL_EMPTY_TITLE = "Open a data cube or a ramp";

export const MIN_CHIP_SECTIONS = 4;

export interface AnalysisSection {
  id: string;
  label: string;
  tool: AnalysisToolId;
}

export interface AnalysisSectionsInput {
  hasHistogram: boolean;
  isCube: boolean;
  isRamp?: boolean;
  showFft: boolean;
  showDeepZoom: boolean;
}

export const ANALYSIS_SECTION = {
  histogram: { id: "analysis-histogram", label: "Histogram", tool: "image" },
  stars: { id: "analysis-stars", label: "Stars", tool: "astrometry" },
  photometry: { id: "analysis-photometry", label: "Photometry", tool: "photometry" },
  table: { id: "analysis-table", label: "Table", tool: "photometry" },
  series: { id: "analysis-series", label: "Series", tool: "photometry" },
  geometry: { id: "analysis-geometry", label: "Geometry", tool: "astrometry" },
  catalog: { id: "analysis-catalog", label: "Catalog", tool: "astrometry" },
  targets: { id: "analysis-targets", label: "Targets", tool: "astrometry" },
  statistics: { id: "analysis-statistics", label: "Stats", tool: "image" },
  pixels: { id: "analysis-pixels", label: "Pixels", tool: "image" },
  regions: { id: "analysis-regions", label: "Regions", tool: "image" },
  profiles: { id: "analysis-profiles", label: "Profiles", tool: "image" },
  contours: { id: "analysis-contours", label: "Contours", tool: "image" },
  fft: { id: "analysis-fft", label: "FFT", tool: "image" },
  ramp: { id: "analysis-ramp", label: "Ramp", tool: "cube" },
  spectrum: { id: "analysis-spectrum", label: "Spectrum", tool: "cube" },
  pv: { id: "analysis-pv", label: "PV", tool: "cube" },
  deepZoom: { id: "analysis-deep-zoom", label: "Deep Zoom", tool: "image" },
  log: { id: "analysis-log", label: "Log", tool: "log" },
} as const satisfies Record<string, AnalysisSection>;

export function toolSections(tool: AnalysisToolId, input: AnalysisSectionsInput): AnalysisSection[] {
  const s = ANALYSIS_SECTION;
  switch (tool) {
    case "image":
      return [
        ...(input.hasHistogram ? [s.histogram] : []),
        s.statistics,
        s.pixels,
        s.regions,
        s.profiles,
        s.contours,
        ...(input.showFft ? [s.fft] : []),
        ...(input.showDeepZoom ? [s.deepZoom] : []),
      ];
    case "astrometry":
      return [s.stars, s.geometry, s.catalog, s.targets];
    case "photometry":
      return [s.photometry, s.table, s.series];
    case "cube":
      return [...(input.isRamp ? [s.ramp] : []), ...(input.isCube ? [s.spectrum] : []), ...(input.isCube && !input.isRamp ? [s.pv] : [])];
    case "log":
      return [s.log];
  }
}

export function showSectionChips(sections: readonly AnalysisSection[]): boolean {
  return sections.length >= MIN_CHIP_SECTIONS;
}

export type AnalysisRouteTool = "image" | "cube";

export function analysisToolForFile(input: { isCube: boolean; isRamp?: boolean }): AnalysisRouteTool {
  return input.isCube || input.isRamp ? "cube" : "image";
}

export interface AnalysisRouteInput {
  layout: Pick<DockLayout, "anchors" | "active">;
  fileKey: string | null;
  isCube: boolean;
  isRamp: boolean;
  memory: ReadonlyMap<string, AnalysisRouteTool>;
}

export interface AnalysisRouteStep {
  anchor: DockAnchor;
  tool: AnalysisRouteTool;
}

function isRouteTool(tool: DockToolId | null): tool is AnalysisRouteTool {
  return tool === "image" || tool === "cube";
}

function openRouteTools(open: DockLayout["active"]): AnalysisRouteTool[] {
  return DOCK_ANCHORS.map((anchor) => open[anchor]).filter(isRouteTool);
}

export function nextAnalysisRoute({ layout, fileKey, isCube, isRamp, memory }: AnalysisRouteInput): AnalysisRouteStep[] {
  if (fileKey === null) return [];
  const open = openRouteTools(layout.active);
  if (open.length === 0) return [];
  const wanted = memory.get(fileKey) ?? analysisToolForFile({ isCube, isRamp });
  const tool = wanted === "cube" && !isCube && !isRamp ? "image" : wanted;
  if (open.includes(tool)) return [];
  const anchor = DOCK_ANCHORS.find((a) => layout.anchors[a].includes(tool));
  return anchor === undefined ? [] : [{ anchor, tool }];
}

export function rememberAnalysisChoice(memory: Map<string, AnalysisRouteTool>, fileKey: string | null, before: DockLayout["active"], after: DockLayout["active"]): void {
  if (fileKey === null || before === after) return;
  const wasOpen = openRouteTools(before);
  const opened = openRouteTools(after).find((tool) => !wasOpen.includes(tool));
  if (opened !== undefined) memory.set(fileKey, opened);
}

export function analysisRouteDue({ fileKey, flagsKey, routedKey }: { fileKey: string | null; flagsKey: string | null; routedKey: string | null }): boolean {
  return fileKey !== null && fileKey !== routedKey && flagsKey === fileKey;
}

export function analysisSections(input: AnalysisSectionsInput): AnalysisSection[] {
  return ANALYSIS_TOOL_IDS.flatMap((tool) => toolSections(tool, input));
}
