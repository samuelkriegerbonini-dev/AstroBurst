import { memo, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Clock, Download, Loader2, X } from "lucide-react";
import LightCurveView from "./LightCurveView";
import LightCurveTablePanel from "./LightCurveTablePanel";
import PhotometryStatusLine from "./PhotometryStatusLine";
import { photometryGainModel, timeSeriesPhotometry, type GainModel } from "../../services/analysis";
import { measurementLog, timeSeriesEntry, type TimeSeriesLogOptions } from "../../utils/measurementLog";
import { cancelProgress } from "../../services/progress";
import {
  TIME_SERIES_PROGRESS_EVENT,
  type TimeSeriesResult,
  type TimeSeriesRole,
  type TimeSeriesTarget,
} from "../../shared/types/analysis";
import type { Region } from "../../shared/types/regions";
import type { LightCurveDoc } from "../../shared/types/lightCurve";
import { useDoneFilesContext, useDqContext, useFileContext, useRenderContext } from "../../context/PreviewContext";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useProgress } from "../../hooks/useProgress";
import { useAnalysisTarget } from "../../hooks/useAnalysisTarget";
import { orderWithReferenceFirst, pathOf, useMatchingFrames } from "../../hooks/useFrameSet";
import { fileStore } from "../../hooks/useFileStore";
import {
  lightCurveStore,
  useActiveLightCurveTable,
  useLightCurveGroups,
  useLightCurveView,
  useLightCurves,
} from "../../hooks/useLightCurveStore";
import { lightCurveTableLoader, openLightCurveTable, tablePathOf } from "../../hooks/useLightCurveTable";
import { tableRouteStore, useTableRoute } from "../../hooks/useTableRouteStore";
import { lightCurveModel, type PointState } from "../../utils/lightCurveViewModel";
import { regionStore } from "../../utils/regionStore";
import { cardText } from "../../utils/frameKind";
import {
  MAX_TIME_SERIES_TARGETS,
  bjdAxisHint,
  bjdSources,
  centroidJumps,
  checkReference,
  checkRmsLabel,
  defaultSeriesRoles,
  frameFilesNotice,
  frameJdLabel,
  frameMeasurementErrors,
  frameTimes,
  frameUnits,
  geometryNotes,
  otherFrameNotes,
  inFrameOrder,
  jdZero,
  lightCurve,
  lightCurveCsvFileName,
  lightCurveCsvText,
  measuredTargets,
  referenceTimeSource,
  resolveTimeAxis,
  resultCoversPath,
  saturatedCompLabels,
  saturatedComps,
  seriesRms,
  seriesTableHeaders,
  timeAxisAvailability,
  timeSeriesRoleHint,
  withExclusiveTarget,
  type SaturatedComp,
  type TimeAxis,
} from "../../utils/differentialPhotometry";
import {
  BJD_ACCURACY_NOTE,
  EXOCLOCK_TIMES,
  defaultPointLabel,
  etdBlocker,
  etdFileName,
  etdText,
  exoClockBlocker,
  exoClockCsv,
  exoClockFileName,
  exportBlockerNotes,
  leftOutReasons,
  leftOutSummary,
  withCurrentLabels,
  type ExoClockTime,
  type LightCurveExportSource,
} from "../../utils/lightCurveExports";
import { frameGroupKey, lightCurveDocFromFrames, rawFluxDocFromFrames } from "../../utils/lightCurveDoc";
import { loadSiteOverride, parseTargetInputs } from "../../utils/observationGeometry";
import { fileBaseName } from "../../utils/values";
import { MAX_SKY_OUTER_RADIUS_PX, apertureRadiusInRange, fluxCellText } from "../../utils/photometryTable";
import { apertureRangeHint } from "../../utils/analysisTarget";
import { GAIN_TITLE, gainCaption, loadGainPrefill, nextGainText } from "../../utils/photometryGain";
import { decimalFieldState, firstFieldError, type DecimalFieldOptions } from "../../utils/decimalText";
import { displaysFileGrid, otherGridHint } from "../../utils/stackingOutputs";
import { abandonRun, beginRun, createRunTracker, isCurrentRun, settleRun } from "../../utils/runTracker";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import { CopyCsvButton, DecimalField, ErrorAlert, RunButton, SMALL_BUTTON_CLASS, Toggle, WarningList } from "../ui";

