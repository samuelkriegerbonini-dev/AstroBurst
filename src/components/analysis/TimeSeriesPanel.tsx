import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ClipboardCopy, Clock, Download, Loader2, X } from "lucide-react";
import ProfilePlot, { type PlotHit, type ProfileSeries } from "../regions/ProfilePlot";
import { timeSeriesPhotometry } from "../../services/analysis";
import { cancelProgress } from "../../services/progress";
import {
  TIME_SERIES_PROGRESS_EVENT,
  type TimeSeriesResult,
  type TimeSeriesRole,
  type TimeSeriesTarget,
} from "../../shared/types/analysis";
import { useDqContext, useFileContext } from "../../context/PreviewContext";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useProgress } from "../../hooks/useProgress";
import { orderWithReferenceFirst, pathOf, useMatchingFrames } from "../../hooks/useFrameSet";
import { fileStore } from "../../hooks/useFileStore";
import {
  centroidJumps,
  checkStarCurve,
  frameTimes,
  hasCompleteTimeAxis,
  jdZero,
  lightCurve,
  lightCurveCsv,
  lightCurveCsvFileName,
  lightCurveExtras,
  seriesRms,
  type LightCurvePoint,
  type TimeAxis,
} from "../../utils/differentialPhotometry";
import { ErrorAlert, RunButton, Toggle, WarningList } from "../ui";

interface TimeSeriesPanelProps {
  filePath: string | null;
}

interface SeriesOption {
  key: string;
  label: string;
}

const DEFAULT_APERTURE_RADIUS_PX = "5";
const TABLE_ROW_LIMIT = 300;
const SAVED_NOTICE_MS = 6000;
const PLOT_HEIGHT_PX = 170;
const JD_TICK_DIGITS = 3;
const ROLE_OPTIONS: TimeSeriesRole[] = ["target", "comp", "check", "ignore"];
const SERIES_TARGET = "target";
const SERIES_RAW = "raw";
const TARGET_COLOR = "#fbbf24";
const CHECK_COLOR = "#67e8f9";
const RAW_COLORS = ["#fbbf24", "#67e8f9", "#a5b4fc", "#86efac", "#f9a8d4", "#fdba74"];
const ANNULUS_NEEDS_BOTH = "sky annulus needs both an inner and an outer radius";
const STARS_HINT =
  "Draw Point regions on the target and comparison stars, or use Add as Point regions in the photometry table.";
const FRAMES_HINT = "Load the other frames of the sequence; only done files with the same dimensions are measured.";

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";
const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const CHIP_CLASS = "inline-block rounded px-1 py-px text-[8px] leading-tight mr-0.5";

function parseOptionalNumber(text: string): number | undefined {
  const v = parseFloat(text);
  return Number.isFinite(v) && v > 0 ? v : undefined;
}

function fmt(v: number | null | undefined, digits: number): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "--";
  return Math.abs(v) >= 1e6 ? v.toExponential(2) : v.toFixed(digits);
}

function defaultRole(index: number): TimeSeriesRole {
  return index === 0 ? "target" : "comp";
}

function instrumentalMag(flux: number | null | undefined): number | null {
  return typeof flux === "number" && Number.isFinite(flux) && flux > 0 ? -2.5 * Math.log10(flux) : null;
}

