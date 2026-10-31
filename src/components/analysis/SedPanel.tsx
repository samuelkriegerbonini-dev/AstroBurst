import { memo, useMemo } from "react";
import type { PhotometricPlane } from "../../shared/types/cube";
import { formatTickLabels, linearScale, niceTicks } from "../../utils/plotScale";
import { sedPoints, sedSeries, sedWavelengthAxis, sedX, type SedSource } from "../../utils/sedAxis";

interface SedPanelProps {
  planes: readonly PhotometricPlane[];
  bunit: string | null;
  source: SedSource;
}

const WIDTH = 320;
const HEIGHT = 180;
const PAD = { top: 14, right: 12, bottom: 32, left: 54 } as const;
const Y_TICK_COUNT = 4;
const Y_PAD_FRACTION = 0.1;
const POINT_RADIUS = 3;
const ERR_CAP = 3;
const LABEL_RISE = 6;
const POINT_COLOR = "#22d3ee";
const ERR_COLOR = "rgba(34,211,238,0.55)";
const GRID_COLOR = "#27272a";
const PIVOT_DECIMALS = 3;
const VALUE_DIGITS = 4;

function yDomain(values: number[], errors: (number | null)[]): [number, number] {
  const lows = values.map((v, i) => v - (errors[i] ?? 0));
  const highs = values.map((v, i) => v + (errors[i] ?? 0));
  const lo = Math.min(...lows);
  const hi = Math.max(...highs);
  const pad = hi > lo ? (hi - lo) * Y_PAD_FRACTION : Math.max(Math.abs(hi) * Y_PAD_FRACTION, 1);
  return [lo - pad, hi + pad];
}

function pointTitle(filter: string, pivotUm: number, value: number, err: number | null): string {
  const flux = err === null ? value.toPrecision(VALUE_DIGITS) : `${value.toPrecision(VALUE_DIGITS)} ± ${err.toPrecision(VALUE_DIGITS)}`;
  return `${filter} at ${pivotUm.toFixed(PIVOT_DECIMALS)} um: ${flux}`;
}

function SedPanel({ planes, bunit, source }: SedPanelProps) {
  const series = useMemo(() => sedSeries(source), [source]);
  const points = useMemo(() => (series ? sedPoints(planes, series.values, series.errors) : []), [planes, series]);
  const axis = useMemo(() => sedWavelengthAxis(points), [points]);
  const plotWidth = WIDTH - PAD.left - PAD.right;
  const plotHeight = HEIGHT - PAD.top - PAD.bottom;
  const y = useMemo(() => {
    if (points.length === 0) return null;
    const domain = yDomain(
      points.map((p) => p.value),
      points.map((p) => p.err),
    );
    const ticks = niceTicks(domain[0], domain[1], Y_TICK_COUNT).filter((t) => t >= domain[0] && t <= domain[1]);
    return { scale: linearScale(domain, [PAD.top + plotHeight, PAD.top]), ticks, labels: formatTickLabels(ticks) };
  }, [points, plotHeight]);
  const unit = series?.unit ?? bunit;
  const fluxLabel = unit ? `Flux (${unit})` : "Flux";

  return (
    <section data-sed-panel className="flex flex-col gap-1.5 px-1">
      <div className="flex items-center justify-between text-[10px]">
        <span className="font-semibold text-zinc-300 uppercase tracking-wider">SED</span>
        <span className="font-mono text-zinc-500 truncate">{series?.label ?? ""}</span>
      </div>
      {series === null && (
        <div className="text-[10px] text-zinc-500">
          Click a pixel or extract a region spectrum to plot the SED of the {planes.length} filter planes.
        </div>
      )}
      {series !== null && (axis === null || y === null) && (
        <div className="text-[10px] text-zinc-500">No plane has both a pivot wavelength and a finite value here.</div>
      )}
      {axis !== null && y !== null && (
        <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} width="100%" role="img" aria-label={`SED: ${fluxLabel} against pivot wavelength`}>
          {axis.ticks.map((tick) => {
            const x = PAD.left + sedX(tick, axis, plotWidth);
            return (
              <g key={`x${tick}`}>
                <line x1={x} x2={x} y1={PAD.top} y2={PAD.top + plotHeight} stroke={GRID_COLOR} strokeWidth={0.5} />
                <text x={x} y={PAD.top + plotHeight + 11} textAnchor="middle" fontSize={8} fill="currentColor" className="text-zinc-500">
                  {tick}
                </text>
              </g>
            );
          })}
          {y.ticks.map((tick, i) => (
            <g key={`y${tick}`}>
              <line x1={PAD.left} x2={PAD.left + plotWidth} y1={y.scale(tick)} y2={y.scale(tick)} stroke={GRID_COLOR} strokeWidth={0.5} />
              <text x={PAD.left - 4} y={y.scale(tick) + 3} textAnchor="end" fontSize={8} fill="currentColor" className="text-zinc-500">
                {y.labels[i]}
              </text>
            </g>
          ))}
          {points.map((p) => {
            const x = PAD.left + sedX(p.pivotUm, axis, plotWidth);
            const cy = y.scale(p.value);
            return (
              <g key={p.index} data-sed-point>
                {p.err !== null && (
                  <g data-sed-err stroke={ERR_COLOR} strokeWidth={1}>
                    <line x1={x} x2={x} y1={y.scale(p.value - p.err)} y2={y.scale(p.value + p.err)} />
                    <line x1={x - ERR_CAP} x2={x + ERR_CAP} y1={y.scale(p.value - p.err)} y2={y.scale(p.value - p.err)} />
                    <line x1={x - ERR_CAP} x2={x + ERR_CAP} y1={y.scale(p.value + p.err)} y2={y.scale(p.value + p.err)} />
                  </g>
                )}
                <circle cx={x} cy={cy} r={POINT_RADIUS} fill={POINT_COLOR}>
                  <title>{pointTitle(p.filter, p.pivotUm, p.value, p.err)}</title>
                </circle>
                <text x={x} y={cy - POINT_RADIUS - LABEL_RISE} textAnchor="middle" fontSize={7} fill="currentColor" className="text-zinc-400">
                  {p.filter}
                </text>
              </g>
            );
          })}
          <text x={PAD.left + plotWidth / 2} y={HEIGHT - 4} textAnchor="middle" fontSize={9} fill="currentColor" className="text-zinc-400">
            Pivot wavelength (um)
          </text>
          <text
            x={10}
            y={PAD.top + plotHeight / 2}
            textAnchor="middle"
            fontSize={9}
            fill="currentColor"
            className="text-zinc-400"
            transform={`rotate(-90 10 ${PAD.top + plotHeight / 2})`}
          >
            {fluxLabel}
          </text>
        </svg>
      )}
    </section>
  );
}

export default memo(SedPanel);
