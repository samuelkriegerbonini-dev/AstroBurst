import { useState, useCallback, useEffect, useId, useMemo } from "react";
import { X } from "lucide-react";
import { backgroundOptionsOf, recordExtractBackground, type BackgroundSettings, type DebandModel } from "../../services/processing";
import { recordBackgroundDbe } from "../../services/dbe";
import { useHistoryScope } from "../../hooks/useHistory";
import { useRerunInitial, useRerunUpdates } from "../../hooks/useRerunParams";
import type { HistoryScope } from "../../shared/types/ops";
import { compositeBackground } from "../../services/compositeChain";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { bustPreviewUrl, isCancelMessage, useProcessingRun } from "../../hooks/useProcessingRun";
import { useCompositeChain } from "../../hooks/useCompositeChain";
import { BACKGROUND_PROGRESS_EVENT } from "../../shared/types/processing";
import { Slider, Toggle, RunButton, ResultGrid, ChainBanner, ErrorAlert, SectionHeader, CompareView } from "../ui";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useRenderContext } from "../../context/PreviewContext";
import { useCompositeStf } from "../../context/CompositeContext";
import { chainHoldsOutput } from "../../utils/processingChain";
import { COMPOSITE_RUN_KEY, compositeChainHolds } from "../../utils/compositeChain";
import {
  DBE_DEFAULT_AUTO_GRID,
  DBE_MIN_AUTO_GRID,
  DEFAULT_DBE_CONFIG,
  dbeRunBlock,
  pointSamplesFromDoc,
  pointSamplesSent,
  validateDbeParams,
  type ImageSize,
} from "../../utils/dbeSamples";
import { backgroundModelLabel } from "../../utils/historyModel";
import { COMPOSITE_RESTARTED_NOTICE, channelTriple, compositeModeNotice, describedByIds } from "./compositeProps";
import type { CompositeBackgroundResult, CompositePanelProps } from "./compositeProps";
import { backgroundControlsFor, withinRange, type BackgroundModel, type BackgroundOperation } from "./backgroundControls";
import type { ProcessedFile } from "../../shared/types";
import type { DbeConfig, DbeMode, DbeSample } from "../../shared/types/dbe";
import type { BackgroundResult as BackgroundCommandResult } from "../../shared/types/processing";

const OPERATIONS: { value: BackgroundOperation; label: string }[] = [
  { value: "gradient", label: "Remove gradient" },
  { value: "deband", label: "De-band" },
];
const DEBAND_AXES: DebandModel[] = ["deband_auto", "deband_rows", "deband_cols", "deband_both"];
const DEBAND_AXIS_LABELS: Record<DebandModel, { choice: string; run: string }> = {
  deband_auto: { choice: "Auto (detect)", run: "De-band (auto axis)" },
  deband_rows: { choice: "Rows", run: "De-band (rows)" },
  deband_cols: { choice: "Columns", run: "De-band (columns)" },
  deband_both: { choice: "Both", run: "De-band (both axes)" },
};
const DEFAULT_DEBAND_AXIS: DebandModel = "deband_auto";
const APPLY_AS: { value: DbeMode; label: string; use: string }[] = [
  { value: "subtract", label: "Subtract", use: "additive: sky glow, light pollution" },
  { value: "divide", label: "Divide", use: "multiplicative: vignetting; needs a positive model" },
];
const DEBAND_COMPOSITE_REASON = "De-banding works on a single file, not on the composite";
const DEBAND_SPLINE_REASON = "De-banding runs with the Polynomial model: set Model to Polynomial to de-band";

const RESULT_LABEL = "Background Removed";
const DEBAND_RESULT_LABEL = "De-banded";
const MODEL_LABEL = "Background Model";
const BAND_PATTERN_LABEL = "Band Pattern";
const POINT_REGIONS_COMPOSITE_REASON = "Point regions belong to the file's pixel grid, not the composite";

interface BackgroundResult {
  previewUrl?: string;
  modelUrl?: string;
  corrected_fits?: string;
  model_fits?: string;
  sample_count?: number;
  rejected_count?: number;
  rms_residual?: number;
  elapsed_ms?: number;
  dimensions?: [number, number];
  samples?: DbeSample[];
  axis?: BackgroundCommandResult["axis"];
}