interface TimeSeriesPanelProps {
  filePath: string | null;
}

interface SaveDialog {
  defaultPath: string;
  filters: { name: string; extensions: string[] }[];
  title: string;
}

const DEFAULT_APERTURE_RADIUS_PX = "5";
const TABLE_ROW_LIMIT = 300;
const SAVED_NOTICE_MS = 6000;
const ROLE_OPTIONS: TimeSeriesRole[] = ["target", "comp", "check", "ignore"];
const RAW_COMPS_OVERLAY = "raw comps";
const ANNULUS_NEEDS_BOTH = "sky annulus needs both an inner and an outer radius";
const SKY_DEFAULTS_CAPTION = "blank sky: 2–3 × r_ap";
const SKY_IN_TITLE = "Inner sky radius in pixels; blank uses 2 × r_ap";
const SKY_OUT_TITLE = "Outer sky radius in pixels; blank uses 3 × r_ap";
const STARS_HINT =
  "Draw Point regions on the target and comparison stars, or use Add as Point regions in the photometry table.";
const FRAMES_HINT = "Load the other frames of the sequence; only done files with the same dimensions are measured.";
const REFERENCE_LEFT_OUT_HINT = "The current file is a calibration frame or a stacked result, so it cannot be the reference.";
const STAR_NAME_TITLE = "Name of the star, kept on its Point region; ETD and ExoClock need the target's designation";
const EARLIER_CURVE_NOTE = "Measured earlier in this session. Measure again for the table and the exports.";
const AIRMASS_COLUMN_TITLE = "header AIRMASS, else Kasten & Young 1989";
const SHOW_FRAME_TITLE = "Show this frame in the viewer";
const LEFT_OUT_STATES: readonly PointState[] = ["flagged", "manual", "clipped"];
const ETD_TITLE =
  "ETD (var.astro.cz): JD_UT, V-C, err as Siril writes it; V-C is the target against the sum of the comparison stars, 2.5 log10(N) mag above Siril's mean-of-comparisons value; upload with Time standard Gjd and Brightness in Magnitudes";
const EXOCLOCK_TITLE =
  "ExoClock: mid-exposure time, relative flux and detrending columns; set the upload's time format to the chosen column and the flux format to flux";
const EXOCLOCK_TIME_TITLE = "Time column of the ExoClock file; ExoClock recommends JD_UTC and converts to BJD_TDB itself";
const TARGET_PLACEHOLDER = "from the WCS or header";
const LIGHT_TIME_TARGET_CAPTION =
  "Target of the light-time correction (BJD_TDB, HJD_UTC); blank uses the WCS position of the target star, else the header coordinates.";

const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const NAME_CLASS = `${SELECT_CLASS} min-w-0`;
const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const CHIP_CLASS = "inline-block rounded px-1 py-px text-[9px] leading-tight mr-0.5";

const APERTURE_FIELD: DecimalFieldOptions = {};
const SKY_IN_FIELD: DecimalFieldOptions = { optional: true, min: 1 };
const SKY_OUT_FIELD: DecimalFieldOptions = { optional: true, min: 1, max: MAX_SKY_OUTER_RADIUS_PX };
const GAIN_FIELD: DecimalFieldOptions = { optional: true, min: 0.01 };
const TARGET_FIELD: DecimalFieldOptions = { optional: true };

function fmt(v: number | null | undefined, digits: number): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "--";
  return Math.abs(v) >= 1e6 ? v.toExponential(2) : v.toFixed(digits);
}

function saturatedCompText(comp: SaturatedComp, leftOut: boolean): string {
  const frames = `${comp.frames} ${comp.frames === 1 ? "frame" : "frames"}`;
  return `comparison star ${comp.label} saturated on ${frames}${leftOut ? ", left out of the ensemble" : ""}`;
}

interface PointTarget {
  id: string;
  region: Region;
  target: TimeSeriesTarget;
}

