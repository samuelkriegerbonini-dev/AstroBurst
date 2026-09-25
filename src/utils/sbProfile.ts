import type { PlotReferenceLine, ProfileSeries } from "../components/regions/ProfilePlot";
import type { SbBin, SbProfile } from "../shared/types/regions";
import { buildCsv, type CsvColumn } from "./catalogCsv";

export type SbYMode = "mean" | "mu" | "ee";
export type SbXUnit = "px" | "arcsec";

export const SB_Y_MODES: readonly SbYMode[] = ["mean", "mu", "ee"];
export const SB_X_UNITS: readonly SbXUnit[] = ["px", "arcsec"];

export interface SbSeriesLayout {
  series: ProfileSeries[];
  invertY: boolean;
  yLabel: string;
  xLabel: string;
}

const MEAN_COLOR = "#7dd3fc";
const MEDIAN_COLOR = "#fbbf24";
const MU_COLOR = "#f0abfc";
const EE_COLOR = "#86efac";
const R50_COLOR = "#86efac";
const R80_COLOR = "#fde68a";
const PETROSIAN_COLOR = "#fca5a5";
const RADIUS_PX_DIGITS = 1;
const RADIUS_ARCSEC_DIGITS = 2;
const PA_DIGITS = 1;
const NON_FINITE = "--";

function finiteOrNull(v: number | null | undefined): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function usesArcsec(profile: SbProfile, xUnit: SbXUnit): boolean {
  return xUnit === "arcsec" && finiteOrNull(profile.pixel_scale_arcsec) !== null;
}

function xOf(bin: SbBin, radiusPx: number, arcsec: boolean, scale: number | null): number {
  if (!arcsec) return radiusPx;
  if (radiusPx === bin.sma && finiteOrNull(bin.sma_arcsec) !== null) return bin.sma_arcsec as number;
  return radiusPx * (scale ?? 1);
}

function meanError(bin: SbBin): number | null {
  const std = finiteOrNull(bin.std);
  if (std === null || bin.count <= 0) return null;
  return std / Math.sqrt(bin.count);
}

export function sbTotalFlux(profile: SbProfile): number | null {
  const last = profile.bins[profile.bins.length - 1];
  const total = finiteOrNull(last?.cumulative_sum);
  return total !== null && total > 0 ? total : null;
}

export function sbSeries(profile: SbProfile, yMode: SbYMode, xUnit: SbXUnit): SbSeriesLayout {
  const arcsec = usesArcsec(profile, xUnit);
  const scale = finiteOrNull(profile.pixel_scale_arcsec);
  const xLabel = arcsec ? "semi-major axis (arcsec)" : "semi-major axis (px)";
  const bins = profile.bins;
  if (yMode === "mu") {
    return {
      series: [
        {
          x: bins.map((b) => xOf(b, b.sma, arcsec, scale)),
          y: bins.map((b) => finiteOrNull(b.mu_ab)),
          yErr: bins.map((b) => finiteOrNull(b.mu_err)),
          color: MU_COLOR,
          label: "mu_AB",
          mode: "both",
        },
      ],
      invertY: true,
      yLabel: "mu (mag/arcsec^2)",
      xLabel,
    };
  }
  if (yMode === "ee") {
    const total = sbTotalFlux(profile);
    return {
      series: [
        {
          x: bins.map((b) => xOf(b, b.sma_outer, arcsec, scale)),
          y: bins.map((b) => (total === null ? null : b.cumulative_sum / total)),
          color: EE_COLOR,
          label: "EE",
        },
      ],
      invertY: false,
      yLabel: "enclosed fraction",
      xLabel,
    };
  }
  const x = bins.map((b) => xOf(b, b.sma, arcsec, scale));
  return {
    series: [
      {
        x,
        y: bins.map((b) => finiteOrNull(b.mean)),
        yErr: bins.map(meanError),
        color: MEAN_COLOR,
        label: "mean",
        mode: "both",
      },
      { x, y: bins.map((b) => finiteOrNull(b.median)), color: MEDIAN_COLOR, label: "median" },
    ],
    invertY: false,
    yLabel: profile.background ? "mean - bg" : "mean",
    xLabel,
  };
}

export function sbReferenceLines(profile: SbProfile, xUnit: SbXUnit): PlotReferenceLine[] {
  const arcsec = usesArcsec(profile, xUnit);
  const factor = arcsec ? (profile.pixel_scale_arcsec as number) : 1;
  const candidates: [number | null, string, string][] = [
    [profile.r50_px, "R50", R50_COLOR],
    [profile.r80_px, "R80", R80_COLOR],
    [profile.petrosian_radius_px, "R_P", PETROSIAN_COLOR],
  ];
  const lines: PlotReferenceLine[] = [];
  for (const [px, label, color] of candidates) {
    const value = finiteOrNull(px);
    if (value === null) continue;
    lines.push({ axis: "x", value: value * factor, label, color, dashed: true });
  }
  return lines;
}

export const SB_CSV_COLUMNS: CsvColumn<SbBin>[] = [
  { header: "sma_px", value: (b) => b.sma },
  { header: "sma_arcsec", value: (b) => b.sma_arcsec },
  { header: "sma_inner", value: (b) => b.sma_inner },
  { header: "sma_outer", value: (b) => b.sma_outer },
  { header: "count", value: (b) => b.count },
  { header: "mean", value: (b) => b.mean },
  { header: "median", value: (b) => b.median },
  { header: "std", value: (b) => b.std },
  { header: "cumulative_sum", value: (b) => b.cumulative_sum },
  { header: "mu_ab", value: (b) => b.mu_ab },
  { header: "mu_err", value: (b) => b.mu_err },
  { header: "mag_ab_cumulative", value: (b) => b.mag_ab_cumulative },
];

export function sbProfileCsv(profile: SbProfile): string {
  return buildCsv(SB_CSV_COLUMNS, profile.bins);
}

export function formatRadius(px: number | null | undefined, scale: number | null | undefined): string {
  const radius = finiteOrNull(px);
  if (radius === null) return NON_FINITE;
  const base = `${radius.toFixed(RADIUS_PX_DIGITS)} px`;
  const arcsecPerPx = finiteOrNull(scale);
  if (arcsecPerPx === null || arcsecPerPx <= 0) return base;
  return `${base} (${(radius * arcsecPerPx).toFixed(RADIUS_ARCSEC_DIGITS)}")`;
}

export function formatPositionAngles(imageDeg: number, skyDeg: number | null | undefined): string {
  const image = `PA ${imageDeg.toFixed(PA_DIGITS)} deg image`;
  const sky = finiteOrNull(skyDeg);
  return sky === null ? image : `${image}, ${sky.toFixed(PA_DIGITS)} deg E of N`;
}

export function firstSurfaceBrightness(profile: SbProfile): number | null {
  return finiteOrNull(profile.bins[0]?.mu_ab);
}
