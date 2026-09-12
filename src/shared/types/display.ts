export type StretchMode = "mtf" | "linear" | "log" | "sqrt" | "asinh" | "power";
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
  | "rainbow";

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
];
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
};

export interface ScaleLimits {
  vmin: number;
  vmax: number;
  algorithm: string;
}

export interface ColormapLutResult {
  name: string;
  rgba: number[];
  colormaps: string[];
}
