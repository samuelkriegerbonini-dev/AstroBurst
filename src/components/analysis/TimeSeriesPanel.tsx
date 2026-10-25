import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ClipboardCopy, Clock, Download, Loader2, X } from "lucide-react";
import ProfilePlot, { type PlotHit, type ProfileSeries } from "../regions/ProfilePlot";
import { photometryGainModel, timeSeriesPhotometry, type GainModel } from "../../services/analysis";
import { measurementLog, timeSeriesEntry } from "../../utils/measurementLog";
import { cancelProgress } from "../../services/progress";
import {
  TIME_SERIES_PROGRESS_EVENT,
  type TimeSeriesResult,
  type TimeSeriesRole,
  type TimeSeriesTarget,
} from "../../shared/types/analysis";
import { useDqContext, useFileContext, useRenderContext } from "../../context/PreviewContext";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useProgress } from "../../hooks/useProgress";
import { useAnalysisTarget } from "../../hooks/useAnalysisTarget";
import { orderWithReferenceFirst, pathOf, useMatchingFrames } from "../../hooks/useFrameSet";
import { fileStore } from "../../hooks/useFileStore";
import {
  MAX_TIME_SERIES_TARGETS,
  centroidJumps,
  checkReference,
  checkRmsLabel,
  checkStarCurve,
  frameFilesNotice,
  frameJdLabel,
  frameMeasurementErrors,
  frameTimes,
  inFrameOrder,
  jdOffsetLabel,
  jdZero,
  lightCurve,
  lightCurveCsv,
  lightCurveCsvFileName,
  lightCurveExtras,
  measuredTargets,
  referenceTimeSource,
  resolveTimeAxis,
  resultCoversPath,
  seriesRms,
  timeAxisAvailability,
  timeAxisLabel,
  timeSeriesRoleHint,
  type LightCurvePoint,
  type TimeAxis,
} from "../../utils/differentialPhotometry";
import {
  EXOCLOCK_TIMES,
  defaultPointLabel,
  etdBlocker,
  etdFileName,
  etdText,
  exoClockBlocker,
  exoClockCsv,
  exoClockFileName,
  exportBlockerNotes,
  withCurrentLabels,
  type ExoClockTime,
  type LightCurveExportSource,
} from "../../utils/lightCurveExports";
import { loadSiteOverride } from "../../utils/observationGeometry";
import { MAX_SKY_OUTER_RADIUS_PX, apertureRadiusInRange, fluxCellText } from "../../utils/photometryTable";
import { apertureRangeHint } from "../../utils/analysisTarget";
import { GAIN_TITLE, gainCaption, loadGainPrefill, nextGainText } from "../../utils/photometryGain";
import { decimalFieldState, firstFieldError, type DecimalFieldOptions } from "../../utils/decimalText";
import { displaysFileGrid, otherGridHint } from "../../utils/stackingOutputs";
import { abandonRun, beginRun, createRunTracker, isCurrentRun, settleRun } from "../../utils/runTracker";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import { DecimalField, ErrorAlert, RunButton, Toggle, WarningList } from "../ui";

interface TimeSeriesPanelProps {
  filePath: string | null;
}

interface SeriesOption {
  key: string;
  label: string;
}

interface SaveDialog {
  defaultPath: string;
  filters: { name: string; extensions: string[] }[];
  title: string;
}

const DEFAULT_APERTURE_RADIUS_PX = "5";
const TABLE_ROW_LIMIT = 300;
const SAVED_NOTICE_MS = 6000;
const PLOT_HEIGHT_PX = 170;
const ROLE_OPTIONS: TimeSeriesRole[] = ["target", "comp", "check", "ignore"];
const SERIES_TARGET = "target";
const SERIES_RAW = "raw";
const TARGET_COLOR = "#fbbf24";
const CHECK_COLOR = "#67e8f9";
const RAW_COLORS = ["#fbbf24", "#67e8f9", "#a5b4fc", "#86efac", "#f9a8d4", "#fdba74"];
const ANNULUS_NEEDS_BOTH = "sky annulus needs both an inner and an outer radius";
const SKY_DEFAULTS_CAPTION = "blank sky: 2–3 × r_ap";
const SKY_IN_TITLE = "Inner sky radius in pixels; blank uses 2 × r_ap";
const SKY_OUT_TITLE = "Outer sky radius in pixels; blank uses 3 × r_ap";
const STARS_HINT =
  "Draw Point regions on the target and comparison stars, or use Add as Point regions in the photometry table.";
