import type { PlotReferenceLine, ProfileSeries } from "../components/regions/ProfilePlot";
import type { SbBin, SbProfile } from "../shared/types/regions";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "./catalogCsv";
import { provenanceLines, summedPixelUnit, unitLines, withProvenance } from "./csvProvenance";
import { parseDecimalText } from "./decimalText";
import type { CsvContext } from "./depthExport";

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
export const PSF_LINE_COLOR = "#c4b5fd";
const RADIUS_PX_DIGITS = 1;
const RADIUS_ARCSEC_DIGITS = 2;
const PA_DIGITS = 1;
const NON_FINITE = "--";
const MIN_PIXELS_FOR_ERROR = 2;

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

export function binMeanError(std: number | null | undefined, count: number): number | null {
  const spread = finiteOrNull(std);
  if (spread === null || count < MIN_PIXELS_FOR_ERROR) return null;
  return spread / Math.sqrt(count);
}

function meanError(bin: SbBin): number | null {
  return binMeanError(bin.std, bin.count);
}

function surfaceBrightnessError(bin: SbBin): number | null {
  return bin.count < MIN_PIXELS_FOR_ERROR ? null : finiteOrNull(bin.mu_err);
}

export function sbTotalFlux(profile: SbProfile): number | null {
  const last = profile.bins[profile.bins.length - 1];
  const total = finiteOrNull(last?.cumulative_sum);
  return total !== null && total > 0 ? total : null;
}

export function withUnit(label: string, unit: string | null | undefined): string {
  const text = typeof unit === "string" ? unit.trim() : "";
  return text === "" ? label : `${label} (${text})`;
}

export function sbSeries(profile: SbProfile, yMode: SbYMode, xUnit: SbXUnit, unit?: string | null): SbSeriesLayout {
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
          yErr: bins.map(surfaceBrightnessError),
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
    yLabel: withUnit(profile.background ? "mean - bg" : "mean", unit),
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

function positiveOrNull(v: number | null | undefined): number | null {
  const value = finiteOrNull(v);
  return value !== null && value > 0 ? value : null;
}

export function psfReferenceLine(
  psf: { fwhmArcsec: number | null; halfLightRadiusArcsec: number | null },
  pixelScaleArcsec: number | null | undefined,
  xUnit: SbXUnit,
): PlotReferenceLine | null {
  const halfLight = positiveOrNull(psf.halfLightRadiusArcsec);
  const fwhm = positiveOrNull(psf.fwhmArcsec);
  const radiusArcsec = halfLight ?? (fwhm === null ? null : fwhm / 2);
  if (radiusArcsec === null) return null;
  const scale = positiveOrNull(pixelScaleArcsec);
  const value = xUnit === "arcsec" ? radiusArcsec : scale === null ? null : radiusArcsec / scale;
  if (value === null) return null;
  return { axis: "x", value, label: halfLight === null ? "PSF FWHM/2" : "PSF R50", color: PSF_LINE_COLOR, dashed: true };
}

export function profileNotes(data: unknown): string[] {
  if (typeof data !== "object" || data === null) return [];
  const notes = (data as { notes?: unknown }).notes;
  return Array.isArray(notes) ? notes.filter((note): note is string => typeof note === "string") : [];
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

export function sbProfileCsvWithProvenance(profile: SbProfile, context: CsvContext): string {
  const pixelUnit = profile.photcal?.bunit ?? "image unit";
  const lines = [
    ...provenanceLines([
      ["software", `AstroBurst ${context.appVersion}`],
      ["exported_utc", context.exportedUtc],
      ["file", context.fileName],
      ["profile", "surface brightness in elliptical annuli"],
      ["centre_x_px", profile.x],
      ["centre_y_px", profile.y],
      ["sma_max_px", profile.sma_max],
      ["ellipticity", profile.ellipticity],
      ["angle_deg_image", profile.angle_deg],
      ["sky_pa_deg", profile.sky_pa_deg],
      ["bin_width_px", profile.bin_width],
      ["background_median", profile.background?.median ?? null],
      ["background_sigma", profile.background?.sigma ?? null],
      ["background_count", profile.background?.count ?? null],
      ["background", profile.background ? "subtracted from mean, median and the sums" : "none subtracted"],
      ["calibration", profile.photcal?.label ?? "none"],
      ["pixel_scale_arcsec", profile.pixel_scale_arcsec],
      ["pixel_area_arcsec2", profile.pixel_area_arcsec2],
      ["r50_px", profile.r50_px],
      ["r80_px", profile.r80_px],
      ["r90_px", profile.r90_px],
      ["petrosian_radius_px", profile.petrosian_radius_px],
      ["total_mag_ab", profile.total_mag_ab],
      ["dq_excluded_px", profile.dq_excluded],
      ["region_excluded_px", profile.region_excluded],
      ["positions", "0-based pixels, integer pixel centres"],
    ]),
    ...unitLines({
      sma_px: "pixel",
      sma_arcsec: "arcsec",
      sma_inner: "pixel",
      sma_outer: "pixel",
      count: "pixel",
      mean: pixelUnit,
      median: pixelUnit,
      std: pixelUnit,
      cumulative_sum: summedPixelUnit(profile.photcal?.bunit) ?? "image unit x pix",
      mu_ab: "mag/arcsec^2",
      mu_err: "mag",
      mag_ab_cumulative: "AB mag",
    }),
  ];
  return withProvenance(lines, sbProfileCsv(profile), CSV_LINE_END);
}

export function formatRadius(px: number | null | undefined, scale: number | null | undefined): string {
  const radius = finiteOrNull(px);
  if (radius === null) return NON_FINITE;
  const base = `${radius.toFixed(RADIUS_PX_DIGITS)} px`;
  const arcsecPerPx = finiteOrNull(scale);
  if (arcsecPerPx === null || arcsecPerPx <= 0) return base;
  return `${base} (${(radius * arcsecPerPx).toFixed(RADIUS_ARCSEC_DIGITS)}")`;
}

export function formatPositionAngles(imageDeg: number, skyDeg: number | null | undefined, ellipticity: number): string {
  if (!(ellipticity > 0) || !Number.isFinite(imageDeg)) return NON_FINITE;
  const image = `PA ${imageDeg.toFixed(PA_DIGITS)} deg image`;
  const sky = finiteOrNull(skyDeg);
  return sky === null ? image : `${image}, ${sky.toFixed(PA_DIGITS)} deg E of N`;
}

export function firstSurfaceBrightness(profile: SbProfile): number | null {
  return finiteOrNull(profile.bins[0]?.mu_ab);
}

export interface ProfileFetchState<R> {
  result: R | null;
  error: string | null;
}

export type ProfileFetchEvent<R> =
  | { type: "success"; result: R }
  | { type: "failure"; message: string }
  | { type: "reset" };

export function profileFetchReducer<R>(state: ProfileFetchState<R>, event: ProfileFetchEvent<R>): ProfileFetchState<R> {
  if (event.type === "success") return { result: event.result, error: null };
  if (event.type === "failure") return { result: null, error: event.message };
  return state.result === null && state.error === null ? state : { result: null, error: null };
}

export const MIN_BIN_WIDTH = 0.5;
export const MAX_BIN_WIDTH = 64;

export function parseBinWidth(text: string): number | null {
  const v = parseDecimalText(text);
  return v !== null && v >= MIN_BIN_WIDTH && v <= MAX_BIN_WIDTH ? v : null;
}
