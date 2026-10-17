export type MastMissionId =
  | "jwst" | "hst" | "roman" | "roman_cgi" | "roman_spectra"
  | "iue" | "classy" | "ullyses" | "tess" | "multi_mission";

export type MastAvailability = "open" | "token" | "not_deployed" | "unsupported";

export interface MastQuickFilter {
  id: "level" | "instrument" | "filter" | "date" | "public";
  label: string;
  column: string;
  options: string[] | null;
}

export interface MastColumn {
  name: string;
  label: string;
  kind: "text" | "int" | "float" | "datetime";
  sortable: boolean;
  unit: string | null;
}

export interface MastMission {
  id: MastMissionId;
  label: string;
  availability: MastAvailability;
  capabilities: { search: boolean; images: boolean; products: boolean };
  views: { gallery: boolean; table: boolean };
  quickFilters: MastQuickFilter[];
  defaultColumns: MastColumn[];
  note: string | null;
  checkedOn: string;
}

export interface MastResolved {
  query: string;
  canonicalName: string;
  resolver: string;
  raDeg: number;
  decDeg: number;
  objectType: string | null;
  elapsedMs: number;
  fromCache: boolean;
}

export type MastWhere =
  | { kind: "none" }
  | { kind: "position"; raDeg: number; decDeg: number; radiusArcmin: number; label: string | null }
  | { kind: "name"; name: string; radiusArcmin: number };

export interface MastFilterValues {
  level?: "combined" | "all";
  instrument?: string;
  filter?: string;
  dateFrom?: string;
  dateTo?: string;
  publicOnly?: boolean;
}

export interface MastQuery {
  mission: MastMissionId;
  where: MastWhere;
  program: number | null;
  datasetId: string | null;
  filters: MastFilterValues;
  sort: { column: string; desc: boolean }[];
  offset: number;
  limit: number;
}

export type MastLevelClass = "combined" | "exposure" | "unknown";

export interface MastRow {
  datasetId: string;
  levelClass: MastLevelClass;
  level: string | null;
  instrument: string | null;
  filter: string | null;
  filters: string[];
  expType: string | null;
  target: string | null;
  program: string | null;
  observation: string | null;
  dateObs: string | null;
  expTimeS: number | null;
  sepArcmin: number | null;
  sepVerified: boolean;
  access: string | null;
  releaseDate: string | null;
  raDeg: number | null;
  decDeg: number | null;
  sRegion: string | null;
  values: Record<string, string | number | boolean | null>;
}

export interface MastSearchPage {
  requestId: string;
  mission: MastMissionId;
  columns: MastColumn[];
  rows: MastRow[];
  offset: number;
  limit: number;
  hasMore: boolean;
  totalReported: number | null;
  totalReliable: boolean;
  radiusArcmin: number | null;
  radiusClamped: boolean;
  searchPosition: { raDeg: number; decDeg: number } | null;
  applied: { conditions: Record<string, string>[]; sortBy: string[]; sortDesc: boolean[] };
  requestBody: unknown;
  messages: string[];
  unresolvedTargets: string[];
  elapsedMs: number;
  fromCache: boolean;
}

export interface MastImageEntry {
  key: string;
  label: string;
  group: string;
  productFilename: string;
  thumbnailFilename: string | null;
  previewFilename: string | null;
  level: string | null;
  suffix: string;
  detector: string | null;
  filter: string | null;
  scienceGroup: string | null;
  colour: boolean;
  mastUri: string;
}

export interface MastImageSet {
  mission: MastMissionId;
  datasetId: string;
  entries: MastImageEntry[];
  groups: { key: string; label: string; count: number }[];
  defaultGroup: string;
  representative: number | null;
  elapsedMs: number;
  fromCache: boolean;
}

export interface MastQuickWcs {
  centreRaDeg: number;
  centreDecDeg: number;
  pixelScaleArcsec: number;
  fieldArcmin: [number, number];
  orientation: "cd_matrix" | "unreliable";
  ctype: string;
}

export interface MastImageRef {
  requestId: string;
  path: string;
  width: number;
  height: number;
  bytes: number;
  filename: string;
  wcs: MastQuickWcs | null;
  imageCount: number | null;
  elapsedMs: number;
  fromCache: boolean;
}

export interface MastPreviewRef extends MastImageRef {
  size: "pane" | "large";
  channels: 1 | 3;
  source: { width: number; height: number; bytes: number };
}

export type MastProductClass =
  | "image" | "cube" | "spectrum" | "ramp" | "catalog" | "preview"
  | "association" | "auxiliary" | "reference" | "guidestar" | "other";

export interface MastProduct {
  datasetId: string;
  filename: string;
  sizeBytes: number;
  category: string;
  type: string;
  suffix: string;
  cls: MastProductClass;
  opensAs: "image" | "cube" | "spectrum" | "ramp" | "catalog" | null;
  access: string;
  mastUri: string;
  downloadUrl: string;
  badges: "hap_aligned"[];
}

export interface MastProductList {
  mission: MastMissionId;
  datasetIds: string[];
  products: MastProduct[];
  groups: { cls: MastProductClass; count: number; bytes: number; defaultVisible: boolean }[];
  totalCount: number;
  totalBytes: number;
  elapsedMs: number;
  fromCache: boolean;
  cachedUntil: string | null;
}

export interface MastTokenStatus {
  present: boolean;
  authorized: boolean | null;
}

export interface MastCacheInfo {
  bytes: number;
  files: number;
}

export interface MastProgressEvent {
  requestId: string;
  received: number;
  total: number | null;
  stage: "download" | "decode" | "ready";
}

export type MastErrorCode =
  | "mast_network" | "mast_timeout" | "mast_cancelled" | "mast_rate_limited"
  | "mast_server" | "mast_rejected" | "mast_unauthorized" | "mast_no_image"
  | "mast_not_deployed" | "mast_decode" | "mast_too_large" | "mast_not_image"
  | "mast_bad_input" | "mast_token_rejected" | "mast_cache";

export interface MastErrorContext {
  status?: number;
  retryAfterSec?: number;
  detail?: string;
  mission?: MastMissionId;
  operation?: string;
  timeoutSec?: number;
  bytes?: number;
  consecutive?: number;
  attempts?: number;
  elapsedMs?: number;
  host?: string;
}

export interface MastError {
  code: MastErrorCode;
  message: string;
  context: MastErrorContext;
}