interface BackgroundParams {
  gridSize: number;
  polyDegree: number;
  sigmaClip: number;
  iterations: number;
  mode: DbeMode;
}

interface BackgroundFileRun {
  composite: false;
  res: BackgroundResult;
  correctedUrl: string | undefined;
  modelUrl: string | undefined;
  inputUrl: string | null;
  inputLabel: string;
  spline: boolean;
  sampleRadius: number;
}

interface BackgroundCompositeRun {
  composite: true;
  res: CompositeBackgroundResult;
  correctedUrl: string | undefined;
  modelUrl: string | undefined;
  inputUrl: string | null;
  inputLabel: string;
  spline: boolean;
  sampleRadius: number;
}

type BackgroundRun = BackgroundFileRun | BackgroundCompositeRun;

type ResultView = "corrected" | "model" | "compare";

const RESULT_VIEW_LABELS: Record<ResultView, string> = {
  corrected: "Corrected",
  model: "Model",
  compare: "Compare",
};

interface BackgroundPanelProps extends CompositePanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onProcessingDone?: (result: BackgroundResult) => void | Promise<void>;
  chainedFrom?: string | null;
  inputLabel?: string;
  fileKey?: string | null;
  disabledReason?: string | null;
  disabledReasonId?: string;
}

const ICON = (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-emerald-400">
    <rect x="3" y="3" width="18" height="18" rx="2" />
    <path d="M3 15h18M3 9h18" opacity="0.3" />
    <path d="M9 3v18M15 3v18" opacity="0.3" />
  </svg>
);

const SAMPLE_STROKE = { accepted: "#34d399", manual: "#fbbf24", rejected: "#f87171" };

function sampleStroke(sample: DbeSample): string {
  if (sample.rejected) return SAMPLE_STROKE.rejected;
  return sample.manual ? SAMPLE_STROKE.manual : SAMPLE_STROKE.accepted;
}

const DEFAULT_PARAMS: BackgroundParams = { gridSize: 8, polyDegree: 3, sigmaClip: 2.5, iterations: 3, mode: "subtract" };

function debandAxisOf(settings: BackgroundSettings): DebandModel | null {
  if (settings.model === "spline") return null;
  const { mode } = settings.params;
  return mode === "subtract" || mode === "divide" ? null : mode;
}

function paramsAfter(prev: BackgroundParams, settings: BackgroundSettings): BackgroundParams {
  if (settings.model === "spline") return { ...prev, mode: settings.dbe.mode };
  const { mode, ...rest } = settings.params;
  return { ...prev, ...rest, mode: mode === "subtract" || mode === "divide" ? mode : prev.mode };
}

function formatCount(v: number): string {
  return String(v);
}

function formatRms(v: number): string {
  return v.toExponential(2);
}

function formatSeconds(ms: number): string {
  return `${(ms / 1000).toFixed(1)}s`;
}

function imageSizeOf(file: ProcessedFile | null): ImageSize | null {
  const dims = file?.result?.dimensions;
  return dims ? { width: dims[0], height: dims[1] } : null;
}

