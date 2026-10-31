import { memo, useEffect, useRef } from "react";
import { WarningList } from "../ui";
import { paintKernel, profilePoints, starStatusLabel } from "../../utils/psfProfile";
import type { EmpiricalPsfReport } from "../../shared/types/processing";

const PLOT_WIDTH = 240;
const PLOT_HEIGHT = 90;

interface EmpiricalPsfResultProps {
  report: EmpiricalPsfReport;
}

function fixed(value: number | null, digits: number): string {
  return value === null ? "n/a" : value.toFixed(digits);
}

function EmpiricalPsfResult({ report }: EmpiricalPsfResultProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    if (canvasRef.current) paintKernel(canvasRef.current, report.kernel);
  }, [report]);

  const used = report.stars.filter((star) => star.used);
  const label = [report.dataset, report.filter].filter(Boolean).join(" ") || "no filter label";
  const fwhm = report.fwhm_px === null
    ? "n/a"
    : `${report.fwhm_px.toFixed(2)} px${report.fwhm_arcsec === null ? "" : ` (${report.fwhm_arcsec.toFixed(3)}")`}`;
  const size = report.oversampled_size === null
    ? `${report.psf_size} px`
    : `${report.psf_size} px, ${report.oversampled_size} px at x${report.oversampling}`;

  return (
    <div className="flex flex-col gap-3 animate-fade-in">
      <div className="flex items-center gap-3 ab-metric-card p-3">
        <canvas
          ref={canvasRef}
          className="rounded border border-zinc-700 flex-shrink-0"
          style={{ width: 100, height: 100, imageRendering: "pixelated" }}
        />
        <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
          <div className="text-zinc-500">FWHM</div>
          <div className="text-zinc-200 font-mono">{fwhm}</div>
          <div className="text-zinc-500">Stars</div>
          <div className="text-zinc-200 font-mono">{`${report.stars_used} / ${report.stars.length}`}</div>
          <div className="text-zinc-500">Size</div>
          <div className="text-zinc-200 font-mono">{size}</div>
          <div className="text-zinc-500">Aperture</div>
          <div className="text-zinc-200 font-mono">{`${report.normalization_radius_px.toFixed(1)} px`}</div>
          <div className="text-zinc-500">Label</div>
          <div className="text-zinc-200 font-mono">{label}</div>
        </div>
      </div>

      <figure className="flex flex-col gap-1">
        <svg
          viewBox={`-4 -4 ${PLOT_WIDTH + 8} ${PLOT_HEIGHT + 8}`}
          className="w-full rounded border border-zinc-800 bg-zinc-950/40"
          role="img"
          aria-label="Radial profiles of the used stars and of the stacked PSF, each scaled to its peak"
        >
          {used.map((star) => (
            <polyline
              key={star.name}
              data-star={star.name}
              points={profilePoints(star.profile, PLOT_WIDTH, PLOT_HEIGHT)}
              fill="none"
              stroke="currentColor"
              strokeWidth={1}
              className="text-violet-400/50"
            >
              <title>{star.name}</title>
            </polyline>
          ))}
          <polyline
            data-star="stack"
            points={profilePoints(report.profile, PLOT_WIDTH, PLOT_HEIGHT)}
            fill="none"
            stroke="currentColor"
            strokeWidth={1.8}
            className="text-amber-300"
          >
            <title>stacked PSF</title>
          </polyline>
        </svg>
        <figcaption className="text-[10px] text-zinc-500 px-1">Radial profile per used star (violet) and of the stacked PSF (amber), radius 0 at the left</figcaption>
      </figure>

      <table className="w-full text-[10px] font-mono text-zinc-400">
        <thead>
          <tr className="text-zinc-500">
            <th className="text-left font-normal px-1">Star</th>
            <th className="text-right font-normal px-1">Centre (0-based)</th>
            <th className="text-left font-normal px-1">Status</th>
          </tr>
        </thead>
        <tbody>
          {report.stars.map((star, index) => (
            <tr key={`${index}-${star.name}`} className={star.used ? "" : "text-amber-300/80"}>
              <td className="px-1">{star.name}</td>
              <td className="px-1 text-right">{`${fixed(star.centre_x ?? star.x, 2)}, ${fixed(star.centre_y ?? star.y, 2)}`}</td>
              <td className="px-1">{starStatusLabel(star)}</td>
            </tr>
          ))}
        </tbody>
      </table>

      {(report.output_path || report.oversampled_path) && (
        <div className="flex flex-col gap-0.5 text-[10px] text-zinc-500 font-mono px-1 break-all">
          {report.output_path && <span>{report.output_path}</span>}
          {report.oversampled_path && <span>{report.oversampled_path}</span>}
        </div>
      )}

      <WarningList warnings={report.warnings} />
    </div>
  );
}

export default memo(EmpiricalPsfResult);