const FRAMES_HINT = "Load the other frames of the sequence; only done files with the same dimensions are measured.";
const NO_PLOT_HINT = "No frame has a positive target and comparison flux to plot.";
const BJD_AXIS_HINT = "BJD_TDB needs a time, a target and no skipped-geometry frame";
const AIRMASS_COLUMN_TITLE = "header AIRMASS, else Kasten & Young 1989";
const ETD_TITLE = "ETD (var.astro.cz): JD_UT, V-C, err as Siril writes it; upload with Time standard Gjd and Brightness in Magnitudes";
const EXOCLOCK_TITLE =
  "ExoClock: mid-exposure time, relative flux and detrending columns; set the upload's time format to the chosen column and the flux format to flux";
const EXOCLOCK_TIME_TITLE = "Time column of the ExoClock file; ExoClock recommends JD_UTC and converts to BJD_TDB itself";

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";
const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const CHIP_CLASS = "inline-block rounded px-1 py-px text-[8px] leading-tight mr-0.5";

const APERTURE_FIELD: DecimalFieldOptions = {};
const SKY_IN_FIELD: DecimalFieldOptions = { optional: true, min: 1 };
const SKY_OUT_FIELD: DecimalFieldOptions = { optional: true, min: 1, max: MAX_SKY_OUTER_RADIUS_PX };
const GAIN_FIELD: DecimalFieldOptions = { optional: true, min: 0.01 };

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

interface PointTarget {
  id: string;
  target: TimeSeriesTarget;
}