export default function BackgroundPanel({
  selectedFile,
  outputDir = "./output",
  onProcessingDone,
  chainedFrom,
  inputLabel,
  fileKey,
  compositeMode,
  compositeInput,
  onCompositeDone,
  fileName,
  disabledReason,
  disabledReasonId,
  runNoteId,
  onRunLabelChange,
}: BackgroundPanelProps) {
  const progress = useProgress(BACKGROUND_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const initial = useRerunInitial("background", backgroundOptionsOf);
  const initialAxis = initial ? debandAxisOf(initial.params) : null;
  const [model, setModel] = useState<BackgroundModel>(initial?.params.model ?? "polynomial");
  const [operation, setOperation] = useState<BackgroundOperation>(initialAxis ? "deband" : "gradient");
  const [axis, setAxis] = useState<DebandModel>(initialAxis ?? DEFAULT_DEBAND_AXIS);
  const [params, setParams] = useState<BackgroundParams>(() => (initial ? paramsAfter(DEFAULT_PARAMS, initial.params) : DEFAULT_PARAMS));
  const [dbe, setDbe] = useState<DbeConfig>(initial?.params.model === "spline" ? initial.params.dbe : DEFAULT_DBE_CONFIG);
  const applyRerun = useCallback((settings: BackgroundSettings) => {
    const rerunAxis = debandAxisOf(settings);
    setModel(settings.model);
    setOperation(rerunAxis ? "deband" : "gradient");
    if (rerunAxis) setAxis(rerunAxis);
    setParams((prev) => paramsAfter(prev, settings));
    if (settings.model === "spline") setDbe(settings.dbe);
  }, []);
  useRerunUpdates("background", initial, backgroundOptionsOf, applyRerun);
  const scope = useHistoryScope();
  const [usePointRegions, setUsePointRegions] = useState(true);
  const [lastAutoGrid, setLastAutoGrid] = useState(DBE_DEFAULT_AUTO_GRID);
  const { running: isRunning, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<BackgroundRun>("background", compositeMode ? COMPOSITE_RUN_KEY : fileKey ?? null);
  const { chain } = useRenderContext();
  const compositeChain = useCompositeChain();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const displayStf = useMemo(
    () => ({ r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked }),
    [compositeStfR, compositeStfG, compositeStfB, compositeStfLinked],
  );
  const held = runResult
    ? runResult.composite
      ? compositeChainHolds(compositeChain, "background", runResult.correctedUrl)
      : chainHoldsOutput(chain, "background", runResult.res.corrected_fits)
    : false;
  const result = held ? runResult : null;
  const canRun = compositeMode ? compositeInput !== null : !!selectedFile && !!scope;
  const [view, setView] = useState<ResultView>("corrected");
  const [showSamples, setShowSamples] = useState(true);

  const modelId = useId();
  const operationId = useId();
  const axisId = useId();
  const applyAsId = useId();
  const dbeNoteId = useId();
  const pointRegionsNoteId = useId();

  const regionKey = useRegionKey();
  const regionDoc = useRegionDoc(regionKey);
  const pointSamples = useMemo(() => pointSamplesFromDoc(regionDoc), [regionDoc]);

  const update = useCallback(<K extends keyof BackgroundParams>(key: K, value: BackgroundParams[K]) => {
    setParams((prev) => ({ ...prev, [key]: value }));
  }, []);

  const updateDbe = useCallback(<K extends keyof DbeConfig>(key: K, value: DbeConfig[K]) => {
    setDbe((prev) => ({ ...prev, [key]: value }));
  }, []);

  const setAutoGridOn = useCallback((on: boolean) => {
    if (!on && dbe.autoGrid !== null) setLastAutoGrid(dbe.autoGrid);
    updateDbe("autoGrid", on ? lastAutoGrid : null);
  }, [dbe.autoGrid, lastAutoGrid, updateDbe]);

  const chooseApplyAs = useCallback((mode: DbeMode) => {
    setParams((prev) => ({ ...prev, mode }));
    setDbe((prev) => ({ ...prev, mode }));
  }, []);

  const isSpline = model === "spline";
  const debandBlockedReason = compositeMode ? DEBAND_COMPOSITE_REASON : isSpline ? DEBAND_SPLINE_REASON : null;
  const activeOperation: BackgroundOperation = debandBlockedReason ? "gradient" : operation;
  const debanding = activeOperation === "deband";
  const controls = backgroundControlsFor(activeOperation, model);
  const iterations = controls.iterations ? withinRange(params.iterations, controls.iterations) : params.iterations;
  const applyAs: DbeMode = isSpline ? dbe.mode : params.mode;
  const runLabel = backgroundModelLabel(debanding ? axis : model);
  useEffect(() => {
    onRunLabelChange?.(runLabel);
  }, [runLabel, onRunLabelChange]);
  const dbeBlock = isSpline
    ? dbeRunBlock({ ...dbe, manualSamples: pointSamples }, usePointRegions, compositeMode, compositeMode ? null : imageSizeOf(selectedFile))
    : null;

  const runPolynomial = useCallback(async (runScope: HistoryScope, path: string): Promise<BackgroundResult> => {
    return recordExtractBackground(runScope, path, {
      gridSize: params.gridSize,
      polyDegree: params.polyDegree,
      sigmaClip: params.sigmaClip,
      iterations,
      mode: params.mode,
    });
  }, [params, iterations]);

  const runDeband = useCallback(async (runScope: HistoryScope, path: string): Promise<BackgroundResult> => {
    return recordExtractBackground(runScope, path, { sigmaClip: params.sigmaClip, iterations, mode: axis });
  }, [params.sigmaClip, iterations, axis]);

  const runSpline = useCallback(async (runScope: HistoryScope, path: string): Promise<BackgroundResult> => {
    const config: DbeConfig = { ...dbe, manualSamples: pointSamplesSent(pointSamples, usePointRegions, false) };
    const validation = validateDbeParams(config, imageSizeOf(selectedFile));
    if (!validation.ok) throw new Error(validation.errors.join("; "));
    return recordBackgroundDbe(runScope, path, config);
  }, [dbe, usePointRegions, pointSamples, selectedFile]);

  const handleFileRun = useCallback(() => {
    if (!selectedFile?.path || !scope) return;
    const path = selectedFile.path;
    const runScope = scope;
    const inputUrl = selectedFile.result?.previewUrl ?? null;
    const inputLabelAtRun = inputLabel ?? "Original";
    const spline = model === "spline";
    const sampleRadius = dbe.sampleRadius;
    const runOn = debanding ? runDeband : spline ? runSpline : runPolynomial;
    resetProgress();
    void run(async () => {
      const res = await runOn(runScope, path);
      await onProcessingDone?.(res);
      const stamp = Date.now();
      return {
        composite: false as const,
        res,
        correctedUrl: bustPreviewUrl(res?.previewUrl, stamp),
        modelUrl: bustPreviewUrl(res?.modelUrl, stamp),
        inputUrl,
        inputLabel: inputLabelAtRun,
        spline,
        sampleRadius,
      };
    }, isCancelMessage).finally(resetProgress);
  }, [selectedFile, scope, inputLabel, model, dbe.sampleRadius, debanding, runDeband, runSpline, runPolynomial, resetProgress, run, onProcessingDone]);

  const handleCompositeRun = useCallback(() => {
    if (!compositeInput) return;
    const chainCall = { chainInput: compositeInput.input, displayStf };
    const inputUrl = compositeInput.previewUrl;
    const inputLabelAtRun = compositeInput.label;
    const spline = model === "spline";
    const splineConfig: DbeConfig = { ...dbe, manualSamples: [] };
    const sampleRadius = dbe.sampleRadius;
    resetProgress();
    void run(async () => {
      if (spline) {
        const validation = validateDbeParams(splineConfig, null);
        if (!validation.ok) throw new Error(validation.errors.join("; "));
      }
      const res = await compositeBackground(outputDir, chainCall, {
        model,
        gridSize: params.gridSize,
        polyDegree: params.polyDegree,
        sigmaClip: params.sigmaClip,
        iterations,
        mode: applyAs,
        dbe: spline ? splineConfig : null,
      });
      onCompositeDone("background", spline ? backgroundModelLabel("spline") : RESULT_LABEL, res, chainCall.displayStf);
      const stamp = Date.now();
      return {
        composite: true as const,
        res,
        correctedUrl: bustPreviewUrl(res.previewUrl, stamp),
        modelUrl: bustPreviewUrl(res.modelPreviewUrl, stamp),
        inputUrl: res.basePreviewUrl ?? inputUrl,
        inputLabel: res.chain_restarted ? "Composite" : inputLabelAtRun,
        spline,
        sampleRadius,
      };
    }, isCancelMessage).finally(resetProgress);
  }, [compositeInput, displayStf, model, dbe, params, iterations, applyAs, outputDir, resetProgress, run, onCompositeDone]);

  const handleRun = compositeMode ? handleCompositeRun : handleFileRun;

  const splineSamples = result && !result.composite ? result.res.samples : undefined;
  const overlayDims = result?.res.dimensions;
  const overlayRadius = result?.sampleRadius ?? 0;
  const canOverlay = !!result?.spline && !!splineSamples && !!overlayDims && overlayDims[0] > 0 && overlayDims[1] > 0;
  const canCompare = !!result?.inputUrl && !!result.correctedUrl;
  const views: ResultView[] = canCompare ? ["corrected", "model", "compare"] : ["corrected", "model"];
  const shownView: ResultView = view === "compare" && !canCompare ? "corrected" : view;
  const showModel = shownView === "model";
  const resultItems = result?.composite
    ? [
        { label: "Samples", value: channelTriple(result.res.sample_count, formatCount) },
        ...(result.res.rejected_count ? [{ label: "Rejected", value: channelTriple(result.res.rejected_count, formatCount) }] : []),
        { label: "RMS", value: channelTriple(result.res.rms_residual, formatRms) },
        { label: "Time", value: formatSeconds(result.res.elapsed_ms) },
      ]
    : [
        { label: "Samples", value: result?.res.sample_count },
        ...(result?.res.rejected_count !== undefined ? [{ label: "Rejected", value: result.res.rejected_count }] : []),
        { label: "RMS", value: result?.res.rms_residual?.toExponential(2) },
        { label: "Time", value: formatSeconds(result?.res.elapsed_ms ?? 0) },
      ];
  const debandAxis = result && !result.composite ? result.res.axis ?? null : null;
  const resultLabel = debandAxis ? DEBAND_RESULT_LABEL : RESULT_LABEL;
  const modelLabel = debandAxis ? BAND_PATTERN_LABEL : MODEL_LABEL;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Background Extraction" />
      <ChainBanner chainedFrom={chainedFrom} accent="emerald" />

      {!compositeMode && !selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to enable background extraction.</div>
      )}
      {compositeMode && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          {compositeModeNotice(fileName)}
        </div>
      )}

      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <label htmlFor={modelId} className="text-xs text-zinc-400">Model</label>
          <select id={modelId} data-testid="background-model" value={model} onChange={(e) => setModel(e.target.value as BackgroundModel)} disabled={isRunning} className="ab-select">
            <option value="polynomial">Polynomial</option>
            <option value="spline">Spline (DBE)</option>
          </select>
        </div>

        <div className="flex flex-col gap-1.5">
          <span id={`${operationId}-label`} className="text-xs text-zinc-400">Operation</span>
          <div
            role="radiogroup"
            aria-labelledby={`${operationId}-label`}
            aria-describedby={debandBlockedReason ? `${operationId}-note` : undefined}
            data-testid="background-operation"
            className="flex items-center gap-4"
          >
            {OPERATIONS.map((option) => {
              const unavailable = option.value === "deband" && !!debandBlockedReason;
              return (
                <label key={option.value} className={`flex items-center gap-1.5 text-xs ${unavailable ? "text-zinc-600" : "text-zinc-300"}`}>
                  <input
                    type="radio"
                    name={operationId}
                    value={option.value}
                    checked={activeOperation === option.value}
                    disabled={isRunning || unavailable}
                    data-testid={`background-operation-${option.value}`}
                    onChange={() => setOperation(option.value)}
                    className="accent-emerald-400"
                  />
                  {option.label}
                </label>
              );
            })}
          </div>
          <div aria-live="polite">
            {debandBlockedReason && (
              <div id={`${operationId}-note`} role="note" className="text-[10px] text-zinc-500 px-0.5">{debandBlockedReason}</div>
            )}
          </div>
        </div>

        {controls.axis && (
          <div className="flex flex-col gap-1.5">
            <span id={`${axisId}-label`} className="text-xs text-zinc-400">Axis</span>
            <div role="radiogroup" aria-labelledby={`${axisId}-label`} data-testid="background-axis" className="flex flex-wrap items-center gap-x-4 gap-y-1">
              {DEBAND_AXES.map((value) => (
                <label key={value} className="flex items-center gap-1.5 text-xs text-zinc-300">
                  <input
                    type="radio"
                    name={axisId}
                    value={value}
                    checked={axis === value}
                    disabled={isRunning}
                    data-testid={`background-axis-${value}`}
                    onChange={() => setAxis(value)}
                    className="accent-emerald-400"
                  />
                  {DEBAND_AXIS_LABELS[value].choice}
                </label>
              ))}
            </div>
          </div>
        )}

        {controls.gridSize && <Slider label="Grid Size" value={params.gridSize} {...controls.gridSize} disabled={isRunning} accent="emerald" onChange={(v) => update("gridSize", v)} />}
        {controls.polyDegree && <Slider label="Polynomial Degree" value={params.polyDegree} {...controls.polyDegree} disabled={isRunning} accent="emerald" onChange={(v) => update("polyDegree", v)} />}
        {controls.sigmaClip && <Slider label="Sigma Clip" value={params.sigmaClip} {...controls.sigmaClip} disabled={isRunning} accent="emerald" format={(v) => v.toFixed(1)} onChange={(v) => update("sigmaClip", v)} />}
        {controls.iterations && <Slider label="Iterations" value={iterations} {...controls.iterations} disabled={isRunning} accent="emerald" onChange={(v) => update("iterations", v)} />}

        {controls.dbe && (
          <>
            <Slider label="Sample Radius (px)" value={dbe.sampleRadius} min={1} max={32} step={1} disabled={isRunning} accent="emerald" onChange={(v) => updateDbe("sampleRadius", Math.round(v))} />
            <Slider label="Tolerance (sigma)" value={dbe.tolerance} min={0.1} max={5} step={0.1} disabled={isRunning} accent="emerald" format={(v) => v.toFixed(1)} onChange={(v) => updateDbe("tolerance", v)} />
            <Slider label="Smoothing" value={dbe.smoothing} min={0} max={1} step={0.01} disabled={isRunning} accent="emerald" format={(v) => v.toFixed(2)} onChange={(v) => updateDbe("smoothing", v)} />
            <Toggle label="Auto Grid" checked={dbe.autoGrid !== null} disabled={isRunning} accent="emerald" onChange={setAutoGridOn} />
            {dbe.autoGrid !== null && (
              <Slider
                label="Auto Grid (per axis)"
                value={dbe.autoGrid}
                min={DBE_MIN_AUTO_GRID}
                max={30}
                step={1}
                disabled={isRunning}
                accent="emerald"
                onChange={(v) => updateDbe("autoGrid", Math.round(v))}
              />
            )}
            <div>
              <Toggle
                label="Use point regions as samples"
                checked={compositeMode ? false : usePointRegions}
                disabled={isRunning || compositeMode}
                accent="emerald"
                badge={!compositeMode && pointSamples.length > 0 ? `${pointSamples.length} pts` : null}
                describedBy={compositeMode ? pointRegionsNoteId : undefined}
                onChange={setUsePointRegions}
              />
              {compositeMode && (
                <div id={pointRegionsNoteId} role="note" className="text-[10px] text-zinc-500 px-0.5">{POINT_REGIONS_COMPOSITE_REASON}</div>
              )}
              <div aria-live="polite">
                {dbeBlock && (
                  <p id={dbeNoteId} role="note" className="mt-1.5 text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded-lg px-3 py-1.5">
                    {dbeBlock}
                  </p>
                )}
              </div>
            </div>
            <Toggle label="Reject stars" checked={dbe.rejectStars} disabled={isRunning} accent="emerald" onChange={(v) => updateDbe("rejectStars", v)} />
            <Toggle label="Keep image median" checked={dbe.normalize} disabled={isRunning} accent="emerald" onChange={(v) => updateDbe("normalize", v)} />
          </>
        )}

        {controls.applyAs && (
          <div className="flex flex-col gap-1.5">
            <span id={`${applyAsId}-label`} className="text-xs text-zinc-400">Apply as</span>
            <div role="radiogroup" aria-labelledby={`${applyAsId}-label`} data-testid="background-apply-as" className="flex flex-col gap-1">
              {APPLY_AS.map((option) => (
                <label key={option.value} className="flex items-start gap-1.5 text-xs text-zinc-300">
                  <input
                    type="radio"
                    name={applyAsId}
                    value={option.value}
                    checked={applyAs === option.value}
                    disabled={isRunning}
                    data-testid={`background-apply-as-${option.value}`}
                    onChange={() => chooseApplyAs(option.value)}
                    className="accent-emerald-400 mt-0.5"
                  />
                  <span>
                    {option.label} <span className="text-zinc-500">({option.use})</span>
                  </span>
                </label>
              ))}
            </div>
          </div>
        )}
      </div>

      <div title={disabledReason ?? dbeBlock ?? busyTitle}>
        <RunButton
          label={debanding ? DEBAND_AXIS_LABELS[axis].run : isSpline ? "Extract Background (Spline)" : "Extract Background"}
          runningLabel={debanding ? "De-banding..." : "Extracting..."}
          running={isRunning}
          disabled={!canRun || blocked || !!disabledReason || !!dbeBlock}
          describedBy={describedByIds(disabledReason ? disabledReasonId : null, dbeBlock ? dbeNoteId : null, runNoteId)}
          accent="emerald"
          onClick={handleRun}
        />
      </div>

      {isRunning && progress.active && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-emerald), #6ee7b7)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span className="flex items-center gap-2 tabular-nums">
              {progress.percent}%
              <button
                onClick={() => { cancelProgress(BACKGROUND_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel background extraction"
                aria-label="Cancel background extraction"
                className="text-zinc-500 hover:text-red-400 transition-colors"
              >
                <X size={11} />
              </button>
            </span>
          </div>
        </div>
      )}

      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          {result.composite && result.res.chain_restarted && (
            <div className="text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded-lg px-3 py-1.5">
              {COMPOSITE_RESTARTED_NOTICE}
            </div>
          )}

          {debandAxis ? (
            <div className="grid grid-cols-2 gap-2 text-xs">
              <div className="ab-metric-card">
                <div className="text-zinc-500 text-[10px]">Axis</div>
                <div data-testid="background-deband-axis" className="text-zinc-200 font-mono text-[11px]">{debandAxis}</div>
              </div>
              <div className="ab-metric-card">
                <div className="text-zinc-500 text-[10px]">Time</div>
                <div className="text-zinc-200 font-mono text-[11px]">{formatSeconds(result.res.elapsed_ms ?? 0)}</div>
              </div>
            </div>
          ) : (
            <ResultGrid items={resultItems} columns={result.composite ? 2 : resultItems.length === 4 ? 4 : 3} />
          )}

          {(result.correctedUrl || result.modelUrl) && (
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                {views.map((v) => (
                  <button
                    key={v}
                    onClick={() => setView(v)}
                    title={v === "compare" ? "Wipe between the original image and the corrected result" : undefined}
                    className={`text-xs px-2.5 py-1 rounded-md transition-all ${shownView === v ? "bg-emerald-600/20 text-emerald-300 ring-1 ring-emerald-500/30" : "text-zinc-500 hover:text-zinc-300"}`}
                  >
                    {RESULT_VIEW_LABELS[v]}
                  </button>
                ))}
                {canOverlay && shownView !== "compare" && (
                  <button onClick={() => setShowSamples((v) => !v)} className={`text-xs px-2.5 py-1 rounded-md transition-all ml-auto ${showSamples ? "bg-emerald-600/20 text-emerald-300 ring-1 ring-emerald-500/30" : "text-zinc-500 hover:text-zinc-300"}`}>
                    Samples
                  </button>
                )}
              </div>
              {shownView === "compare" && result.inputUrl && result.correctedUrl ? (
                <CompareView originalUrl={result.inputUrl} resultUrl={result.correctedUrl} originalLabel={result.inputLabel} resultLabel={resultLabel} accent="emerald" />
              ) : (
                <div className="relative w-full aspect-square rounded-lg overflow-hidden bg-zinc-900 border border-zinc-700/50">
                  <img src={showModel ? result.modelUrl : result.correctedUrl} alt={showModel ? modelLabel : "Corrected"} className="absolute inset-0 w-full h-full object-contain" draggable={false} />
                  {canOverlay && showSamples && splineSamples && overlayDims && (
                    <svg viewBox={`0 0 ${overlayDims[0]} ${overlayDims[1]}`} preserveAspectRatio="xMidYMid meet" className="absolute inset-0 w-full h-full pointer-events-none">
                      {splineSamples.map((s, i) => (
                        <rect
                          key={i}
                          x={s.x - overlayRadius}
                          y={s.y - overlayRadius}
                          width={2 * overlayRadius + 1}
                          height={2 * overlayRadius + 1}
                          fill="none"
                          stroke={sampleStroke(s)}
                          strokeWidth={1}
                          vectorEffect="non-scaling-stroke"
                          opacity={s.rejected ? 0.9 : 0.7}
                        />
                      ))}
                    </svg>
                  )}
                  <div className="ab-compare-label left-2">{showModel ? modelLabel : resultLabel}</div>
                </div>
              )}
              {canOverlay && showSamples && shownView !== "compare" && (
                <div className="flex items-center gap-3 text-[10px] text-zinc-500 px-1">
                  <span><span className="inline-block w-2 h-2 rounded-sm mr-1 align-middle" style={{ background: SAMPLE_STROKE.accepted }} />used</span>
                  <span><span className="inline-block w-2 h-2 rounded-sm mr-1 align-middle" style={{ background: SAMPLE_STROKE.manual }} />point region</span>
                  <span><span className="inline-block w-2 h-2 rounded-sm mr-1 align-middle" style={{ background: SAMPLE_STROKE.rejected }} />rejected</span>
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