function pointTargets(regions: ReturnType<typeof useRegionDoc>["regions"], roles: Record<string, TimeSeriesRole>): PointTarget[] {
  const points = regions.flatMap((region) => (region.shape.shape === "point" ? [{ region, shape: region.shape }] : []));
  const seriesRoles = defaultSeriesRoles(points.map(({ region }) => region.id), roles);
  return points.map(({ region, shape }, index) => ({
    id: region.id,
    region,
    target: {
      x: shape.x,
      y: shape.y,
      label: region.props.text || defaultPointLabel(index),
      role: seriesRoles[index],
    },
  }));
}

function TimeSeriesPanel({ filePath }: TimeSeriesPanelProps) {
  const apertureId = useId();
  const skyInId = useId();
  const skyOutId = useId();
  const gainId = useId();
  const targetRaId = useId();
  const targetDecId = useId();
  const axisId = useId();
  const { file } = useFileContext();
  const { doneFiles } = useDoneFilesContext();
  const { excludeDq } = useDqContext();
  const { processed } = useRenderContext();
  const analysisTarget = useAnalysisTarget();
  const progress = useProgress(TIME_SERIES_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [roles, setRoles] = useState<Record<string, TimeSeriesRole>>({});
  const [apertureText, setApertureText] = useState(DEFAULT_APERTURE_RADIUS_PX);
  const [skyInText, setSkyInText] = useState("");
  const [skyOutText, setSkyOutText] = useState("");
  const [gainText, setGainText] = useState("");
  const [targetRaText, setTargetRaText] = useState("");
  const [targetDecText, setTargetDecText] = useState("");
  const [gainModel, setGainModel] = useState<GainModel | null>(null);
  const [trackDrift, setTrackDrift] = useState(true);
  const [timeAxis, setTimeAxis] = useState<TimeAxis>("bjd");
  const [exoClockTime, setExoClockTime] = useState<ExoClockTime>("jd_utc");
  const [result, setResult] = useState<TimeSeriesResult | null>(null);
  const [fluxUnit, setFluxUnit] = useState<string | null>(null);
  const [leftOut, setLeftOut] = useState<number[]>([]);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [seriesStatus, setSeriesStatus] = useState("");
  const [savedPath, showSavedPath, clearSavedPath] = useTimedNotice<string>(SAVED_NOTICE_MS);
  const runRef = useRef(createRunTracker());
  const resultRef = useRef<TimeSeriesResult | null>(null);
  const logOptionsRef = useRef<TimeSeriesLogOptions | null>(null);
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
    setLeftOut([]);
    logOptionsRef.current = null;
    setError(null);
    setRunning(false);
    clearSavedPath();
    setRoles({});
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

  const runReference = result && resultCoversPath(result, filePath) && filePath !== result.reference_path ? result.reference_path : null;
  const starsPath = runReference ?? filePath;
  const regionDoc = useRegionDoc(starsPath);

  const dims = file?.result?.dimensions ?? null;
  const matching = useMatchingFrames(doneFiles, dims);
  const frames = useMemo(() => orderWithReferenceFirst(matching.files, starsPath), [matching, starsPath]);
  const referenceLoaded = frames.length > 0 && pathOf(frames[0]) === starsPath;
  const referenceLeftOut = !referenceLoaded && doneFiles.some((f) => pathOf(f) === filePath);

  const targets = useMemo(() => pointTargets(regionDoc.regions, roles), [regionDoc, roles]);
  const ignoredByDefault = targets.filter((t) => t.target.role === "ignore" && roles[t.id] === undefined).length;
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
  const targetInputs = useMemo(() => parseTargetInputs(targetRaText, targetDecText), [targetRaText, targetDecText]);
  const inputError = fieldError ?? targetInputs.error;
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
    inputError === null &&
    !annulusHalfFilled &&
    !overLimit &&
    !running;

  const measure = useCallback(async () => {
    if (!canMeasure || apertureRadius === undefined) return;
    const seq = beginRun(runRef.current);
    const storeOrder = matching.files.map(pathOf);
    const referenceUnit = cardText(frames[0].result?.header?.BUNIT);
    setRunning(true);
    setError(null);
    setSeriesStatus("");
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
        targetRa: targetInputs.target?.ra ?? null,
        targetDec: targetInputs.target?.dec ?? null,
        siteLat: site?.lat ?? null,
        siteLon: site?.lon ?? null,
        siteHeight: site?.height ?? null,
      });
      if (!isCurrentRun(runRef.current, seq)) return;
      setResult(inFrameOrder(res, storeOrder));
      setFluxUnit(referenceUnit);
      setLeftOut([]);
      setSeriesStatus(`Measured ${res.n_frames} ${res.n_frames === 1 ? "frame" : "frames"}, ${res.n_skipped} skipped`);
      const logOptions = { apertureRadius, annulusInner, annulusOuter, gain, excludeDq, trackDrift };
      logOptionsRef.current = logOptions;
      measurementLog.append(timeSeriesEntry(res, logOptions));
    } catch (e: unknown) {
      if (isCurrentRun(runRef.current, seq)) setError(e instanceof Error ? e.message : String(e));
    } finally {
      settleRun(runRef.current, seq);
      if (isCurrentRun(runRef.current, seq)) {
        setRunning(false);
        resetProgress();
      }
    }
  }, [canMeasure, apertureRadius, matching, frames, sent, annulusInner, annulusOuter, gain, excludeDq, trackDrift, targetInputs, resetProgress, clearSavedPath]);

  const cancel = useCallback(() => {
    cancelProgress(TIME_SERIES_PROGRESS_EVENT).catch(() => {});
  }, []);

  const setRole = useCallback(
    (id: string, role: TimeSeriesRole) => {
      const ids = targets.map((t) => t.id);
      setRoles((prev) => withExclusiveTarget(prev, ids, id, role));
    },
    [targets],
  );

  const nameStar = useCallback(
    (region: Region, typed: string) => {
      const text = typed.trim();
      if (!starsPath || text === (region.props.text ?? "").trim()) return;
      regionStore.update(starsPath, region.id, { props: { ...region.props, text: text === "" ? null : text } });
    },
    [starsPath],
  );

  const showReference = useCallback(() => {
    const reference = frames.find((f) => pathOf(f) === runReference);
    if (reference) fileStore.selectFile(reference.id);
  }, [frames, runReference]);

  const toggleLeftOut = useCallback(
    (index: number) => {
      const next = leftOut.includes(index) ? leftOut.filter((i) => i !== index) : [...leftOut, index];
      setLeftOut(next);
      const logOptions = logOptionsRef.current;
      if (!result || !logOptions) return;
      const compIdx = result.targets.map((t, i) => (t.role === "comp" && !next.includes(i) ? i : -1)).filter((i) => i >= 0);
      measurementLog.append(timeSeriesEntry(result, { ...logOptions, compIdx }));
    },
    [leftOut, result],
  );

  const analysis = useMemo(() => {
    if (!result) return null;
    const echo = result.targets;
    const targetIdx = echo.findIndex((t) => t.role === "target");
    const compIdx = echo.map((t, i) => (t.role === "comp" && !leftOut.includes(i) ? i : -1)).filter((i) => i >= 0);
    const checkIdx = echo.map((t, i) => (t.role === "check" ? i : -1)).filter((i) => i >= 0);
    const axis = resolveTimeAxis(result, timeAxis);
    const targetCurve = targetIdx >= 0 ? lightCurve(result, targetIdx, compIdx, axis) : [];
    const reference = checkReference(result, compIdx, checkIdx, axis);
    const jd0 = axis !== "index" ? jdZero(frameTimes(result, axis)) : 0;
    const units = frameUnits(result);
    return {
      targetIdx,
      compIdx,
      checkIdx,
      axis,
      targetCurve,
      jd0,
      units,
      headers: seriesTableHeaders(axis, jd0, units.unit),
      targetRms: seriesRms(targetCurve),
      checkRms: reference ? seriesRms(reference.curve) : null,
      checkLabel: checkRmsLabel(reference, echo),
      timeSource: referenceTimeSource(result),
      used: targetCurve.filter((p) => p.mag !== null).length,
      jumps: new Set(centroidJumps(result)),
    };
  }, [result, timeAxis, leftOut]);

  const currentFrameIndex = useMemo(() => (result ? result.frames.findIndex((f) => f.path === filePath) : -1), [result, filePath]);

  const showFrameAt = useCallback(
    (index: number) => {
      const frame = result?.frames[index];
      const target = frame && frames.find((f) => pathOf(f) === frame.path);
      if (target) fileStore.selectFile(target.id);
    },
    [result, frames],
  );

  const showPoint = useCallback((_doc: LightCurveDoc, pointRef: number) => showFrameAt(pointRef), [showFrameAt]);

  const exportSource = useMemo((): LightCurveExportSource | null => {
    if (!result || !analysis) return null;
    const labelled = withCurrentLabels(result, referenceTargets.map((t) => t.target));
    return { result: labelled, targetIdx: analysis.targetIdx, compIdx: analysis.compIdx };
  }, [result, analysis, referenceTargets]);

  const docs = useMemo((): LightCurveDoc[] => {
    if (!exportSource || !analysis) return [];
    const { result: labelled, targetIdx, compIdx } = exportSource;
    const label = (i: number) => labelled.targets[i].label;
    return [
      ...(targetIdx >= 0 ? [lightCurveDocFromFrames(labelled, targetIdx, compIdx, timeAxis)] : []),
      ...analysis.checkIdx.map((i) =>
        lightCurveDocFromFrames(labelled, i, compIdx, timeAxis, { id: `check:${i}`, label: `check star ${label(i)}` }),
      ),
      ...(compIdx.length >= 2
        ? compIdx.map((i) => lightCurveDocFromFrames(labelled, i, compIdx, timeAxis, { id: `comp:${i}`, label: `comp ${label(i)} vs others` }))
        : []),
      ...compIdx.map((i) => rawFluxDocFromFrames(labelled, i, timeAxis, { overlay: RAW_COMPS_OVERLAY })),
    ];
  }, [exportSource, analysis, timeAxis]);

  const group = result ? frameGroupKey(result.reference_path) : null;

  useEffect(() => {
    if (group) lightCurveStore.put(group, docs);
  }, [group, docs]);

  const view = useLightCurveView(group);
  const targetModel = useMemo(() => {
    const target = docs.find((doc) => doc.id === "target");
    return target ? lightCurveModel(target, view) : null;
  }, [docs, view]);

  const exportedSource = useMemo(
    () => exportSource && { ...exportSource, leftOut: targetModel ? leftOutReasons(targetModel.state) : undefined },
    [exportSource, targetModel],
  );
  const leftOutNote = exportedSource ? leftOutSummary(exportedSource.leftOut ?? []) : null;

  const earlierGroup = !result && filePath ? frameGroupKey(filePath) : null;
  const stored = useLightCurves(earlierGroup);

  const activeTable = useActiveLightCurveTable();
  const openTables = useLightCurveGroups("table:");
  const tableRoute = useTableRoute("light_curve");

  useEffect(() => {
    if (tableRoute === null) return;
    void openLightCurveTable(tableRoute.path);
    tableRouteStore.clear("light_curve");
  }, [tableRoute]);

  const saturated = useMemo(() => (exportSource ? saturatedComps(exportSource.result) : []), [exportSource]);
  const notes = useMemo(() => (result ? [...geometryNotes(result), ...otherFrameNotes(result)] : []), [result]);

  const framesUnit = analysis?.units.mixed === null ? analysis.units.unit : null;
  const csvUnit = framesUnit ?? fluxUnit;

  const csvText = useCallback((): string => {
    if (!exportedSource || exportedSource.targetIdx < 0) return "";
    return lightCurveCsvText(exportedSource.result, exportedSource.targetIdx, exportedSource.compIdx, csvUnit, {
      exportedUtc: new Date().toISOString(),
      leftOut: exportedSource.leftOut,
    });
  }, [exportedSource, csvUnit]);

  const copyText = useCallback(() => {
    const text = csvText();
    return text === "" ? null : text;
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

  const etdBlock = useMemo(() => (exportedSource ? etdBlocker(exportedSource) : null), [exportedSource]);
  const exoClockBlock = useMemo(() => (exportedSource ? exoClockBlocker(exportedSource, exoClockTime) : null), [exportedSource, exoClockTime]);
  const exportNotes = useMemo(() => exportBlockerNotes(etdBlock, exoClockBlock), [etdBlock, exoClockBlock]);

  const saveEtd = useCallback(async () => {
    if (!exportedSource || etdBlock) return;
    await saveText(
      { defaultPath: etdFileName(exportedSource), filters: [{ name: "ETD text", extensions: ["txt"] }], title: "Save ETD light curve" },
      () => etdText(exportedSource),
    );
  }, [exportedSource, etdBlock, saveText]);

  const saveExoClock = useCallback(async () => {
    if (!exportedSource || exoClockBlock) return;
    await saveText(
      { defaultPath: exoClockFileName(exportedSource, exoClockTime), filters: [{ name: "CSV", extensions: ["csv"] }], title: "Save ExoClock light curve" },
      () => exoClockCsv(exportedSource, exoClockTime),
    );
  }, [exportedSource, exoClockBlock, exoClockTime, saveText]);

  const axes = useMemo(() => (result ? timeAxisAvailability(result) : { jd: false, bjd: false, any: false }), [result]);
  const shownFrames = result ? result.frames.slice(0, TABLE_ROW_LIMIT) : [];
  const targetLabel = analysis && analysis.targetIdx >= 0 ? result?.targets[analysis.targetIdx].label : null;
  const geometryTarget = result?.geometry_target ?? null;
  const lightTimeTarget = geometryTarget
    ? `light-time target RA ${geometryTarget.ra_deg.toFixed(6)} Dec ${geometryTarget.dec_deg.toFixed(6)} deg (${geometryTarget.source})`
    : null;
  const measuredBjdSources = result ? bjdSources(result.frames.filter((f) => f.skipped === null)) : null;
  const bjdAccuracy =
    analysis?.axis === "bjd" && measuredBjdSources?.split(", ").includes("computed") ? `BJD_TDB: ${BJD_ACCURACY_NOTE}` : undefined;

  if (activeTable) return <LightCurveTablePanel />;

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
        <PhotometryStatusLine filePath={starsPath ?? null} measureKey={null} excludeDq={excludeDq} />
        {openTables.length > 0 && (
          <div className="flex flex-wrap items-center gap-1 text-[9px] text-zinc-500" data-open-tables>
            <span>Saved light curves:</span>
            {openTables.map((group) => (
              <button
                key={group}
                type="button"
                title={tablePathOf(group)}
                onClick={() => void lightCurveTableLoader.activate(group)}
                className={SMALL_BUTTON_CLASS}
              >
                {fileBaseName(tablePathOf(group))}
              </button>
            ))}
          </div>
        )}
        {runReference ? (
          <div className="flex items-center justify-between gap-1.5">
            <div className={`${SECTION_CLASS} truncate`}>{`Stars of ${fileBaseName(runReference)}`}</div>
            <button type="button" onClick={showReference} className={SMALL_BUTTON_CLASS}>
              Show reference frame
            </button>
          </div>
        ) : (
          <div className={SECTION_CLASS}>Stars</div>
        )}
        {targets.length === 0 ? (
          <div className="text-[10px] text-zinc-600">{STARS_HINT}</div>
        ) : (
          <div className="space-y-0.5">
            {targets.map(({ id, region, target }) => (
              <div
                key={id}
                data-selected-star={id === regionDoc.selectedId || undefined}
                className={`grid grid-cols-[1fr_auto_auto] items-center gap-1.5 text-[10px] font-mono${id === regionDoc.selectedId ? " bg-zinc-800/60 rounded" : ""}`}
              >
                <input
                  key={region.props.text ?? ""}
                  type="text"
                  defaultValue={region.props.text ?? ""}
                  placeholder={target.label}
                  aria-label={`Name of ${target.label}`}
                  title={STAR_NAME_TITLE}
                  onBlur={(e) => nameStar(region, e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                  }}
                  className={NAME_CLASS}
                />
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
            {ignoredByDefault > 0 && (
              <div className="text-[9px] text-zinc-500" data-ignored-note>
                {`${ignoredByDefault} ${ignoredByDefault === 1 ? "star" : "stars"} beyond the limit of ${MAX_TIME_SERIES_TARGETS} start as ignore; change a role to measure one.`}
              </div>
            )}
          </div>
        )}

        <div className={SECTION_CLASS}>Frames</div>
        <div className="text-[10px] text-zinc-400 font-mono">
          {frames.length} matching {frames.length === 1 ? "frame" : "frames"}
          {dims ? ` at ${dims[0]} x ${dims[1]}` : ""}
        </div>
        {matching.leftOutLine && <div className="text-[9px] text-zinc-500">{matching.leftOutLine}</div>}
        {frames.length < 2 && <div className="text-[10px] text-zinc-600">{FRAMES_HINT}</div>}
        {!referenceLoaded && !!filePath && (
          <div className="text-[9px] text-amber-400/90">
            {referenceLeftOut ? REFERENCE_LEFT_OUT_HINT : "The current file is not a done frame, so it cannot be the reference."}
          </div>
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
        <div className="grid grid-cols-2 gap-1.5">
          <div className="flex flex-col gap-0.5">
            <label htmlFor={targetRaId} className="text-[9px] text-zinc-500">
              target RA deg
            </label>
            <DecimalField
              id={targetRaId}
              {...TARGET_FIELD}
              value={targetRaText}
              placeholder={TARGET_PLACEHOLDER}
              onChange={setTargetRaText}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={targetDecId} className="text-[9px] text-zinc-500">
              target Dec deg
            </label>
            <DecimalField
              id={targetDecId}
              {...TARGET_FIELD}
              value={targetDecText}
              placeholder={TARGET_PLACEHOLDER}
              onChange={setTargetDecText}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        <div className="text-[9px] text-zinc-600">{LIGHT_TIME_TARGET_CAPTION}</div>
        {annulusHalfFilled && <div className="text-[9px] text-amber-400/90">{ANNULUS_NEEDS_BOTH}</div>}
        {apertureHint && <div className="text-[9px] text-amber-400/90">{apertureHint}</div>}
        {inputError && (
          <div className="text-[9px] text-amber-400/90" data-field-error>
            {inputError}
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
          <div
            role="progressbar"
            aria-label="Time series progress"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(progress.percent)}
            className="w-full h-1 bg-zinc-800 rounded-full overflow-hidden"
          >
            <div className="h-full rounded-full bg-amber-400/80 transition-all duration-300" style={{ width: `${progress.percent}%` }} />
          </div>
        )}
        <p role="status" className="sr-only" data-series-status>
          {seriesStatus}
        </p>

        <ErrorAlert message={error} />
        <WarningList warnings={result?.warnings} />
        {notes.length > 0 && (
          <details className="text-[9px] text-zinc-500">
            <summary className="cursor-pointer select-none">Geometry notes</summary>
            <ul className="mt-1 space-y-0.5 list-disc pl-4">
              {notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </details>
        )}

        {earlierGroup && stored.length > 0 && (
          <>
            <div className={SECTION_CLASS}>Light curve</div>
            <div className="text-[9px] text-zinc-500" data-earlier-curve>
              {EARLIER_CURVE_NOTE}
            </div>
            <LightCurveView group={earlierGroup} csvName="lightcurve" />
          </>
        )}

        {result && analysis && group && (
          <>
            <div className={SECTION_CLASS}>Light curve</div>
            <div className="grid grid-cols-2 gap-1.5">
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
                {!axes.bjd && <div className="text-[9px] text-zinc-600">{bjdAxisHint(result)}</div>}
              </div>
            </div>

            {saturated.map((comp) => {
              const out = leftOut.includes(comp.index);
              return (
                <div key={comp.index} className={`flex items-center gap-1.5 text-[9px] ${out ? "text-zinc-500" : "text-amber-400/90"}`}>
                  <span className="flex-1">{saturatedCompText(comp, out)}</span>
                  <button type="button" onClick={() => toggleLeftOut(comp.index)} className={SMALL_BUTTON_CLASS}>
                    {out ? `Put ${comp.label} back` : `Leave ${comp.label} out`}
                  </button>
                </div>
              );
            })}

            <LightCurveView
              group={group}
              onSelectPoint={showPoint}
              selectedPointRef={currentFrameIndex >= 0 ? currentFrameIndex : null}
              csvName="lightcurve"
            />

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
              <div className="bg-zinc-900/80 rounded px-2 py-1.5" title={bjdAccuracy} data-bjd-accuracy={bjdAccuracy !== undefined || undefined}>
                <div className="text-zinc-500">{analysis.axis === "bjd" ? "BJD0 (TDB)" : "JD0"}</div>
                <div className="text-zinc-300 font-mono">{analysis.axis !== "index" ? analysis.jd0 : "no frame time"}</div>
                {analysis.timeSource && (
                  <div className="text-[9px] text-zinc-500 truncate" title={analysis.timeSource}>
                    {analysis.timeSource}
                  </div>
                )}
                {lightTimeTarget && (
                  <div className="text-[9px] text-zinc-500 truncate" title={lightTimeTarget} data-light-time-target>
                    {lightTimeTarget}
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
                    <th className="text-right px-1.5 py-0.5">{analysis.headers.jd}</th>
                    <th className="text-right px-1.5 py-0.5">{analysis.headers.bjd}</th>
                    <th className="text-right px-1.5 py-0.5" title={AIRMASS_COLUMN_TITLE}>
                      airmass
                    </th>
                    <th className="text-right px-1.5 py-0.5">{analysis.headers.diffMag}</th>
                    <th className="text-right px-1.5 py-0.5">SNR</th>
                    <th className="text-right px-1.5 py-0.5">{analysis.headers.fwhm}</th>
                    <th className="text-right px-1.5 py-0.5">{analysis.headers.sky}</th>
                    <th className="text-right px-1.5 py-0.5">{analysis.headers.offset}</th>
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
                    const saturatedComp = saturatedCompLabels(frame, result.targets, analysis.compIdx);
                    const pointState = targetModel?.state[frame.index];
                    const leftOutAs = pointState !== undefined && LEFT_OUT_STATES.includes(pointState) ? pointState : undefined;
                    const tone = frame.skipped || leftOutAs ? "text-zinc-500" : isCurrent ? "text-amber-200" : "text-zinc-300";
                    return (
                      <tr key={frame.index} className={`${tone} ${isCurrent ? "bg-zinc-800/60" : ""}`} data-left-out={leftOutAs}>
                        <td className="text-right px-1.5 py-0.5">
                          <button
                            type="button"
                            onClick={() => showFrameAt(frame.index)}
                            aria-label={`Show frame ${frame.index}`}
                            title={SHOW_FRAME_TITLE}
                            className="hover:underline"
                          >
                            {frame.index}
                          </button>
                        </td>
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
                          {saturatedComp.length > 0 && (
                            <span className={`${CHIP_CLASS} bg-red-900/50 text-red-300`}>{`comp ${saturatedComp.join(", ")} saturated`}</span>
                          )}
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
                          {leftOutAs === "manual" && <span className={`${CHIP_CLASS} bg-amber-900/50 text-amber-300`}>left out by hand</span>}
                          {leftOutAs === "clipped" && <span className={`${CHIP_CLASS} bg-amber-900/50 text-amber-300`}>clipped</span>}
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
            {analysis.units.mixed && (
              <div className="text-[9px] text-amber-400/90" data-unit-note>
                {analysis.units.mixed}
              </div>
            )}

            <div className="flex flex-wrap items-center gap-1.5">
              <CopyCsvButton text={copyText} disabled={analysis.targetIdx < 0} onError={setError} />
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
            {leftOutNote && (
              <div className="text-[9px] text-zinc-500" data-left-out-note>
                {`ETD and ExoClock leave out ${leftOutNote}`}
              </div>
            )}
            {exportNotes.map((note) => (
              <div key={note.key} className="text-[9px] text-zinc-500" data-export-blocker={note.key}>
                {note.text}
              </div>
            ))}
            {savedPath && (
              <div role="status" className="text-[9px] text-emerald-400/90 break-all">
                Saved {savedPath}
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default memo(TimeSeriesPanel);
