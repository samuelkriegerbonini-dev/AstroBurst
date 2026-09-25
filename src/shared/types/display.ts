import type { SkyFrame } from "./astrometry";

export type StretchMode = "mtf" | "linear" | "log" | "sqrt" | "asinh" | "power";
export type GridFrame = SkyFrame;
export const GRID_FRAMES: readonly GridFrame[] = ["icrs", "fk5", "galactic", "ecliptic"];
export const GRID_DENSITY_MIN = 1;
export const GRID_DENSITY_MAX = 5;
export const GRID_DENSITY_DEFAULT = 3;
export const GRID_DENSITIES: readonly number[] = [1, 2, 3, 4, 5];
export type LimitMode = "minmax" | "zscale" | "percentile" | "user";
export type ColormapName =
  | "gray"
  | "viridis"
  | "inferno"
  | "magma"
  | "plasma"
  | "cividis"
  | "heat"
  | "cool"
  | "rainbow"
  | "rdbu"
  | "coolwarm"
  | "bwr"
  | "seismic";

export const COLORMAP_NAMES: readonly ColormapName[] = [
  "gray",
  "viridis",
  "inferno",
  "magma",
  "plasma",
  "cividis",
  "heat",
  "cool",
  "rainbow",
  "rdbu",
  "coolwarm",
  "bwr",
  "seismic",
];

export const COLORMAP_LABELS: Readonly<Record<ColormapName, string>> = {
  gray: "gray",
  viridis: "viridis",
  inferno: "inferno",
  magma: "magma",
  plasma: "plasma",
  cividis: "cividis",
  heat: "heat",
  cool: "cool",
  rainbow: "rainbow",
  rdbu: "RdBu",
  coolwarm: "coolwarm",
  bwr: "bwr",
  seismic: "seismic",
};
export const STRETCH_MODES: readonly StretchMode[] = ["mtf", "linear", "log", "sqrt", "asinh", "power"];
export const LIMIT_MODES: readonly LimitMode[] = ["minmax", "zscale", "percentile", "user"];

export interface DisplaySettings {
  stretch: StretchMode;
  limits: LimitMode;
  percentileLow: number;
  percentileHigh: number;
  userLo: number | null;
  userHi: number | null;
  zscaleContrast: number;
  asinhA: number;
  power: number;
  colormap: ColormapName;
  invert: boolean;
  grid: boolean;
  gridFrame: GridFrame;
  gridDensity: number;
  compass: boolean;
  symmetric: boolean;
  centre: number;
}

export const DEFAULT_DISPLAY_SETTINGS: DisplaySettings = {
  stretch: "mtf",
  limits: "minmax",
  percentileLow: 1,
  percentileHigh: 99.5,
  userLo: null,
  userHi: null,
  zscaleContrast: 0.25,
  asinhA: 0.1,
  power: 2,
  colormap: "gray",
  invert: false,
  grid: false,
  gridFrame: "icrs",
  gridDensity: GRID_DENSITY_DEFAULT,
  compass: false,
  symmetric: false,
  centre: 0,
};

export interface ScaleLimits {
  vmin: number;
  vmax: number;
  algorithm: string;
  symmetric: boolean;
  centre: number | null;
  notes: string[];
}

export interface ColormapLutResult {
  name: string;
  rgba: number[];
  nodata: number[];
  colormaps: string[];
}
