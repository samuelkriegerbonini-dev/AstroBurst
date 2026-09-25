import { memo, useMemo } from "react";
import type { CrossMatchResult, GaiaBand, MatchedCatalogRow } from "../../shared/types/catalog";
import { splitZeroPointPoints } from "../../utils/plotInteraction";
import ProfilePlot, { type PlotReferenceLine, type ProfileSeries } from "../regions/ProfilePlot";

interface CrossMatchPlotsProps {
  cross: CrossMatchResult;
}

const RESIDUAL_COLOR = "#22d3ee";
const MEDIAN_COLOR = "#a1a1aa";
const FITTED_COLOR = "#22d3ee";
const MODEL_COLOR = "#e4e4e7";
const OUTLIER_COLOR = "#fbbf24";
const PLOT_HEIGHT = 170;
const RESIDUALS_CSV = "residuals";
const ZERO_POINT_CSV = "zero_point";
const RMS_DIGITS = 3;
const TITLE_CLASS = "text-[10px] text-zinc-500";

function catalogueMag(row: MatchedCatalogRow, band: GaiaBand): number | null {
  if (band === "BP") return row.bp;
  if (band === "RP") return row.rp;
  return row.g;
}

function fmtArcsec(v: number): string {
  return Number.isFinite(v) ? `${v.toFixed(RMS_DIGITS)}"` : "--";
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
      { axis: "x", value: a.median_d_ra_arcsec, label: "median", color: MEDIAN_COLOR },
      { axis: "y", value: a.median_d_dec_arcsec, label: `rms ${fmtArcsec(a.rms_arcsec)}`, color: MEDIAN_COLOR },
    ];
  }, [cross]);

  const zeroPoint = useMemo(() => {
    const zp = cross.zero_point;
    const split = splitZeroPointPoints(
      cross.matches.map((m) => catalogueMag(m.row, cross.band)),
      cross.matches.map((m) => m.star.mag_inst),
      zp ? zp.zp : null,
      zp ? zp.rms : null,
    );
    const series: ProfileSeries[] = [
      { x: split.fitted.x, y: split.fitted.y, color: FITTED_COLOR, label: "matched stars", mode: "points" },
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
      series.push({ x: split.outliers.x, y: split.outliers.y, color: OUTLIER_COLOR, label: "outside fit", mode: "points" });
    }
    const yLabel = zp && zp.colour_coeff !== null ? "mag_inst (colour term not drawn)" : "mag_inst";
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
          csvName={RESIDUALS_CSV}
        />
      </div>
      <div>
        <div className={TITLE_CLASS}>Zero point</div>
        <ProfilePlot
          series={zeroPoint.series}
          xLabel={`Gaia ${cross.band} (mag)`}
          yLabel={zeroPoint.yLabel}
          height={PLOT_HEIGHT}
          toolbar
          csvName={ZERO_POINT_CSV}
        />
      </div>
    </div>
  );
}

export default memo(CrossMatchPlots);
