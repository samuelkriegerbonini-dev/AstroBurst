import { memo, useCallback, useId, useMemo, useState } from "react";
import { useFileContext } from "../../context/PreviewContext";
import type { PhotometricPlane } from "../../shared/types/cube";
import { APP_VERSION } from "../../utils/constants";
import { formatWithUncertainty, MISSING_VALUE } from "../../utils/formatUncertainty";
import { AB_MAG_ZERO_POINT } from "../../utils/fluxUnits";
import { formatLogTickLabel, formatTickLabels, linearScale, niceTicks } from "../../utils/plotScale";
import {
  psfFwhmSummary,
  sedAbView,
  sedCsv,
  sedCsvFileName,
  sedLogAllowed,
  sedLogYAxis,
  sedPoints,
  sedSeries,
  sedWavelengthAxis,
  sedX,
  sedYDomain,
  sedYLabel,
  type SedPoint,
  type SedSource,
  type SedYScale,
} from "../../utils/sedAxis";
import { PROVENANCE_FILE_SEPARATOR, summedPixelUnit } from "../../utils/csvProvenance";
import { saveCsvDialog } from "../../utils/spectrumExport";
import { measurementLog } from "../../utils/measurementLog";
import { sedEntry } from "../../utils/measurementLogForced";
import { fileBaseName } from "../../utils/values";
import { SMALL_BUTTON_CLASS } from "../ui";

interface SedCubeProps {
  planes: readonly PhotometricPlane[];
  bunit: string | null;
  source: SedSource;
}

interface SedPointsProps {
  points: readonly SedPoint[];
  unit: string;
  label: string;
  fileNames?: readonly string[];
}

type SedPanelProps = SedCubeProps | SedPointsProps;

interface SedViewProps {
  label: string;
  yLabel: string;
  points: readonly SedPoint[];
  emptyText: string;
  psfSummary: string | null;
  psfFwhms: readonly (number | null)[] | null;
  csvUnit: string | null;
  csvFile: string | null;
  csvFileName: string | null;
  logImage: string;
}

const WIDTH = 320;
const HEIGHT = 180;
const PAD = { top: 14, right: 12, bottom: 32, left: 54 } as const;
const Y_TICK_COUNT = 4;
const POINT_RADIUS = 3;
const ERR_CAP = 3;
const LIMIT_ARROW_LENGTH = 10;
const LIMIT_ARROW_HEAD = 3;
const LABEL_RISE = 6;
const POINT_COLOR = "#22d3ee";
const ERR_COLOR = "rgba(34,211,238,0.55)";
const GRID_COLOR = "#27272a";
const PIVOT_DECIMALS = 3;
const PSF_DECIMALS = 3;
const SCALE_BUTTON_CLASS = "text-[9px] px-1.5 py-0.5 rounded border transition-colors disabled:opacity-40 disabled:cursor-not-allowed";
const SCALE_ON_CLASS = "border-cyan-500/60 text-cyan-300 bg-cyan-500/10";
const SCALE_OFF_CLASS = "border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80";
const LOG_Y_TITLE = "Plot the values on a log scale";
const LOG_Y_REFUSED = "A log scale needs every plotted value minus its error above 0";
const AB_TITLE = `Plot AB magnitudes, m = ${AB_MAG_ZERO_POINT.toFixed(2)} - 2.5 log10(flux / 1 Jy); brighter is up`;
const AB_REFUSED = "No point has a flux above zero: there is no AB magnitude to plot";
const AB_Y_LABEL = "AB magnitude";

function scaleButtonClass(pressed: boolean): string {
  return `${SCALE_BUTTON_CLASS} ${pressed ? SCALE_ON_CLASS : SCALE_OFF_CLASS}`;
}

function valueText(point: SedPoint): string {
  return point.limit ? `< ${formatWithUncertainty(point.value, null)}` : formatWithUncertainty(point.value, point.err);
}

function pointTitle(point: SedPoint): string {
  const suffix = point.limit ? " (upper limit)" : "";
  return `${point.filter} at ${point.pivotUm.toFixed(PIVOT_DECIMALS)} um: ${valueText(point)}${suffix}`;
}

