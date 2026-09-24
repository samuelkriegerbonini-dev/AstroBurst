import { useState, useCallback, useEffect, useId, useRef, useMemo } from "react";
import { Layers, GripVertical, ArrowDown, CheckCircle2, X } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, ErrorAlert, SectionHeader, WarningList } from "../ui";
import { noiseWeightsFor, stackFrames } from "../../services/stacking";
import type { CombineMethod, NormalizationMethod, RejectionMethod, StackResult } from "../../shared/types/stacking";
import { STACK_PROGRESS_EVENT } from "../../shared/types/stacking";
import { getOutputDir } from "../../infrastructure/tauri";
import type { ProcessedFile } from "../../shared/types";
import type { RunTarget, StackConfig } from "./StackingTab";
import { resolveEffectivePath } from "../../hooks/useFileStore";
import { stackOutputName } from "../../utils/stackingOutputs";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { useTimer } from "../../hooks/useTimer";
import { combineFrameWeights, formatWeightRange } from "../../utils/noiseWeights";
import {
  appendMissingPaths,
  COMBINE_OPTIONS,
  DEFAULT_STACK_SETTINGS,
  NORMALIZATION_OPTIONS,
  REJECTION_OPTIONS,
  rejectionFrameHint,
  rejectionUsesSigma,
  selectableStackPaths,
  stackProgressText,
  subframeWeightsFor,
} from "../../utils/stackingRejection";

interface StackingPanelProps {
  files: ProcessedFile[];
  runTarget?: RunTarget | null;
  onResult?: (result: StackResult, inputs: string[], target: RunTarget | null) => void;
  injectedPaths?: string[];
  acceptedPaths?: string[];
  stackConfig?: StackConfig;
  onStackConfigChange?: (config: Partial<StackConfig>) => void;
  rejectedPaths?: string[];
  subframeWeights?: Record<string, number>;
}

const ICON = <Layers size={14} className="text-amber-400" />;
const ITEM_HEIGHT = 28;
const OVERSCAN = 6;

type FrameRow =
  | { kind: "header"; key: string }
  | { kind: "injected"; key: string; path: string; label: string }
  | { kind: "file"; key: string; path: string; label: string };

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