function pointTargets(regions: ReturnType<typeof useRegionDoc>["regions"], roles: Record<string, TimeSeriesRole>): PointTarget[] {
  const rows: PointTarget[] = [];
  for (const region of regions) {
    if (region.shape.shape !== "point") continue;
    const index = rows.length;
    rows.push({
      id: region.id,
      target: {
        x: region.shape.x,
        y: region.shape.y,
        label: region.props.text || defaultPointLabel(index),
        role: roles[region.id] ?? defaultRole(index),
      },
    });
  }
  return rows;
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
  const { processed } = useRenderContext();
  const analysisTarget = useAnalysisTarget();
  const regionDoc = useRegionDoc(filePath);
  const progress = useProgress(TIME_SERIES_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [roles, setRoles] = useState<Record<string, TimeSeriesRole>>({});
  const [apertureText, setApertureText] = useState(DEFAULT_APERTURE_RADIUS_PX);
  const [skyInText, setSkyInText] = useState("");
  const [skyOutText, setSkyOutText] = useState("");
  const [gainText, setGainText] = useState("");
  const [gainModel, setGainModel] = useState<GainModel | null>(null);
  const [trackDrift, setTrackDrift] = useState(true);
  const [seriesKey, setSeriesKey] = useState(SERIES_TARGET);
  const [timeAxis, setTimeAxis] = useState<TimeAxis>("bjd");
  const [exoClockTime, setExoClockTime] = useState<ExoClockTime>("jd_utc");
  const [result, setResult] = useState<TimeSeriesResult | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, showSavedPath, clearSavedPath] = useTimedNotice<string>(SAVED_NOTICE_MS);
  const runRef = useRef(createRunTracker());
  const resultRef = useRef<TimeSeriesResult | null>(null);
  const gainTextRef = useRef(gainText);
  gainTextRef.current = gainText;
  const gainPrefillRef = useRef("");
  const gainSeqRef = useRef(0);

  const abandon = useCallback(() => {
    if (abandonRun(runRef.current)) cancelProgress(TIME_SERIES_PROGRESS_EVENT).catch(() => {});
  }, []);

  useEffect(() => {
    resultRef.current = result;
  }, [result]);

  useEffect(() => {
    if (resultCoversPath(resultRef.current, filePath)) return;
    abandon();
    setResult(null);
    setError(null);
    setRunning(false);
    clearSavedPath();
    setRoles({});
    setSeriesKey(SERIES_TARGET);
  }, [filePath, abandon, clearSavedPath]);

  useEffect(() => abandon, [abandon]);

  useEffect(() => {
    const seq = ++gainSeqRef.current;
    setGainModel(null);
    const cleared = nextGainText(gainTextRef.current, gainPrefillRef.current, null);
    gainPrefillRef.current = cleared.prefill;
    gainTextRef.current = cleared.text;
    setGainText(cleared.text);
    void loadGainPrefill(filePath, () => gainTextRef.current, cleared.prefill, photometryGainModel).then((next) => {
      if (gainSeqRef.current !== seq) return;
      gainPrefillRef.current = next.prefill;
      setGainModel(next.model);
      setGainText(next.text);
    });
  }, [filePath]);

  const dims = file?.result?.dimensions ?? null;
  const matching = useMatchingFrames(dims);
  const frames = useMemo(() => orderWithReferenceFirst(matching, filePath), [matching, filePath]);
  const referenceLoaded = frames.length > 0 && pathOf(frames[0]) === filePath;

  const targets = useMemo(() => pointTargets(regionDoc.regions, roles), [regionDoc, roles]);
  const referenceDoc = useRegionDoc(result?.reference_path ?? filePath);
  const referenceTargets = useMemo(() => pointTargets(referenceDoc.regions, roles), [referenceDoc, roles]);

  const apertureField = decimalFieldState(apertureText, APERTURE_FIELD);
  const skyInField = decimalFieldState(skyInText, SKY_IN_FIELD);
  const skyOutField = decimalFieldState(skyOutText, SKY_OUT_FIELD);
  const gainField = decimalFieldState(gainText, GAIN_FIELD);
  const apertureRadius = apertureField.value ?? undefined;
  const annulusInner = skyInField.value ?? undefined;
  const annulusOuter = skyOutField.value ?? undefined;
  const gain = gainField.value ?? undefined;
  const fieldError = firstFieldError([
    ["r_ap px", apertureField],
    ["sky in px", skyInField],
    ["sky out px", skyOutField],
    ["gain e-/ADU", gainField],
  ]);
  const annulusHalfFilled = (annulusInner === undefined) !== (annulusOuter === undefined);
  const apertureValid = apertureRadius !== undefined && apertureRadiusInRange(apertureRadius);
  const apertureHint = apertureRangeHint(apertureField);
  const sent = useMemo(() => measuredTargets(targets.map((t) => t.target)), [targets]);
  const overLimit = sent.length > MAX_TIME_SERIES_TARGETS;
  const roleHint = timeSeriesRoleHint(targets.map((t) => t.target.role));
  const onFileGrid = !!filePath && displaysFileGrid(processed, filePath);
  const sourceNotice = frameFilesNotice({
    compositeOnScreen: analysisTarget.composite,
    processedLabel: analysisTarget.displayed.isProcessed ? analysisTarget.displayed.label : null,
  });
  const canMeasure =
    !!filePath &&
    referenceLoaded &&
    onFileGrid &&
    roleHint === null &&
    apertureValid &&
    fieldError === null &&
    !annulusHalfFilled &&
    !overLimit &&
    !running;

  const measure = useCallback(async () => {
    if (!canMeasure || apertureRadius === undefined) return;
    const seq = beginRun(runRef.current);
    const storeOrder = matching.map(pathOf);
    setRunning(true);
    setError(null);
    clearSavedPath();
    resetProgress();
    try {
      const site = loadSiteOverride(window.localStorage);
      const res = await timeSeriesPhotometry(frames.map(pathOf), sent, {
        apertureRadius,
        annulusInner,
        annulusOuter,
        gain,
        excludeDq,
        trackDrift,
        siteLat: site?.lat ?? null,
        siteLon: site?.lon ?? null,
        siteHeight: site?.height ?? null,
      });
      if (!isCurrentRun(runRef.current, seq)) return;
      setResult(inFrameOrder(res, storeOrder));
      measurementLog.append(timeSeriesEntry(res, { apertureRadius, annulusInner, annulusOuter, gain, excludeDq, trackDrift }));
      setSeriesKey(SERIES_TARGET);
    } catch (e: unknown) {
      if (isCurrentRun(runRef.current, seq)) setError(e instanceof Error ? e.message : String(e));
    } finally {
      settleRun(runRef.current, seq);
      if (isCurrentRun(runRef.current, seq)) {
        setRunning(false);
        resetProgress();
      }
    }
  }, [canMeasure, apertureRadius, matching, frames, sent, annulusInner, annulusOuter, gain, excludeDq, trackDrift, resetProgress, clearSavedPath]);

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
    const axis = resolveTimeAxis(result, timeAxis);
    const targetCurve = targetIdx >= 0 ? lightCurve(result, targetIdx, compIdx, axis) : [];
    const checkCurves = checkIdx.map((i) => lightCurve(result, i, compIdx, axis));
    const compCurves = compIdx.length >= 2 ? compIdx.map((i) => checkStarCurve(result, i, compIdx, axis)) : [];
    const options: SeriesOption[] = [];
    if (targetIdx >= 0) options.push({ key: SERIES_TARGET, label: `target ${echo[targetIdx].label}` });
    checkIdx.forEach((i) => options.push({ key: `check:${i}`, label: `check star ${echo[i].label}` }));
    if (compCurves.length > 0) compIdx.forEach((i) => options.push({ key: `comp:${i}`, label: `comp ${echo[i].label} vs others` }));
    if (compIdx.length > 0) options.push({ key: SERIES_RAW, label: "raw comps" });
    const reference = checkReference(result, compIdx, checkIdx, axis);
    const timeline = frameTimes(result, axis);
    const jd0 = axis !== "index" ? jdZero(timeline) : 0;
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
      checkRms: reference ? seriesRms(reference.curve) : null,
      checkLabel: checkRmsLabel(reference, echo),
      timeSource: referenceTimeSource(result),
      used: targetCurve.filter((p) => p.mag !== null).length,
      jumps: new Set(centroidJumps(result)),
    };
  }, [result, timeAxis]);

  const activeKey = analysis && analysis.options.some((o) => o.key === seriesKey) ? seriesKey : (analysis?.options[0]?.key ?? SERIES_TARGET);

  const series = useMemo((): ProfileSeries[] => {
    if (!result || !analysis) return [];
    const { axis, jd0 } = analysis;
    const xOf = (p: { t: number }) => (axis !== "index" ? p.t - jd0 : p.t);
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

  const saveText = useCallback(async (dialog: SaveDialog, text: () => string) => {
    setError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save(dialog);
      if (!target) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, text());
      showSavedPath(target);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [showSavedPath]);

  const saveCsv = useCallback(async () => {
    if (!result) return;
    await saveText(
      { defaultPath: lightCurveCsvFileName(result.reference_path), filters: [{ name: "CSV", extensions: ["csv"] }], title: "Save light curve" },
      csvText,
    );
  }, [result, csvText, saveText]);

  const exportSource = useMemo((): LightCurveExportSource | null => {
    if (!result || !analysis) return null;
    const labelled = withCurrentLabels(result, referenceTargets.map((t) => t.target));
    return { result: labelled, targetIdx: analysis.targetIdx, compIdx: analysis.compIdx };
  }, [result, analysis, referenceTargets]);
  const etdBlock = useMemo(() => (exportSource ? etdBlocker(exportSource) : null), [exportSource]);
  const exoClockBlock = useMemo(() => (exportSource ? exoClockBlocker(exportSource, exoClockTime) : null), [exportSource, exoClockTime]);
  const exportNotes = useMemo(() => exportBlockerNotes(etdBlock, exoClockBlock), [etdBlock, exoClockBlock]);

  const saveEtd = useCallback(async () => {
    if (!exportSource || etdBlock) return;
    await saveText(
      { defaultPath: etdFileName(exportSource), filters: [{ name: "ETD text", extensions: ["txt"] }], title: "Save ETD light curve" },
      () => etdText(exportSource),
    );
  }, [exportSource, etdBlock, saveText]);

  const saveExoClock = useCallback(async () => {
    if (!exportSource || exoClockBlock) return;
    await saveText(
      { defaultPath: exoClockFileName(exportSource, exoClockTime), filters: [{ name: "CSV", extensions: ["csv"] }], title: "Save ExoClock light curve" },
      () => exoClockCsv(exportSource, exoClockTime),
    );
  }, [exportSource, exoClockBlock, exoClockTime, saveText]);

  const xLabel = timeAxisLabel(analysis?.axis ?? "index", analysis?.jd0 ?? 0);
  const axes = useMemo(() => (result ? timeAxisAvailability(result) : { jd: false, bjd: false, any: false }), [result]);
  const yLabel = activeKey === SERIES_RAW ? "inst mag" : "diff mag";
  const xTickFormat = analysis && analysis.axis !== "index" ? jdOffsetLabel : undefined;
  const plottable = series.some((s) => s.y.some((v) => v !== null && Number.isFinite(v)));
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
            {roleHint && <div className="text-[9px] text-amber-400/90">{roleHint}</div>}
            {overLimit && (
              <div className="text-[9px] text-amber-400/90">
                {`${sent.length} stars are not ignored; set the others to ignore (limit ${MAX_TIME_SERIES_TARGETS}).`}
              </div>
            )}
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
        {!onFileGrid && !!filePath ? (
          <div className="text-[9px] text-amber-400/90">{otherGridHint(processed?.label ?? null)}</div>
        ) : (
          sourceNotice && <div className="text-[9px] text-amber-400/90">{sourceNotice}</div>
        )}

        <div className="grid grid-cols-4 gap-1.5">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={apertureId} className="text-[9px] text-zinc-500">
              r_ap px
            </label>
            <DecimalField
              id={apertureId}
              {...APERTURE_FIELD}
              value={apertureText}
              onChange={setApertureText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyInId} className="text-[9px] text-zinc-500">
              sky in px
            </label>
            <DecimalField
              id={skyInId}
              {...SKY_IN_FIELD}
              value={skyInText}
              placeholder="2×r"
              title={SKY_IN_TITLE}
              onChange={setSkyInText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={skyOutId} className="text-[9px] text-zinc-500">
              sky out px
            </label>
            <DecimalField
              id={skyOutId}
              {...SKY_OUT_FIELD}
              value={skyOutText}
              placeholder="3×r"
              title={SKY_OUT_TITLE}
              onChange={setSkyOutText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={gainId} className="text-[9px] text-zinc-500">
              gain e-/ADU
            </label>
            <DecimalField
              id={gainId}
              {...GAIN_FIELD}
              value={gainText}
              placeholder="none"
              title={GAIN_TITLE}
              data-gain-source={gainModel?.source ?? ""}
              onChange={setGainText}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        <div className="text-[9px] text-zinc-600">{SKY_DEFAULTS_CAPTION}</div>
        <div className="text-[9px] text-zinc-500" data-gain-caption>
          {gainCaption(gainModel, gainText)}
        </div>
        {annulusHalfFilled && <div className="text-[9px] text-amber-400/90">{ANNULUS_NEEDS_BOTH}</div>}
        {apertureHint && <div className="text-[9px] text-amber-400/90">{apertureHint}</div>}
        {fieldError && (
          <div className="text-[9px] text-amber-400/90" data-field-error>
            {fieldError}
          </div>
        )}

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
        {result && result.geometry_notes.length > 0 && (
          <details className="text-[9px] text-zinc-500">
            <summary className="cursor-pointer select-none">Geometry notes</summary>
            <ul className="mt-1 space-y-0.5 list-disc pl-4">
              {result.geometry_notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </details>
        )}

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
                  disabled={!axes.any}
                  onChange={(e) => setTimeAxis(e.target.value as TimeAxis)}
                  className={SELECT_CLASS}
                >
                  <option value="jd" disabled={!axes.jd}>
                    JD - JD0 (header time)
                  </option>
                  <option value="bjd" disabled={!axes.bjd}>
                    BJD_TDB - BJD0
                  </option>
                  <option value="index">frame index</option>
                </select>
                {!axes.bjd && <div className="text-[9px] text-zinc-600">{BJD_AXIS_HINT}</div>}
              </div>
            </div>

            {plottable ? (
              <ProfilePlot
                series={series}
                xLabel={xLabel}
                yLabel={yLabel}
                height={PLOT_HEIGHT_PX}
                invertY
                toolbar
                logToggle={false}
                onSelect={showFrame}
                selected={selected}
                csvName="lightcurve"
                xTickFormat={xTickFormat}
              />
            ) : (
              <div className="text-[10px] text-zinc-600">{NO_PLOT_HINT}</div>
            )}

            <div className="grid grid-cols-2 gap-1.5 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">Target rms {targetLabel ? `(${targetLabel})` : ""}</div>
                <div className="text-amber-300 font-mono">{fmt(analysis.targetRms, 4)} mag</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1.5">
                <div className="text-zinc-500">{analysis.checkLabel}</div>
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
                <div className="text-zinc-500">{analysis.axis === "bjd" ? "BJD0 (TDB)" : "JD0"}</div>
                <div className="text-zinc-300 font-mono">{analysis.axis !== "index" ? analysis.jd0 : "no frame time"}</div>
                {analysis.timeSource && (
                  <div className="text-[9px] text-zinc-500 truncate" title={analysis.timeSource}>
                    {analysis.timeSource}
                  </div>
                )}
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
                    <th className="text-right px-1.5 py-0.5">BJD_TDB</th>
                    <th className="text-right px-1.5 py-0.5" title={AIRMASS_COLUMN_TITLE}>
                      airmass
                    </th>
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
                    const measurementErrors =
                      analysis.targetIdx >= 0 ? frameMeasurementErrors(frame, result.targets, analysis.targetIdx, analysis.compIdx) : [];
                    const tone = frame.skipped ? "text-zinc-500" : isCurrent ? "text-amber-200" : "text-zinc-300";
                    return (
                      <tr key={frame.index} className={`${tone} ${isCurrent ? "bg-zinc-800/60" : ""}`}>
                        <td className="text-right px-1.5 py-0.5">{frame.index}</td>
                        <td className="text-left px-1.5 py-0.5 truncate max-w-[90px]" title={frame.path}>
                          {frame.file_name}
                        </td>
                        <td className="text-right px-1.5 py-0.5">
                          {frameJdLabel(frame.jd_mid, analysis.axis === "jd" ? "jd" : "index", analysis.jd0)}
                        </td>
                        <td className="text-right px-1.5 py-0.5">
                          {frameJdLabel(frame.geometry?.bjd_tdb ?? null, analysis.axis === "bjd" ? "bjd" : "index", analysis.jd0)}
                        </td>
                        <td className="text-right px-1.5 py-0.5" title={AIRMASS_COLUMN_TITLE}>
                          {fmt(frame.airmass ?? frame.geometry?.airmass_computed ?? null, 3)}
                        </td>
                        <td className="text-right whitespace-nowrap px-1.5 py-0.5">
                          {point && point.mag !== null ? `${fmt(point.mag, 4)} ± ${fmt(point.err, 4)}` : "--"}
                        </td>
                        <td className="text-right px-1.5 py-0.5">{fmt(target?.snr, 1)}</td>
                        <td className="text-right px-1.5 py-0.5">{fmt(target?.fwhm, 2)}</td>
                        <td className="text-right px-1.5 py-0.5" data-column="bg_mean">
                          {fluxCellText(target?.bg_mean)}
                        </td>
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
                          {measurementErrors.map((message) => (
                            <span key={message} className={`${CHIP_CLASS} bg-red-900/50 text-red-300`} title={message}>
                              {message}
                            </span>
                          ))}
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
              <button
                type="button"
                onClick={() => void saveEtd()}
                disabled={etdBlock !== null}
                title={etdBlock ?? ETD_TITLE}
                data-export="etd"
                className={SMALL_BUTTON_CLASS}
              >
                <Download size={10} />
                ETD
              </button>
              <button
                type="button"
                onClick={() => void saveExoClock()}
                disabled={exoClockBlock !== null}
                title={exoClockBlock ?? EXOCLOCK_TITLE}
                data-export="exoclock"
                className={SMALL_BUTTON_CLASS}
              >
                <Download size={10} />
                ExoClock
              </button>
              <select
                value={exoClockTime}
                aria-label="ExoClock time column"
                title={EXOCLOCK_TIME_TITLE}
                onChange={(e) => setExoClockTime(e.target.value as ExoClockTime)}
                className="bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono"
              >
                {EXOCLOCK_TIMES.map((time) => (
                  <option key={time} value={time}>
                    {time}
                  </option>
                ))}
              </select>
              <span className="text-[9px] text-zinc-500 font-mono ml-auto">{result.elapsed_ms} ms</span>
            </div>
            {exportNotes.map((note) => (
              <div key={note.key} className="text-[9px] text-zinc-500" data-export-blocker={note.key}>
                {note.text}
              </div>
            ))}
            {savedPath && <div className="text-[9px] text-emerald-400/90 break-all">Saved {savedPath}</div>}
          </>
        )}
      </div>
    </div>
  );
}

export default memo(TimeSeriesPanel);