function abText(point: SedPoint | undefined): string {
  if (point === undefined) return MISSING_VALUE;
  return point.limit ? `> ${formatWithUncertainty(point.value, null)}` : formatWithUncertainty(point.value, point.err);
}

function abPointTitle(point: SedPoint): string {
  const suffix = point.limit ? " (upper limit on the flux)" : "";
  return `${point.filter} at ${point.pivotUm.toFixed(PIVOT_DECIMALS)} um: ${abText(point)} AB mag${suffix}`;
}

function abNote(dropped: number): string {
  return `Not plotted in AB magnitudes: ${dropped} ${dropped === 1 ? "point" : "points"} with a flux of zero or below.`;
}

function psfText(fwhm: number | null | undefined): string {
  return typeof fwhm === "number" && Number.isFinite(fwhm) && fwhm > 0 ? fwhm.toFixed(PSF_DECIMALS) : MISSING_VALUE;
}

function exportFailure(error: unknown): string {
  return `SED export failed: ${error instanceof Error ? error.message : String(error)}`;
}

function SedView({ label, yLabel, points, emptyText, psfSummary, psfFwhms, csvUnit, csvFile, csvFileName, logImage }: SedViewProps) {
  const tableId = useId();
  const [exportError, setExportError] = useState<string | null>(null);
  const [scale, setScale] = useState<SedYScale>("linear");
  const ab = useMemo(() => sedAbView(points, csvUnit), [points, csvUnit]);
  const logOk = sedLogAllowed(points);
  const abShown = scale === "ab" && ab !== null && ab.points.length > 0 ? ab : null;
  const shown: SedYScale = scale === "log" && logOk ? "log" : abShown !== null ? "ab" : "linear";
  const plotted = abShown?.points ?? points;
  const abByIndex = useMemo(() => new Map((abShown?.points ?? []).map((p) => [p.index, p])), [abShown]);
  const shownYLabel = shown === "ab" ? AB_Y_LABEL : yLabel;
  const titleOf = shown === "ab" ? abPointTitle : pointTitle;
  const axis = useMemo(() => sedWavelengthAxis(points), [points]);
  const plotWidth = WIDTH - PAD.left - PAD.right;
  const plotHeight = HEIGHT - PAD.top - PAD.bottom;
  const y = useMemo(() => {
    if (shown === "log") {
      const axisY = sedLogYAxis(plotted);
      if (axisY === null) return null;
      const toPixel = linearScale([Math.log10(axisY.min), Math.log10(axisY.max)], [PAD.top + plotHeight, PAD.top]);
      return { scale: (v: number) => toPixel(Math.log10(v)), ticks: axisY.ticks, labels: axisY.ticks.map(formatLogTickLabel) };
    }
    const domain = sedYDomain(plotted);
    if (domain === null) return null;
    const ticks = niceTicks(domain[0], domain[1], Y_TICK_COUNT).filter((t) => t >= domain[0] && t <= domain[1]);
    const range: [number, number] = shown === "ab" ? [PAD.top, PAD.top + plotHeight] : [PAD.top + plotHeight, PAD.top];
    return { scale: linearScale(domain, range), ticks, labels: formatTickLabels(ticks) };
  }, [shown, plotted, plotHeight]);

  const csvText = useCallback(
    () => sedCsv(points, { unit: csvUnit, source: label, file: csvFile, exportedUtc: new Date().toISOString(), appVersion: APP_VERSION }),
    [points, csvUnit, label, csvFile],
  );

  const copyCsv = useCallback(async () => {
    setExportError(null);
    try {
      const text = csvText();
      await navigator.clipboard.writeText(text);
      measurementLog.append(sedEntry({ file: csvFile, image: logImage, label, unit: csvUnit, points, savedPath: null }));
    } catch (e: unknown) {
      setExportError(exportFailure(e));
    }
  }, [csvText, csvFile, logImage, label, csvUnit, points]);

  const saveCsv = useCallback(async () => {
    setExportError(null);
    try {
      const text = csvText();
      const savedPath = await saveCsvDialog(text, sedCsvFileName(csvFileName), "Save SED");
      if (savedPath !== null) measurementLog.append(sedEntry({ file: csvFile, image: logImage, label, unit: csvUnit, points, savedPath }));
    } catch (e: unknown) {
      setExportError(exportFailure(e));
    }
  }, [csvText, csvFileName, csvFile, logImage, label, csvUnit, points]);

  return (
    <section data-sed-panel className="flex flex-col gap-1.5 px-1">
      <div className="flex items-center justify-between gap-2 text-[10px]">
        <span className="font-semibold text-zinc-300 uppercase tracking-wider">SED</span>
        <span className="font-mono text-zinc-500 truncate">{label}</span>
        {points.length > 0 && (
          <span className="flex items-center gap-1 shrink-0">
            <button type="button" onClick={() => void copyCsv()} className={SMALL_BUTTON_CLASS}>
              Copy CSV
            </button>
            <button type="button" onClick={() => void saveCsv()} className={SMALL_BUTTON_CLASS}>
              Save CSV
            </button>
            <button
              type="button"
              data-action="sed-log-y"
              aria-pressed={shown === "log"}
              disabled={!logOk}
              title={logOk ? LOG_Y_TITLE : LOG_Y_REFUSED}
              onClick={() => setScale(shown === "log" ? "linear" : "log")}
              className={scaleButtonClass(shown === "log")}
            >
              Log y
            </button>
            {ab !== null && (
              <button
                type="button"
                data-action="sed-ab"
                aria-pressed={shown === "ab"}
                disabled={ab.points.length === 0}
                title={ab.points.length > 0 ? AB_TITLE : AB_REFUSED}
                onClick={() => setScale(shown === "ab" ? "linear" : "ab")}
                className={scaleButtonClass(shown === "ab")}
              >
                AB mag
              </button>
            )}
          </span>
        )}
      </div>
      {psfSummary !== null && (
        <div data-sed-psf className="text-[10px] text-zinc-400">
          {psfSummary}
        </div>
      )}
      {(axis === null || y === null) && <div className="text-[10px] text-zinc-500">{emptyText}</div>}
      {axis !== null && y !== null && (
        <svg
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          width="100%"
          role="img"
          aria-label={`SED: ${shownYLabel}${shown === "log" ? " on a log scale" : ""} against pivot wavelength`}
          aria-describedby={tableId}
        >
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
              <text data-sed-ytick x={PAD.left - 4} y={y.scale(tick) + 3} textAnchor="end" fontSize={8} fill="currentColor" className="text-zinc-500">
                {y.labels[i]}
              </text>
            </g>
          ))}
          {plotted.map((p) => {
            const x = PAD.left + sedX(p.pivotUm, axis, plotWidth);
            const cy = y.scale(p.value);
            const tip = cy + LIMIT_ARROW_LENGTH;
            return p.limit ? (
              <g key={p.index} data-sed-point data-sed-limit>
                <g stroke={POINT_COLOR} strokeWidth={1.5}>
                  <title>{titleOf(p)}</title>
                  <line x1={x} x2={x} y1={cy} y2={tip} />
                  <line x1={x - LIMIT_ARROW_HEAD} x2={x} y1={tip - LIMIT_ARROW_HEAD} y2={tip} />
                  <line x1={x + LIMIT_ARROW_HEAD} x2={x} y1={tip - LIMIT_ARROW_HEAD} y2={tip} />
                </g>
                <text x={x} y={cy - LABEL_RISE} textAnchor="middle" fontSize={7} fill="currentColor" className="text-zinc-400">
                  {p.filter}
                </text>
              </g>
            ) : (
              <g key={p.index} data-sed-point>
                {p.err !== null && (
                  <g data-sed-err stroke={ERR_COLOR} strokeWidth={1}>
                    <line x1={x} x2={x} y1={y.scale(p.value - p.err)} y2={y.scale(p.value + p.err)} />
                    <line x1={x - ERR_CAP} x2={x + ERR_CAP} y1={y.scale(p.value - p.err)} y2={y.scale(p.value - p.err)} />
                    <line x1={x - ERR_CAP} x2={x + ERR_CAP} y1={y.scale(p.value + p.err)} y2={y.scale(p.value + p.err)} />
                  </g>
                )}
                <circle cx={x} cy={cy} r={POINT_RADIUS} fill={POINT_COLOR}>
                  <title>{titleOf(p)}</title>
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
            {shownYLabel}
          </text>
        </svg>
      )}
      {abShown !== null && abShown.dropped > 0 && (
        <div data-sed-ab-note className="text-[10px] text-zinc-400">
          {abNote(abShown.dropped)}
        </div>
      )}
      {axis !== null && y !== null && (
        <table id={tableId} data-sed-table className="w-full text-[10px] font-mono text-zinc-300">
          <caption className="sr-only">SED values</caption>
          <thead className="text-zinc-500">
            <tr>
              <th scope="col" className="text-left font-normal">
                Filter
              </th>
              <th scope="col" className="text-right font-normal">
                Pivot (um)
              </th>
              <th scope="col" className="text-right font-normal">
                {yLabel}
              </th>
              {abShown !== null && (
                <th scope="col" className="text-right font-normal">
                  AB mag
                </th>
              )}
              {psfFwhms !== null && (
                <th scope="col" className="text-right font-normal">
                  PSF FWHM (arcsec)
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {points.map((p) => (
              <tr key={p.index} data-sed-row>
                <th scope="row" className="text-left font-normal">
                  {p.filter}
                </th>
                <td className="text-right">{p.pivotUm.toFixed(PIVOT_DECIMALS)}</td>
                <td className="text-right">{valueText(p)}</td>
                {abShown !== null && <td className="text-right">{abText(abByIndex.get(p.index))}</td>}
                {psfFwhms !== null && <td className="text-right">{psfText(psfFwhms[p.index])}</td>}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {exportError !== null && (
        <div role="alert" className="text-[10px] text-red-400">
          {exportError}
        </div>
      )}
    </section>
  );
}

function SedCubePanel({ planes, bunit, source }: SedCubeProps) {
  const { file } = useFileContext();
  const series = useMemo(() => sedSeries(source), [source]);
  const points = useMemo(() => (series ? sedPoints(planes, series.values, series.errors) : []), [planes, series]);
  const psfSummary = useMemo(() => psfFwhmSummary(planes), [planes]);
  const psfFwhms = useMemo(() => (psfSummary === null ? null : planes.map((plane) => plane.psf_fwhm_arcsec)), [planes, psfSummary]);
  const fileName = file ? file.name || fileBaseName(file.path) : null;
  const csvUnit = series?.unit ?? (series?.kind === "region_sum" ? summedPixelUnit(bunit) : bunit);
  return (
    <SedView
      label={series?.label ?? ""}
      yLabel={sedYLabel(series, bunit)}
      points={points}
      emptyText={
        series === null
          ? `Click a pixel or extract a region spectrum to plot the SED of the ${planes.length} filter planes.`
          : "No plane has both a pivot wavelength and a finite value here."
      }
      psfSummary={psfSummary}
      psfFwhms={psfFwhms}
      csvUnit={csvUnit}
      csvFile={fileName}
      csvFileName={fileName}
      logImage="loaded cube"
    />
  );
}

function SedPointsPanel({ points, unit, label, fileNames }: SedPointsProps) {
  const plotted = useMemo(
    () => points.filter((p) => Number.isFinite(p.pivotUm) && p.pivotUm > 0 && Number.isFinite(p.value)),
    [points],
  );
  return (
    <SedView
      label={label}
      yLabel={`Flux (${unit})`}
      points={plotted}
      emptyText="No band of this source has both a pivot wavelength and a flux."
      psfSummary={null}
      psfFwhms={null}
      csvUnit={unit}
      csvFile={fileNames && fileNames.length > 0 ? fileNames.join(PROVENANCE_FILE_SEPARATOR) : null}
      csvFileName={null}
      logImage="original"
    />
  );
}

function SedPanel(props: SedPanelProps) {
  return "points" in props ? <SedPointsPanel {...props} /> : <SedCubePanel {...props} />;
}

export default memo(SedPanel);