export default function StackingPanel({
  files = [],
  runTarget = null,
  onResult,
  injectedPaths = [],
  acceptedPaths,
  stackConfig,
  onStackConfigChange,
  rejectedPaths = [],
  subframeWeights,
}: StackingPanelProps) {
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [isStacking, setIsStacking] = useState(false);
  const [result, setResult] = useState<StackResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [noiseWeighting, setNoiseWeighting] = useState(false);
  const [noiseWeightRange, setNoiseWeightRange] = useState<string | null>(null);
  const prevInjectedRef = useRef<string[]>([]);
  const listRef = useRef<HTMLDivElement>(null);
  const listRafRef = useRef<number | null>(null);
  const [listScrollTop, setListScrollTop] = useState(0);
  const [listHeight, setListHeight] = useState(240);
  const progress = useProgress(STACK_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const rejectionId = useId();
  const combineId = useId();
  const normalizationId = useId();
  const alignMethodId = useId();
  const timer = useTimer();
  const startTimer = timer.start;
  const stopTimer = timer.stop;
  const resetTimer = timer.reset;

  const replaceSelection = useCallback((update: (prev: string[]) => string[]) => {
    setNoiseWeightRange(null);
    setSelectedPaths(update);
  }, []);

  const config: StackConfig = { ...DEFAULT_STACK_SETTINGS, ...stackConfig };
  const {
    sigmaLow,
    sigmaHigh,
    maxIterations,
    align,
    rejection,
    combine,
    normalization,
    winsorCutoff,
    percentileLow,
    percentileHigh,
    minmaxLow,
    minmaxHigh,
    rejectionMaps,
  } = config;
  const alignMethod = config.alignMethod ?? "phase_correlation";

  useEffect(() => {
    if (injectedPaths.length === 0) return;
    const newPaths = injectedPaths.filter((p) => !prevInjectedRef.current.includes(p));
    if (newPaths.length === 0) return;
    prevInjectedRef.current = injectedPaths;
    replaceSelection((prev) => appendMissingPaths(prev, newPaths));
  }, [injectedPaths, replaceSelection]);

  const prevAcceptedRef = useRef<string[] | null>(null);
  useEffect(() => {
    if (!acceptedPaths) return;
    if (prevAcceptedRef.current === acceptedPaths) return;
    prevAcceptedRef.current = acceptedPaths;
    if (acceptedPaths.length === 0) return;
    replaceSelection(() => [...acceptedPaths]);
  }, [acceptedPaths, replaceSelection]);

  const prevRejectedRef = useRef<string>("");
  useEffect(() => {
    const key = rejectedPaths.join("|");
    if (key === prevRejectedRef.current) return;
    prevRejectedRef.current = key;
    if (rejectedPaths.length === 0) return;
    const rejected = new Set(rejectedPaths);
    replaceSelection((prev) => prev.filter((p) => !rejected.has(p)));
  }, [rejectedPaths, replaceSelection]);

  const selectedSet = useMemo(() => new Set(selectedPaths), [selectedPaths]);
  const rejectedSet = useMemo(() => new Set(rejectedPaths), [rejectedPaths]);

  const toggleFile = useCallback((path: string) => {
    replaceSelection((prev) =>
      prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path],
    );
  }, [replaceSelection]);

  const selectAll = useCallback(() => {
    const allPaths = selectableStackPaths(files.map((f) => f.path), injectedPaths, rejectedPaths);
    replaceSelection(() => allPaths);
  }, [files, injectedPaths, rejectedPaths, replaceSelection]);

  const selectNone = useCallback(() => replaceSelection(() => []), [replaceSelection]);

  useEffect(() => {
    const el = listRef.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setListHeight(entry.contentRect.height));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const handleListScroll = useCallback(() => {
    if (listRafRef.current) return;
    listRafRef.current = requestAnimationFrame(() => {
      listRafRef.current = null;
      if (listRef.current) setListScrollTop(listRef.current.scrollTop);
    });
  }, []);

  useEffect(() => {
    return () => { if (listRafRef.current) cancelAnimationFrame(listRafRef.current); };
  }, []);

  const weights = useMemo(() => subframeWeightsFor(selectedPaths, subframeWeights), [selectedPaths, subframeWeights]);
  const weightedCount = weights ? weights.filter((w) => w !== 1.0).length : 0;
  const hint = rejectionFrameHint(rejection, selectedPaths.length, minmaxLow, minmaxHigh);

  const stackProgressLabel = stackProgressText(progress.stage, progress.current, progress.total);

  const handleStack = useCallback(async () => {
    if (selectedPaths.length < 2) return;
    const target = runTarget;
    const inputs = [...selectedPaths];
    const name = stackOutputName(inputs[0], inputs.length, new Date());
    setIsStacking(true);
    setError(null);
    setResult(null);
    resetProgress();
    resetTimer();
    startTimer();
    try {
      const paths = inputs.map(resolveEffectivePath);
      let frameWeights = weights;
      if (noiseWeighting) {
        const noise = await noiseWeightsFor(paths);
        frameWeights = combineFrameWeights(weights, noise.weights);
        setNoiseWeightRange(formatWeightRange({
          weights: frameWeights,
          min: Math.min(...frameWeights),
          max: Math.max(...frameWeights),
          missing: noise.missing,
        }));
      } else {
        setNoiseWeightRange(null);
      }
      const res = await stackFrames(paths, await getOutputDir(), {
        name,
        sigmaLow,
        sigmaHigh,
        maxIterations,
        align,
        alignMethod,
        weights: frameWeights,
        rejection,
        combine,
        normalization,
        winsorCutoff,
        percentileLow,
        percentileHigh,
        minmaxLow,
        minmaxHigh,
        rejectionMaps,
      });
      setResult(res);
      onResult?.(res, inputs, target);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (!/cancel/i.test(msg)) setError(msg);
    } finally {
      setIsStacking(false);
      stopTimer();
      resetProgress();
    }
  }, [
    selectedPaths,
    resetProgress,
    resetTimer,
    startTimer,
    stopTimer,
    sigmaLow,
    sigmaHigh,
    maxIterations,
    align,
    alignMethod,
    weights,
    noiseWeighting,
    rejection,
    combine,
    normalization,
    winsorCutoff,
    percentileLow,
    percentileHigh,
    minmaxLow,
    minmaxHigh,
    rejectionMaps,
    runTarget,
    onResult,
  ]);

  const rows = useMemo<FrameRow[]>(() => {
    const filePaths = new Set(files.map((f) => f.path));
    const injectedOnly = injectedPaths.filter((p) => !filePaths.has(p));
    const list: FrameRow[] = [];
    if (injectedOnly.length > 0) {
      list.push({ kind: "header", key: "__calibration_header" });
      for (const path of injectedOnly) {
        list.push({ kind: "injected", key: `inj:${path}`, path, label: fileName(path) });
      }
    }
    for (const f of files) {
      list.push({ kind: "file", key: f.id, path: f.path, label: f.name });
    }
    return list;
  }, [files, injectedPaths]);

  const startIdx = Math.max(0, Math.floor(listScrollTop / ITEM_HEIGHT) - OVERSCAN);
  const endIdx = Math.min(rows.length, Math.ceil((listScrollTop + listHeight) / ITEM_HEIGHT) + OVERSCAN);
  const visibleRows = useMemo(() => rows.slice(startIdx, endIdx), [rows, startIdx, endIdx]);

  return (
    <div className="flex flex-col gap-4 p-4 h-full min-h-0 overflow-y-auto">
      <div className="flex items-center justify-between">
        <SectionHeader icon={ICON} title="Frames to Stack" subtitle={selectedPaths.length > 0 ? `${selectedPaths.length} selected` : undefined} />
        <div className="flex gap-2">
          <button onClick={selectAll} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">All</button>
          <button onClick={selectNone} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">None</button>
        </div>
      </div>

      <div ref={listRef} onScroll={handleListScroll} className="flex-1 min-h-[160px] overflow-y-auto">
        <div style={{ height: rows.length * ITEM_HEIGHT, position: "relative" }}>
          <div style={{ position: "absolute", top: startIdx * ITEM_HEIGHT, left: 0, right: 0 }}>
            {visibleRows.map((row) => {
              if (row.kind === "header") {
                return (
                  <div key={row.key} style={{ height: ITEM_HEIGHT }} className="flex items-center gap-1.5 px-2 text-[10px] text-emerald-400/80">
                    <ArrowDown size={10} />
                    From Calibration
                  </div>
                );
              }
              const isSelected = selectedSet.has(row.path);
              if (row.kind === "injected") {
                return (
                  <button key={row.key} onClick={() => toggleFile(row.path)} style={{ height: ITEM_HEIGHT }} className={`w-full flex items-center gap-2 px-2.5 rounded text-[11px] transition-all text-left ${isSelected ? "bg-emerald-500/10 text-zinc-200 ring-1 ring-emerald-500/30" : "text-zinc-500 hover:bg-zinc-800/40 hover:text-zinc-300"}`}>
                    <GripVertical size={10} className="text-zinc-700 shrink-0" />
                    <span className={`w-3 h-3 rounded-sm border flex items-center justify-center shrink-0 ${isSelected ? "bg-emerald-500/20 border-emerald-500" : "border-zinc-600"}`}>
                      {isSelected && <CheckCircle2 size={10} className="text-emerald-400" />}
                    </span>
                    <span className="truncate">{row.label}</span>
                    <span className="ml-auto text-[9px] text-emerald-500/60 shrink-0">calibrated</span>
                  </button>
                );
              }
              const isRejected = rejectedSet.has(row.path);
              const weight = subframeWeights?.[row.path];
              return (
                <button key={row.key} onClick={() => toggleFile(row.path)} style={{ height: ITEM_HEIGHT }} className={`w-full flex items-center gap-2 px-2.5 rounded text-[11px] transition-all text-left ${isSelected ? "bg-amber-500/10 text-zinc-200 ring-1 ring-amber-500/30" : "text-zinc-500 hover:bg-zinc-800/40 hover:text-zinc-300"} ${isRejected && !isSelected ? "opacity-50" : ""}`}>
                  <GripVertical size={10} className="text-zinc-700 shrink-0" />
                  <span className={`w-3 h-3 rounded-sm border flex items-center justify-center shrink-0 ${isSelected ? "bg-amber-500/20 border-amber-500" : "border-zinc-600"}`}>
                    {isSelected && <CheckCircle2 size={10} className="text-amber-400" />}
                  </span>
                  <span className="truncate">{row.label}</span>
                  {isRejected ? (
                    <span className="ml-auto text-[9px] text-red-400/70 shrink-0" title="Rejected by subframe quality analysis">rejected</span>
                  ) : weight !== undefined ? (
                    <span className="ml-auto text-[9px] text-teal-400/70 shrink-0 font-mono" title="Subframe quality weight">{(weight * 100).toFixed(0)}%</span>
                  ) : null}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <span className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">Pixel Rejection</span>
        <div className="flex items-center justify-between">
          <label htmlFor={rejectionId} className="text-xs text-zinc-400">Rejection</label>
          <select id={rejectionId} value={rejection} onChange={(e) => onStackConfigChange?.({ rejection: e.target.value as RejectionMethod })} className="ab-select">
            {REJECTION_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </select>
        </div>
        <p className={`text-[10px] leading-snug ${hint.severity === "warn" ? "text-amber-400/80" : "text-zinc-500"}`}>{hint.text}</p>

        {rejectionUsesSigma(rejection) && (
          <>
            <Slider label="Sigma Low" value={sigmaLow} min={1.0} max={6.0} step={0.1} accent="amber" format={(v) => v.toFixed(1)} onChange={(v) => onStackConfigChange?.({ sigmaLow: v })} />
            <Slider label="Sigma High" value={sigmaHigh} min={1.0} max={6.0} step={0.1} accent="amber" format={(v) => v.toFixed(1)} onChange={(v) => onStackConfigChange?.({ sigmaHigh: v })} />
            <Slider label="Max Iterations" value={maxIterations} min={1} max={20} step={1} accent="amber" onChange={(v) => onStackConfigChange?.({ maxIterations: v })} />
          </>
        )}
        {rejection === "winsorized_sigma_clip" && (
          <Slider label="Winsorization Cutoff" value={winsorCutoff} min={1.0} max={10.0} step={0.1} accent="amber" format={(v) => `${v.toFixed(1)}σ`} onChange={(v) => onStackConfigChange?.({ winsorCutoff: v })} />
        )}
        {rejection === "percentile_clip" && (
          <>
            <Slider label="Percentile Low" value={percentileLow} min={0.0} max={1.0} step={0.01} accent="amber" format={(v) => v.toFixed(2)} onChange={(v) => onStackConfigChange?.({ percentileLow: v })} />
            <Slider label="Percentile High" value={percentileHigh} min={0.0} max={1.0} step={0.01} accent="amber" format={(v) => v.toFixed(2)} onChange={(v) => onStackConfigChange?.({ percentileHigh: v })} />
          </>
        )}
        {rejection === "min_max" && (
          <>
            <Slider label="Reject Low" value={minmaxLow} min={0} max={10} step={1} accent="amber" onChange={(v) => onStackConfigChange?.({ minmaxLow: v })} />
            <Slider label="Reject High" value={minmaxHigh} min={0} max={10} step={1} accent="amber" onChange={(v) => onStackConfigChange?.({ minmaxHigh: v })} />
          </>
        )}

        <div className="flex items-center justify-between">
          <label htmlFor={combineId} className="text-xs text-zinc-400">Combine</label>
          <select id={combineId} value={combine} onChange={(e) => onStackConfigChange?.({ combine: e.target.value as CombineMethod })} className="ab-select">
            {COMBINE_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </select>
        </div>
        {combine === "median" && (weightedCount > 0 || noiseWeighting) && (
          <p className="text-[10px] text-amber-400/80 leading-snug">Median combination ignores frame weights.</p>
        )}
        <div className="flex items-center justify-between">
          <label htmlFor={normalizationId} className="text-xs text-zinc-400">Normalization</label>
          <select id={normalizationId} value={normalization} onChange={(e) => onStackConfigChange?.({ normalization: e.target.value as NormalizationMethod })} className="ab-select">
            {NORMALIZATION_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
          </select>
        </div>
        <Toggle label="Save rejection maps" checked={rejectionMaps} accent="amber" onChange={(v) => onStackConfigChange?.({ rejectionMaps: v })} />
        {weightedCount > 0 && (
          <p className="text-[10px] text-teal-400/80 leading-snug">Subframe quality weights apply to {weightedCount} selected frame(s).</p>
        )}
        <Toggle label="Weight frames by noise (1/sigma^2)" checked={noiseWeighting} accent="amber" onChange={setNoiseWeighting} />
        {noiseWeighting && (
          <p className="text-[10px] text-zinc-500 leading-snug">
            {noiseWeightRange
              ? `Frame weights ${noiseWeightRange}, normalised to mean 1${weightedCount > 0 ? " and multiplied by the subframe weights" : ""}.`
              : "Noise is estimated per frame (k-sigma MRS) when you stack; quieter frames get more weight."}
          </p>
        )}
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <Toggle label="Auto-align before stacking" checked={align} accent="amber" onChange={(v) => onStackConfigChange?.({ align: v })} />
        {align && (
          <div className="flex items-center justify-between">
            <label htmlFor={alignMethodId} className="text-xs text-zinc-400">Alignment method</label>
            <select id={alignMethodId} value={alignMethod} onChange={(e) => onStackConfigChange?.({ alignMethod: e.target.value })} className="ab-select">
              <option value="phase_correlation">Phase Correlation (translation)</option>
              <option value="affine">Star-based Affine (rotation)</option>
            </select>
          </div>
        )}
      </div>

      <RunButton label={`Stack ${selectedPaths.length} Frames`} runningLabel="Stacking..." running={isStacking} disabled={selectedPaths.length < 2} accent="amber" onClick={handleStack} />

      {isStacking && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-amber), #fcd34d)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{stackProgressLabel}</span>
            <span className="flex items-center gap-2">
              <span className="font-mono">{timer.formatted}</span>
              <button
                onClick={() => { cancelProgress(STACK_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel stacking"
                aria-label="Cancel stacking"
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
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Stacking Complete
          </div>
          <ResultGrid columns={3} items={[
            { label: "Dimensions", value: result.dimensions ? `${result.dimensions[0]}×${result.dimensions[1]}` : "--" },
            { label: "Frames", value: result.frame_count },
            { label: "Rejected", value: result.rejected_pixels ? result.rejected_pixels.toLocaleString() : "0" },
            { label: "Rejection", value: result.rejection ?? rejection },
            { label: "Combine", value: result.combine ?? combine },
            { label: "Normalization", value: result.normalization ?? normalization },
          ]} />
          <WarningList warnings={result.warnings} />
          {result.fits_path && (
            <div className="text-[10px] text-zinc-500 truncate" title={result.fits_path}>
              FITS: <span className="text-zinc-300">{fileName(result.fits_path)}</span>
            </div>
          )}
          {(result.rejection_low_fits || result.rejection_high_fits) && (
            <div className="flex flex-col gap-0.5 text-[10px] text-zinc-500 font-mono">
              {result.rejection_low_fits && <span title={result.rejection_low_fits}>low map: {fileName(result.rejection_low_fits)}</span>}
              {result.rejection_high_fits && <span title={result.rejection_high_fits}>high map: {fileName(result.rejection_high_fits)}</span>}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