function TimeSeriesPanel({ filePath }: TimeSeriesPanelProps) {
  const apertureId = useId();
  const skyInId = useId();
  const skyOutId = useId();
  const gainId = useId();
  const seriesId = useId();
  const axisId = useId();
  const { file } = useFileContext();
  const { excludeDq } = useDqContext();
  const regionDoc = useRegionDoc(filePath);
  const progress = useProgress(TIME_SERIES_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [roles, setRoles] = useState<Record<string, TimeSeriesRole>>({});
  const [apertureText, setApertureText] = useState(DEFAULT_APERTURE_RADIUS_PX);
  const [skyInText, setSkyInText] = useState("");
  const [skyOutText, setSkyOutText] = useState("");
  const [gainText, setGainText] = useState("");
  const [trackDrift, setTrackDrift] = useState(true);
  const [seriesKey, setSeriesKey] = useState(SERIES_TARGET);
  const [timeAxis, setTimeAxis] = useState<TimeAxis>("jd");
  const [result, setResult] = useState<TimeSeriesResult | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const requestSeqRef = useRef(0);

  useEffect(() => {
    requestSeqRef.current++;
    setResult(null);
    setError(null);
    setRunning(false);
    setSavedPath(null);
    setRoles({});
    setSeriesKey(SERIES_TARGET);
  }, [filePath]);

  const dims = file?.result?.dimensions ?? null;
  const matching = useMatchingFrames(dims);
  const frames = useMemo(() => orderWithReferenceFirst(matching, filePath), [matching, filePath]);
  const referenceLoaded = frames.length > 0 && pathOf(frames[0]) === filePath;

  const pointRegions = useMemo(() => regionDoc.regions.filter((r) => r.shape.shape === "point"), [regionDoc]);
  const targets = useMemo((): { id: string; target: TimeSeriesTarget }[] => {
    const rows: { id: string; target: TimeSeriesTarget }[] = [];
    for (const region of pointRegions) {
      if (region.shape.shape !== "point") continue;
      const index = rows.length;
      rows.push({
        id: region.id,
        target: {
          x: region.shape.x,
          y: region.shape.y,
          label: region.props.text || `P${index + 1}`,
          role: roles[region.id] ?? defaultRole(index),
        },
      });
    }
    return rows;
  }, [pointRegions, roles]);

  const apertureRadius = parseOptionalNumber(apertureText);
  const annulusInner = parseOptionalNumber(skyInText);
  const annulusOuter = parseOptionalNumber(skyOutText);
  const gain = parseOptionalNumber(gainText);
  const annulusHalfFilled = (annulusInner === undefined) !== (annulusOuter === undefined);
  const hasTarget = targets.some((t) => t.target.role === "target");
  const canMeasure =
    !!filePath && referenceLoaded && hasTarget && apertureRadius !== undefined && !annulusHalfFilled && !running;

  const measure = useCallback(async () => {
    if (!filePath || !referenceLoaded || apertureRadius === undefined || targets.length === 0) return;
    const seq = ++requestSeqRef.current;
    setRunning(true);
    setError(null);
    setSavedPath(null);
    resetProgress();
    try {
      const res = await timeSeriesPhotometry(
        frames.map(pathOf),
        targets.map((t) => t.target),
        { apertureRadius, annulusInner, annulusOuter, gain, excludeDq, trackDrift },
      );
      if (requestSeqRef.current !== seq) return;
      setResult(res);
      setSeriesKey(SERIES_TARGET);
    } catch (e: unknown) {
      if (requestSeqRef.current === seq) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (requestSeqRef.current === seq) {
        setRunning(false);
        resetProgress();
      }
    }
  }, [filePath, referenceLoaded, apertureRadius, targets, frames, annulusInner, annulusOuter, gain, excludeDq, trackDrift, resetProgress]);

  const cancel = useCallback(() => {
    cancelProgress(TIME_SERIES_PROGRESS_EVENT).catch(() => {});
  }, []);

  const setRole = useCallback((id: string, role: TimeSeriesRole) => {
    setRoles((prev) => ({ ...prev, [id]: role }));
  }, []);

  const analysis = useMemo(() => {
    if (!result) return null;
    const echo = result.targets;
    const targetIdx = echo.findIndex((t) => t.role === "target");
    const compIdx = echo.map((t, i) => (t.role === "comp" ? i : -1)).filter((i) => i >= 0);
    const checkIdx = echo.map((t, i) => (t.role === "check" ? i : -1)).filter((i) => i >= 0);
    const axis: TimeAxis = timeAxis === "jd" && hasCompleteTimeAxis(result) ? "jd" : "index";
    const targetCurve = targetIdx >= 0 ? lightCurve(result, targetIdx, compIdx, axis) : [];
    const checkCurves = checkIdx.map((i) => lightCurve(result, i, compIdx, axis));
    const compCurves = compIdx.length >= 2 ? compIdx.map((i) => checkStarCurve(result, i, compIdx, axis)) : [];
    const options: SeriesOption[] = [];
    if (targetIdx >= 0) options.push({ key: SERIES_TARGET, label: `target ${echo[targetIdx].label}` });
    checkIdx.forEach((i) => options.push({ key: `check:${i}`, label: `check star ${echo[i].label}` }));
    if (compCurves.length > 0) compIdx.forEach((i) => options.push({ key: `comp:${i}`, label: `comp ${echo[i].label} vs others` }));
    if (compIdx.length > 0) options.push({ key: SERIES_RAW, label: "raw comps" });
    const checkReference = compCurves[0] ?? checkCurves[0] ?? null;
    const timeline = frameTimes(result, axis);
    const jd0 = axis === "jd" ? jdZero(timeline) : 0;
    return {
      targetIdx,
      compIdx,
      checkIdx,
      axis,
      targetCurve,
      checkCurves,
      compCurves,
      options,
      jd0,
      timeline,
      targetRms: seriesRms(targetCurve),
      checkRms: checkReference ? seriesRms(checkReference) : null,
      used: targetCurve.filter((p) => p.mag !== null).length,
      jumps: new Set(centroidJumps(result)),
    };
  }, [result, timeAxis]);

  const activeKey = analysis && analysis.options.some((o) => o.key === seriesKey) ? seriesKey : (analysis?.options[0]?.key ?? SERIES_TARGET);

  const series = useMemo((): ProfileSeries[] => {
    if (!result || !analysis) return [];
    const { axis, jd0 } = analysis;
    const xOf = (p: { t: number }) => (axis === "jd" ? p.t - jd0 : p.t);
    const fromCurve = (points: LightCurvePoint[], label: string, color: string): ProfileSeries => ({
      x: points.map(xOf),
      y: points.map((p) => p.mag),
      yErr: points.map((p) => p.err),
      color,
      label,
      mode: "points",
    });
    if (activeKey === SERIES_RAW) {
      return analysis.compIdx.map((idx, k) => ({
        x: analysis.timeline.map(xOf),
        y: result.frames.map((f) => instrumentalMag(f.targets[idx]?.net_flux)),
        color: RAW_COLORS[k % RAW_COLORS.length],
        label: result.targets[idx].label,
        mode: "points",
      }));
    }
    if (activeKey.startsWith("check:")) {
      const idx = Number(activeKey.slice("check:".length));
      const k = analysis.checkIdx.indexOf(idx);
      return k >= 0 ? [fromCurve(analysis.checkCurves[k], result.targets[idx].label, CHECK_COLOR)] : [];
    }
    if (activeKey.startsWith("comp:")) {
      const idx = Number(activeKey.slice("comp:".length));
      const k = analysis.compIdx.indexOf(idx);
      return k >= 0 && analysis.compCurves[k] ? [fromCurve(analysis.compCurves[k], result.targets[idx].label, CHECK_COLOR)] : [];
    }
    return analysis.targetIdx >= 0 ? [fromCurve(analysis.targetCurve, result.targets[analysis.targetIdx].label, TARGET_COLOR)] : [];
  }, [result, analysis, activeKey]);

  const currentFrameIndex = useMemo(() => (result ? result.frames.findIndex((f) => f.path === filePath) : -1), [result, filePath]);
  const selected = useMemo(() => (currentFrameIndex >= 0 ? { seriesIndex: 0, index: currentFrameIndex } : null), [currentFrameIndex]);

  const showFrame = useCallback(
    (hit: PlotHit) => {
      if (!result) return;
      const frame = result.frames[hit.index];
      if (!frame) return;
      const target = frames.find((f) => pathOf(f) === frame.path);
      if (target) fileStore.selectFile(target.id);
    },
    [result, frames],
  );

  const csvText = useCallback((): string => {
    if (!result || !analysis || analysis.targetIdx < 0) return "";
    return lightCurveCsv(result, analysis.targetCurve, lightCurveExtras(result, analysis.targetIdx, analysis.compIdx));
  }, [result, analysis]);

  const copyCsv = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(csvText());
    } catch (e: unknown) {
      setError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [csvText]);

  const saveCsv = useCallback(async () => {
    if (!result) return;
    setError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: lightCurveCsvFileName(result.reference_path),
        filters: [{ name: "CSV", extensions: ["csv"] }],
        title: "Save light curve",
      });
      if (!target) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, csvText());
      setSavedPath(target);
      setTimeout(() => setSavedPath(null), SAVED_NOTICE_MS);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [result, csvText]);

  const xLabel = analysis?.axis === "jd" ? `JD - ${analysis.jd0}` : "frame index";
  const yLabel = activeKey === SERIES_RAW ? "inst mag" : "diff mag";
  const xTickFormat = useMemo(() => (analysis?.axis === "jd" ? (v: number) => v.toFixed(JD_TICK_DIGITS) : undefined), [analysis?.axis]);
  const shownFrames = result ? result.frames.slice(0, TABLE_ROW_LIMIT) : [];
  const targetLabel = analysis && analysis.targetIdx >= 0 ? result?.targets[analysis.targetIdx].label : null;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Clock size={12} className="text-amber-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Time series</span>
        </div>
        {running && <Loader2 size={12} className="animate-spin text-amber-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className={SECTION_CLASS}>Stars</div>
        {targets.length === 0 ? (
          <div className="text-[10px] text-zinc-600">{STARS_HINT}</div>
        ) : (
          <div className="space-y-0.5">
            {targets.map(({ id, target }) => (
              <div key={id} className="grid grid-cols-[1fr_auto_auto] items-center gap-1.5 text-[10px] font-mono">
                <span className="text-zinc-300 truncate" title={target.label}>
                  {target.label}
                </span>
                <span className="text-zinc-500">
                  {fmt(target.x, 1)}, {fmt(target.y, 1)}
                </span>
                <select
                  value={target.role}
                  aria-label={`Role of ${target.label}`}
                  onChange={(e) => setRole(id, e.target.value as TimeSeriesRole)}
                  className="bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono"
                >
                  {ROLE_OPTIONS.map((role) => (
                    <option key={role} value={role}>
                      {role}
                    </option>
                  ))}
                </select>
              </div>
            ))}
            {!hasTarget && <div className="text-[9px] text-amber-400/90">Mark one star as the target.</div>}
          </div>
        )}

        <div className={SECTION_CLASS}>Frames</div>
        <div className="text-[10px] text-zinc-400 font-mono">
          {frames.length} matching {frames.length === 1 ? "frame" : "frames"}
          {dims ? ` at ${dims[0]} x ${dims[1]}` : ""}
        </div>
        {frames.length < 2 && <div className="text-[10px] text-zinc-600">{FRAMES_HINT}</div>}
        {!referenceLoaded && !!filePath && (
          <div className="text-[9px] text-amber-400/90">The current file is not a done frame, so it cannot be the reference.</div>
        )}

        <div className="grid grid-cols-4 gap-1.5">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={apertureId} className="text-[9px] text-zinc-500">
              r_ap px
            </label>
            <input
              id={apertureId}
              type="number"
              min={2}
              max={60}
              step={0.5}
              value={apertureText}
              onChange={(e) => setApertureText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyInId} className="text-[9px] text-zinc-500">
              sky in px
            </label>
            <input
              id={skyInId}
              type="number"
              min={1}
              step={0.5}
              value={skyInText}
              placeholder="2 x r_ap"
              onChange={(e) => setSkyInText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyOutId} className="text-[9px] text-zinc-500">
              sky out px
            </label>
            <input
              id={skyOutId}
              type="number"
              min={1}
              step={0.5}
              value={skyOutText}
              placeholder="3 x r_ap"
              onChange={(e) => setSkyOutText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={gainId} className="text-[9px] text-zinc-500">
              gain e-/ADU
            </label>
            <input
              id={gainId}
              type="number"
              min={0.01}
              step={0.1}
              value={gainText}
              placeholder="none"
              onChange={(e) => setGainText(e.target.value)}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        {annulusHalfFilled && <div className="text-[9px] text-amber-400/90">{ANNULUS_NEEDS_BOTH}</div>}
        {apertureRadius === undefined && <div className="text-[9px] text-amber-400/90">Aperture radius must be between 2 and 60 px.</div>}

        <Toggle label="Track drift (phase correlation)" checked={trackDrift} accent="amber" onChange={setTrackDrift} />

        <div className="flex items-center gap-1.5">
          <div className="flex-1">
            <RunButton
              label={`Measure ${frames.length} ${frames.length === 1 ? "frame" : "frames"}`}
              runningLabel={`Measuring ${progress.percent}%`}
              running={running}
              disabled={!canMeasure}
              accent="amber"
              icon={<Clock size={12} />}
              onClick={() => void measure()}
            />
          </div>
          {running && (
            <button type="button" onClick={cancel} title="Cancel" aria-label="Cancel time series" className={SMALL_BUTTON_CLASS}>
              <X size={10} />
              Cancel
            </button>
          )}
        </div>
        {running && progress.active && (
          <div className="w-full h-1 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full bg-amber-400/80 transition-all duration-300" style={{ width: `${progress.percent}%` }} />
          </div>
        )}

        <ErrorAlert message={error} />
        <WarningList warnings={result?.warnings} />

        {result && analysis && (
          <>
            <div className={SECTION_CLASS}>Light curve</div>
            <div className="grid grid-cols-2 gap-1.5">
              <div className="flex flex-col gap-0.5">
                <label htmlFor={seriesId} className="text-[9px] text-zinc-500">
                  series
                </label>
                <select id={seriesId} value={activeKey} onChange={(e) => setSeriesKey(e.target.value)} className={SELECT_CLASS}>
                  {analysis.options.map((o) => (
                    <option key={o.key} value={o.key}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={axisId} className="text-[9px] text-zinc-500">
                  time axis
                </label>
                <select
                  id={axisId}
                  value={analysis.axis}
                  disabled={!hasCompleteTimeAxis(result)}
                  onChange={(e) => setTimeAxis(e.target.value as TimeAxis)}
                  className={SELECT_CLASS}
                >
                  <option value="jd">JD - JD0</option>
                  <option value="index">frame index</option>
                </select>
              </div>
            </div>

            {series.length > 0 ? (
              <ProfilePlot
                series={series}
                xLabel={xLabel}
                yLabel={yLabel}
                height={PLOT_HEIGHT_PX}
                invertY
                toolbar
                onSelect={showFrame}
                selected={selected}
                csvName="lightcurve"
                xTickFormat={xTickFormat}
              />
            ) : (
              <div className="text-[10px] text-zinc-600">No series to plot: mark a target and at least one comparison star.</div>
            )}

            <div className="grid grid-cols-2 gap-1.5 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Target rms {targetLabel ? `(${targetLabel})` : ""}</div>
                <div className="text-amber-300 font-mono">{fmt(analysis.targetRms, 4)} mag</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Check-star rms</div>
                <div className="text-zinc-300 font-mono">{fmt(analysis.checkRms, 4)} mag</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Frames used</div>
                <div className="text-zinc-300 font-mono">
                  {analysis.used} / {result.n_frames}
                  {result.n_skipped > 0 && <span className="text-amber-400/90"> ({result.n_skipped} skipped)</span>}
                </div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">JD0</div>
                <div className="text-zinc-300 font-mono">{analysis.axis === "jd" ? analysis.jd0 : "no frame time"}</div>
              </div>
            </div>

            <div className={SECTION_CLASS}>Table</div>
            <div className="max-h-56 overflow-auto border border-zinc-800/60 rounded">
              <table className="w-full text-[9px] font-mono">
                <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
                  <tr>
                    <th className="text-right px-1.5 py-0.5">#</th>
                    <th className="text-left px-1.5 py-0.5">file</th>
                    <th className="text-right px-1.5 py-0.5">JD</th>
                    <th className="text-right px-1.5 py-0.5">airmass</th>
                    <th className="text-right px-1.5 py-0.5">diff mag</th>
                    <th className="text-right px-1.5 py-0.5">SNR</th>
                    <th className="text-right px-1.5 py-0.5">FWHM</th>
                    <th className="text-right px-1.5 py-0.5">sky</th>
                    <th className="text-right px-1.5 py-0.5">dx/dy</th>
                    <th className="text-left px-1.5 py-0.5">flags</th>
                  </tr>
                </thead>
                <tbody>
                  {shownFrames.map((frame) => {
                    const point = analysis.targetCurve[frame.index];
                    const target = analysis.targetIdx >= 0 ? frame.targets[analysis.targetIdx] : null;
                    const lowConfidence = frame.offset !== null && !frame.offset.registered;
                    const jump = analysis.jumps.has(frame.index);
                    const isCurrent = frame.index === currentFrameIndex;
                    const tone = frame.skipped ? "text-zinc-500" : isCurrent ? "text-amber-200" : "text-zinc-300";
                    return (
                      <tr key={frame.index} className={`${tone} ${isCurrent ? "bg-zinc-800/60" : ""}`}>
                        <td className="text-right px-1.5 py-0.5">{frame.index}</td>
                        <td className="text-left px-1.5 py-0.5 truncate max-w-[90px]" title={frame.path}>
                          {frame.file_name}
                        </td>
                        <td className="text-right px-1.5 py-0.5">
                          {frame.jd_mid === null ? "--" : analysis.axis === "jd" ? fmt(frame.jd_mid - analysis.jd0, 4) : fmt(frame.jd_mid, 3)}
                        </td>
                        <td className="text-right px-1.5 py-0.5">{fmt(frame.airmass, 3)}</td>
                        <td className="text-right px-1.5 py-0.5">
                          {point && point.mag !== null ? `${fmt(point.mag, 4)} +/- ${fmt(point.err, 4)}` : "--"}
                        </td>
                        <td className="text-right px-1.5 py-0.5">{fmt(target?.snr, 1)}</td>
                        <td className="text-right px-1.5 py-0.5">{fmt(target?.fwhm, 2)}</td>
                        <td className="text-right px-1.5 py-0.5">{fmt(target?.bg_mean, 2)}</td>
                        <td className="text-right px-1.5 py-0.5">
                          {frame.offset ? `${fmt(frame.offset.dx, 2)}/${fmt(frame.offset.dy, 2)}` : "--"}
                        </td>
                        <td className="text-left px-1.5 py-0.5 max-w-[140px]">
                          {target?.saturated && <span className={`${CHIP_CLASS} bg-red-900/50 text-red-300`}>saturated</span>}
                          {lowConfidence && <span className={`${CHIP_CLASS} bg-amber-900/50 text-amber-300`}>low-confidence drift</span>}
                          {jump && <span className={`${CHIP_CLASS} bg-amber-900/50 text-amber-300`}>centroid jump</span>}
                          {frame.skipped && (
                            <span className={`${CHIP_CLASS} bg-zinc-800 text-zinc-400`} title={frame.skipped}>
                              {frame.skipped}
                            </span>
                          )}
                          {!frame.skipped && analysis.targetIdx >= 0 && frame.errors[analysis.targetIdx] && (
                            <span className={`${CHIP_CLASS} bg-red-900/50 text-red-300`} title={frame.errors[analysis.targetIdx] ?? undefined}>
                              {frame.errors[analysis.targetIdx]}
                            </span>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              {result.frames.length > TABLE_ROW_LIMIT && (
                <div className="text-[9px] text-zinc-600 px-1.5 py-0.5">
                  showing {TABLE_ROW_LIMIT} of {result.frames.length}; the CSV holds every row
                </div>
              )}
            </div>

            <div className="flex flex-wrap items-center gap-1.5">
              <button type="button" onClick={() => void copyCsv()} disabled={analysis.targetIdx < 0} className={SMALL_BUTTON_CLASS}>
                <ClipboardCopy size={10} />
                Copy CSV
              </button>
              <button type="button" onClick={() => void saveCsv()} disabled={analysis.targetIdx < 0} className={SMALL_BUTTON_CLASS}>
                <Download size={10} />
                Save CSV
              </button>
              <span className="text-[9px] text-zinc-500 font-mono ml-auto">{result.elapsed_ms} ms</span>
            </div>
            {savedPath && <div className="text-[9px] text-emerald-400/90 break-all">Saved {savedPath}</div>}
          </>
        )}
      </div>
    </div>
  );
}

export default memo(TimeSeriesPanel);
