export type LightCurveTimeScale = "bmjd_tdb" | "bjd_tdb" | "hjd_utc" | "jd_utc" | "jd_header" | "mjd_utc" | "index";

export type LightCurveQuantity = "diff_mag" | "flux" | "rel_flux";

export type LightCurveSource =
  | { kind: "frames"; reference: string; target: string; comps: string[] }
  | {
      kind: "tso";
      paths: string[];
      detector: string;
      sporder: number | null;
      band_um: [number, number] | null;
      mode: "pipeline" | "masked";
    }
  | { kind: "table"; path: string; time_column: string; value_column: string };

export interface LightCurveDoc {
  id: string;
  label: string;
  quantity: LightCurveQuantity;
  unit: string | null;
  timeScale: LightCurveTimeScale;
  time: number[];
  value: (number | null)[];
  err: (number | null)[];
  pointRef: number[];
  flags: string[][];
  source: LightCurveSource;
  notes: string[];
  timeLabel?: string;
  overlay?: string;
  magErrFactor?: number;
}

export interface LightCurveWindow {
  start: number;
  end: number;
}

export type LightCurveBin = { kind: "none" } | { kind: "points"; points: number } | { kind: "minutes"; minutes: number };

export interface LightCurveViewState {
  selection: string | null;
  display: "native" | "normalised";
  window: LightCurveWindow | null;
  marginDays: number;
  baselineKind: "mean" | "median" | "linear";
  bin: LightCurveBin;
  clipSigma: number | null;
  clipWindowPoints: number;
  excludeFlagged: boolean;
  excluded: number[];
  clickExcludes: boolean;
  fold: { periodDays: number; t0: number } | null;
}

export const DEFAULT_LIGHT_CURVE_VIEW: LightCurveViewState = Object.freeze({
  selection: null,
  display: "native",
  window: null,
  marginDays: 0,
  baselineKind: "mean",
  bin: { kind: "none" },
  clipSigma: null,
  clipWindowPoints: 11,
  excludeFlagged: true,
  excluded: [],
  clickExcludes: false,
  fold: null,
}) as LightCurveViewState;

export interface LightCurveQuickLook {
  docId: string;
  label: string;
  source: LightCurveSource;
  quantity: LightCurveQuantity;
  unit: string | null;
  timeScale: LightCurveTimeScale;
  timeName: string;
  nPoints: number;
  window: LightCurveWindow;
  marginDays: number;
  baselineKind: "mean" | "median" | "linear";
  depth: number;
  errScatter: number | null;
  errFormal: number | null;
  err: number | null;
  nIn: number;
  nOut: number;
  scatterPpm: number | null;
  formalPpm: number | null;
  scatterRatio: number | null;
  binned: string | null;
  nExcluded: number;
  nClipped: number;
  notes: string[];
}
