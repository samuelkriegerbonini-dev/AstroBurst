export type LayerRole = "science" | "sigma" | "variance" | "inverse_variance" | "mask" | "other";

export type CompanionOrigin = "same_file" | "sibling" | "user" | "recipe";

export interface LayerGuess {
  role: LayerRole;
  basis: string;
  assumed: boolean;
}

export interface ErrOrigin {
  role: LayerRole;
  source: string;
  assumed: boolean;
  n_no_data: number | null;
  origin?: CompanionOrigin;
}

export interface PixelTableLayers {
  err_source?: string | null;
  dq_source?: string | null;
  err_unavailable?: string | null;
  dq_unavailable?: string | null;
}

export type CompanionRole = "err" | "dq";

export type ErrorLayerRole = "sigma" | "variance" | "inverse_variance";

export interface ErrCandidate {
  source: string;
  role: LayerRole;
  basis: string;
  usable: boolean;
  reason: string | null;
}

export interface DqCandidate {
  source: string;
  basis: string;
  usable: boolean;
  reason: string | null;
}

export interface CompanionSuggestions {
  role: LayerGuess;
  err: { source: string; role: LayerRole; basis: string } | null;
  dq: { source: string; basis: string } | null;
  applied: boolean;
  candidates: { err: ErrCandidate[]; dq: DqCandidate[] };
  notes: string[];
}

export interface ErrRecipe {
  kind: "sdss_frame";
  gain: number | null;
  dark_variance: number | null;
}

export interface BandPlaneInfo {
  hdu: number;
  plane: number;
  label: string | null;
}

export interface BandPlaneEntry {
  plane: number;
  label: string | null;
  key: string;
}
