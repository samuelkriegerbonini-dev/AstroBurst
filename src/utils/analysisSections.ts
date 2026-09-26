export const DEEP_ZOOM_MIN_SIDE = 4096;

export function deepZoomAvailable(width: number | undefined, height: number | undefined): boolean {
  return (width ?? 0) > DEEP_ZOOM_MIN_SIDE || (height ?? 0) > DEEP_ZOOM_MIN_SIDE;
}

export interface AnalysisSection {
  id: string;
  label: string;
}

export interface AnalysisSectionsInput {
  hasHistogram: boolean;
  isCube: boolean;
  showFft: boolean;
  showDeepZoom: boolean;
}

export const ANALYSIS_SECTION = {
  histogram: { id: "analysis-histogram", label: "Histogram" },
  stars: { id: "analysis-stars", label: "Stars" },
  photometry: { id: "analysis-photometry", label: "Photometry" },
  table: { id: "analysis-table", label: "Table" },
  series: { id: "analysis-series", label: "Series" },
  geometry: { id: "analysis-geometry", label: "Geometry" },
  catalog: { id: "analysis-catalog", label: "Catalog" },
  targets: { id: "analysis-targets", label: "Targets" },
  statistics: { id: "analysis-statistics", label: "Stats" },
  pixels: { id: "analysis-pixels", label: "Pixels" },
  regions: { id: "analysis-regions", label: "Regions" },
  profiles: { id: "analysis-profiles", label: "Profiles" },
  contours: { id: "analysis-contours", label: "Contours" },
  fft: { id: "analysis-fft", label: "FFT" },
  spectrum: { id: "analysis-spectrum", label: "Spectrum" },
  pv: { id: "analysis-pv", label: "PV" },
  deepZoom: { id: "analysis-deep-zoom", label: "Deep Zoom" },
  log: { id: "analysis-log", label: "Log" },
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
    ...(input.isCube ? [s.spectrum, s.pv] : []),
    ...(input.showDeepZoom ? [s.deepZoom] : []),
    s.log,
  ];
}
