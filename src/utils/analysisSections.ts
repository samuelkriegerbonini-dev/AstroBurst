export const DEEP_ZOOM_MIN_SIDE = 4096;

export function deepZoomAvailable(width: number | undefined, height: number | undefined): boolean {
  return (width ?? 0) > DEEP_ZOOM_MIN_SIDE || (height ?? 0) > DEEP_ZOOM_MIN_SIDE;
}

export type AnalysisTabId = "image" | "sources" | "cube";

export interface AnalysisTab {
  id: AnalysisTabId;
  label: string;
}

export const ANALYSIS_TABS: readonly AnalysisTab[] = [
  { id: "image", label: "Image" },
  { id: "sources", label: "Sources" },
  { id: "cube", label: "Cube" },
];

export const CUBE_TAB_UNAVAILABLE_TITLE = "Open a data cube or a ramp";

export interface AnalysisSection {
  id: string;
  label: string;
  tab: AnalysisTabId | null;
}

export interface AnalysisSectionsInput {
  hasHistogram: boolean;
  isCube: boolean;
  isRamp?: boolean;
  showFft: boolean;
  showDeepZoom: boolean;
}

export const ANALYSIS_SECTION = {
  histogram: { id: "analysis-histogram", label: "Histogram", tab: "image" },
  stars: { id: "analysis-stars", label: "Stars", tab: "sources" },
  photometry: { id: "analysis-photometry", label: "Photometry", tab: "sources" },
  table: { id: "analysis-table", label: "Table", tab: "sources" },
  series: { id: "analysis-series", label: "Series", tab: "sources" },
  geometry: { id: "analysis-geometry", label: "Geometry", tab: "sources" },
  catalog: { id: "analysis-catalog", label: "Catalog", tab: "sources" },
  targets: { id: "analysis-targets", label: "Targets", tab: "sources" },
  statistics: { id: "analysis-statistics", label: "Stats", tab: "image" },
  pixels: { id: "analysis-pixels", label: "Pixels", tab: "image" },
  regions: { id: "analysis-regions", label: "Regions", tab: "image" },
  profiles: { id: "analysis-profiles", label: "Profiles", tab: "image" },
  contours: { id: "analysis-contours", label: "Contours", tab: "image" },
  fft: { id: "analysis-fft", label: "FFT", tab: "image" },
  ramp: { id: "analysis-ramp", label: "Ramp", tab: "cube" },
  spectrum: { id: "analysis-spectrum", label: "Spectrum", tab: "cube" },
  pv: { id: "analysis-pv", label: "PV", tab: "cube" },
  deepZoom: { id: "analysis-deep-zoom", label: "Deep Zoom", tab: "image" },
  log: { id: "analysis-log", label: "Log", tab: null },
} as const satisfies Record<string, AnalysisSection>;

export function analysisSections(input: AnalysisSectionsInput): AnalysisSection[] {
  const s = ANALYSIS_SECTION;
  return [
    ...(input.hasHistogram ? [s.histogram] : []),
    s.stars,
    s.photometry,
    s.table,
    s.series,
    s.geometry,
    s.catalog,
    s.targets,
    s.statistics,
    s.pixels,
    s.regions,
    s.profiles,
    s.contours,
    ...(input.showFft ? [s.fft] : []),
    ...(input.isRamp ? [s.ramp] : []),
    ...(input.isCube ? [s.spectrum, s.pv] : []),
    ...(input.showDeepZoom ? [s.deepZoom] : []),
    s.log,
  ];
}

export function analysisTabSections(input: AnalysisSectionsInput, tab: AnalysisTabId): AnalysisSection[] {
  return analysisSections(input).filter((s) => s.tab === tab);
}

export function defaultAnalysisTab(input: { isCube: boolean; isRamp?: boolean }): AnalysisTabId {
  return input.isCube || input.isRamp ? "cube" : "image";
}

export type AnalysisTabAvailability = Record<AnalysisTabId, boolean>;

export function tabAvailability(input: AnalysisSectionsInput): AnalysisTabAvailability {
  const sections = analysisSections(input);
  const has = (tab: AnalysisTabId) => sections.some((s) => s.tab === tab);
  return { image: has("image"), sources: has("sources"), cube: has("cube") };
}

export function resolveAnalysisTab(remembered: AnalysisTabId | undefined, input: AnalysisSectionsInput): AnalysisTabId {
  if (remembered !== undefined && tabAvailability(input)[remembered]) return remembered;
  return defaultAnalysisTab(input);
}

export interface AnalysisTabMemory {
  resolve(fileKey: string | null, input: AnalysisSectionsInput): AnalysisTabId;
  remember(fileKey: string | null, tab: AnalysisTabId): void;
}

export function createAnalysisTabMemory(): AnalysisTabMemory {
  const chosen = new Map<string, AnalysisTabId>();
  return {
    resolve: (fileKey, input) => resolveAnalysisTab(chosen.get(fileKey ?? ""), input),
    remember: (fileKey, tab) => {
      chosen.set(fileKey ?? "", tab);
    },
  };
}

export type TabNavKey = "ArrowLeft" | "ArrowRight" | "Home" | "End";

export function isTabNavKey(key: string): key is TabNavKey {
  return key === "ArrowLeft" || key === "ArrowRight" || key === "Home" || key === "End";
}

export function nextAnalysisTab(current: AnalysisTabId, key: TabNavKey, available: AnalysisTabAvailability): AnalysisTabId {
  const enabled = ANALYSIS_TABS.map((t) => t.id).filter((id) => available[id]);
  if (enabled.length === 0) return current;
  if (key === "Home") return enabled[0];
  if (key === "End") return enabled[enabled.length - 1];
  const at = enabled.indexOf(current);
  const step = key === "ArrowRight" ? 1 : -1;
  if (at < 0) return step > 0 ? enabled[0] : enabled[enabled.length - 1];
  return enabled[(at + step + enabled.length) % enabled.length];
}

export interface AnalysisPanelAttributes {
  role: "tabpanel";
  id: string;
  "aria-labelledby": string;
  "data-analysis-panel": AnalysisTabId;
  hidden: boolean;
}

export function analysisPanelAttributes(tab: AnalysisTabId, activeTab: AnalysisTabId): AnalysisPanelAttributes {
  return {
    role: "tabpanel",
    id: `analysis-panel-${tab}`,
    "aria-labelledby": `analysis-tab-${tab}`,
    "data-analysis-panel": tab,
    hidden: tab !== activeTab,
  };
}
