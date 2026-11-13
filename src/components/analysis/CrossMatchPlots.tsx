import { memo, useMemo } from "react";
import type { CrossMatchResult } from "../../shared/types/catalog";
import { zeroPointSplitOf } from "../../utils/zeroPointPlot";
import ProfilePlot, { type PlotReferenceLine, type ProfileSeries } from "../regions/ProfilePlot";

interface CrossMatchPlotsProps {
  cross: CrossMatchResult;
}

const RESIDUAL_COLOR = "#22d3ee";
const MEDIAN_COLOR = "#a1a1aa";
const FITTED_COLOR = "#22d3ee";
const MODEL_COLOR = "#e4e4e7";
const OUTLIER_COLOR = "#fbbf24";
const SATURATED_COLOR = "#f87171";
const NO_COLOUR_COLOR = "#a1a1aa";
const PLOT_HEIGHT = 170;
const RESIDUALS_CSV = "residuals";
const ZERO_POINT_CSV = "zero_point";
const RMS_DIGITS = 3;
const TITLE_CLASS = "text-[10px] text-zinc-500";

function fmtArcsec(v: number): string {
  return Number.isFinite(v) ? `${v.toFixed(RMS_DIGITS)}"` : "--";
}

function colourCorrectedLabel(coeff: number): string {
  const sign = coeff < 0 ? "-" : "+";
  return `mag_inst ${sign} ${Math.abs(coeff).toFixed(RMS_DIGITS)}(BP-RP)`;
}

function CrossMatchPlots({ cross }: CrossMatchPlotsProps) {
  const residualSeries = useMemo<ProfileSeries[]>(() => {
    const rms = cross.astrometry ? `, rms ${fmtArcsec(cross.astrometry.rms_arcsec)}` : "";
    return [
      {
        x: cross.matches.map((m) => m.d_ra_arcsec),
        y: cross.matches.map((m) => m.d_dec_arcsec),
        color: RESIDUAL_COLOR,
        label: `star - Gaia${rms}`,
        mode: "points",
      },
    ];
  }, [cross]);

  const residualLines = useMemo<PlotReferenceLine[]>(() => {
    const a = cross.astrometry;
    if (!a) return [];
    return [
      { axis: "x", value: a.median_d_ra_arcsec, label: `median ${fmtArcsec(a.median_d_ra_arcsec)}`, color: MEDIAN_COLOR },
      { axis: "y", value: a.median_d_dec_arcsec, label: `median ${fmtArcsec(a.median_d_dec_arcsec)}`, color: MEDIAN_COLOR },
    ];
  }, [cross]);

  const zeroPoint = useMemo(() => {
    const zp = cross.zero_point;
    const coeff = zp ? zp.colour_coeff : null;
    const split = zeroPointSplitOf(cross);
    const series: ProfileSeries[] = [
      { x: split.fitted.x, y: split.fitted.y, color: FITTED_COLOR, label: "in fit", mode: "points" },
    ];
    if (split.line) {
      series.push({
        x: split.line.x,
        y: split.line.y,
        color: MODEL_COLOR,
        label: `mag - ZP (${zp ? zp.zp.toFixed(RMS_DIGITS) : "--"})`,
        mode: "line",
        dashed: true,
      });
    }
    if (split.outliers.x.length > 0) {
      series.push({ x: split.outliers.x, y: split.outliers.y, color: OUTLIER_COLOR, label: "rejected by clipping", mode: "points" });
    }
    if (split.saturated.x.length > 0) {
      series.push({ x: split.saturated.x, y: split.saturated.y, color: SATURATED_COLOR, label: "saturated (not fitted)", mode: "points" });
    }
    if (split.noColour.x.length > 0) {
      series.push({
        x: split.noColour.x,
        y: split.noColour.y,
        color: NO_COLOUR_COLOR,
        label: "no BP-RP (not in colour fit)",
        mode: "points",
      });
    }
    const yLabel = coeff !== null && Number.isFinite(coeff) ? colourCorrectedLabel(coeff) : "mag_inst";
    return { series, yLabel };
  }, [cross]);

  return (
    <div className="space-y-2">
      <div>
        <div className={TITLE_CLASS}>Residuals</div>
        <ProfilePlot
          series={residualSeries}
          xLabel="dRA (arcsec)"
          yLabel="dDec (arcsec)"
          height={PLOT_HEIGHT}
          referenceLines={residualLines}
          toolbar
          logToggle={false}
          csvName={RESIDUALS_CSV}
        />
      </div>
      <div>
        <div className={TITLE_CLASS}>Zero point</div>
        <ProfilePlot
          series={zeroPoint.series}
          xLabel={`Gaia ${cross.band} (Vega mag)`}
          yLabel={zeroPoint.yLabel}
          height={PLOT_HEIGHT}
          toolbar
          logToggle={false}
          csvName={ZERO_POINT_CSV}
        />
      </div>
    </div>
  );
}

export default memo(CrossMatchPlots);
