import { useState, useCallback, useId, useMemo } from "react";
import { X } from "lucide-react";
import { extractBackground } from "../../services/processing";
import { extractBackgroundDbe } from "../../services/dbe";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { bustPreviewUrl, isCancelMessage, useProcessingRun } from "../../hooks/useProcessingRun";
import { BACKGROUND_PROGRESS_EVENT } from "../../shared/types/processing";
import { Slider, Toggle, RunButton, ResultGrid, ChainBanner, ErrorAlert, SectionHeader, CompareView } from "../ui";
import { useRegionDoc } from "../../hooks/useRegionStore";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useRenderContext } from "../../context/PreviewContext";
import { chainHoldsOutput } from "../../utils/processingChain";
import { DEFAULT_DBE_CONFIG, pointSamplesFromDoc, validateDbeParams } from "../../utils/dbeSamples";
import type { ProcessedFile } from "../../shared/types";
import type { DbeConfig, DbeMode, DbeSample } from "../../shared/types/dbe";

type BackgroundModel = "polynomial" | "spline";

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
}

interface BackgroundParams {
  gridSize: number;
  polyDegree: number;
  sigmaClip: number;
  iterations: number;
  mode: string;
}

interface BackgroundRun {
  res: BackgroundResult;
  correctedUrl: string | undefined;
  modelUrl: string | undefined;
  inputUrl: string | null;
  spline: boolean;
  sampleRadius: number;
}

type ResultView = "corrected" | "model" | "compare";

const RESULT_VIEW_LABELS: Record<ResultView, string> = {
  corrected: "Corrected",
  model: "Model",
  compare: "Compare",
};

interface BackgroundPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onProcessingDone?: (result: BackgroundResult) => void;
  chainedFrom?: string | null;
  fileKey?: string | null;
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

export default function BackgroundPanel({ selectedFile, outputDir = "./output", onProcessingDone, chainedFrom, fileKey }: BackgroundPanelProps) {
  const progress = useProgress(BACKGROUND_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [model, setModel] = useState<BackgroundModel>("polynomial");
  const [params, setParams] = useState<BackgroundParams>({
    gridSize: 8,
    polyDegree: 3,
    sigmaClip: 2.5,
    iterations: 3,
    mode: "subtract",
  });
  const [dbe, setDbe] = useState<DbeConfig>(DEFAULT_DBE_CONFIG);
  const [usePointRegions, setUsePointRegions] = useState(true);
  const { running: isRunning, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<BackgroundRun>("background", fileKey ?? null);
  const { chain } = useRenderContext();
  const result = runResult && chainHoldsOutput(chain, "background", runResult.res.corrected_fits) ? runResult : null;
  const [view, setView] = useState<ResultView>("corrected");
  const [showSamples, setShowSamples] = useState(true);

  const modelId = useId();
  const modeId = useId();

  const regionKey = useRegionKey();
  const regionDoc = useRegionDoc(regionKey);
  const pointSamples = useMemo(() => pointSamplesFromDoc(regionDoc), [regionDoc]);

  const update = useCallback(<K extends keyof BackgroundParams>(key: K, value: BackgroundParams[K]) => {
    setParams((prev) => ({ ...prev, [key]: value }));
  }, []);

  const updateDbe = useCallback(<K extends keyof DbeConfig>(key: K, value: DbeConfig[K]) => {
    setDbe((prev) => ({ ...prev, [key]: value }));
  }, []);

  const setMode = useCallback((mode: DbeMode) => {
    setParams((prev) => ({ ...prev, mode }));
    setDbe((prev) => ({ ...prev, mode }));
  }, []);

  const runPolynomial = useCallback(async (path: string): Promise<BackgroundResult> => {
    return extractBackground(path, outputDir, {
      gridSize: params.gridSize,
      polyDegree: params.polyDegree,
      sigmaClip: params.sigmaClip,
      iterations: params.iterations,
      mode: params.mode,
    });
  }, [outputDir, params]);

  const runSpline = useCallback(async (path: string): Promise<BackgroundResult> => {
    const config: DbeConfig = { ...dbe, manualSamples: usePointRegions ? pointSamples : [] };
    const dims = selectedFile?.result?.dimensions;
    const imageSize = dims ? { width: dims[0], height: dims[1] } : null;
    const validation = validateDbeParams(config, imageSize);
    if (!validation.ok) throw new Error(validation.errors.join("; "));
    return extractBackgroundDbe(path, outputDir, config);
  }, [dbe, usePointRegions, pointSamples, selectedFile, outputDir]);

  const handleRun = useCallback(() => {
    if (!selectedFile?.path) return;
    const path = selectedFile.path;
    const inputUrl = selectedFile.result?.previewUrl ?? null;
    const spline = model === "spline";
    const sampleRadius = dbe.sampleRadius;
    resetProgress();
    void run(async () => {
      const res = spline ? await runSpline(path) : await runPolynomial(path);
      onProcessingDone?.(res);
      const stamp = Date.now();
      return {
        res,
        correctedUrl: bustPreviewUrl(res?.previewUrl, stamp),
        modelUrl: bustPreviewUrl(res?.modelUrl, stamp),
        inputUrl,
        spline,
        sampleRadius,
      };
    }, isCancelMessage).finally(resetProgress);
  }, [selectedFile, model, dbe.sampleRadius, runSpline, runPolynomial, resetProgress, run, onProcessingDone]);

  const isSpline = model === "spline";
  const splineSamples = result?.res.samples;
  const overlayDims = result?.res.dimensions;
  const overlayRadius = result?.sampleRadius ?? 0;
  const canOverlay = !!result?.spline && !!splineSamples && !!overlayDims && overlayDims[0] > 0 && overlayDims[1] > 0;
  const canCompare = !!result?.inputUrl && !!result.correctedUrl;
  const views: ResultView[] = canCompare ? ["corrected", "model", "compare"] : ["corrected", "model"];
  const shownView: ResultView = view === "compare" && !canCompare ? "corrected" : view;
  const showModel = shownView === "model";
  const resultItems = [
    { label: "Samples", value: result?.res.sample_count },
    ...(result?.res.rejected_count !== undefined ? [{ label: "Rejected", value: result.res.rejected_count }] : []),
    { label: "RMS", value: result?.res.rms_residual?.toExponential(2) },
    { label: "Time", value: `${((result?.res.elapsed_ms ?? 0) / 1000).toFixed(1)}s` },
  ];

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Background Extraction" />
      <ChainBanner chainedFrom={chainedFrom} accent="emerald" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to enable background extraction.</div>
      )}

      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <label htmlFor={modelId} className="text-xs text-zinc-400">Model</label>
          <select id={modelId} value={model} onChange={(e) => setModel(e.target.value as BackgroundModel)} disabled={isRunning} className="ab-select">
            <option value="polynomial">Polynomial</option>
            <option value="spline">Spline (DBE)</option>
          </select>
        </div>

        {!isSpline && (
          <>
            <Slider label="Grid Size" value={params.gridSize} min={3} max={24} step={1} disabled={isRunning} accent="emerald" onChange={(v) => update("gridSize", v)} />
            <Slider label="Polynomial Degree" value={params.polyDegree} min={1} max={5} step={1} disabled={isRunning} accent="emerald" onChange={(v) => update("polyDegree", v)} />
            <Slider label="Sigma Clip" value={params.sigmaClip} min={1} max={5} step={0.1} disabled={isRunning} accent="emerald" format={(v) => v.toFixed(1)} onChange={(v) => update("sigmaClip", v)} />
            <Slider label="Iterations" value={params.iterations} min={1} max={10} step={1} disabled={isRunning} accent="emerald" onChange={(v) => update("iterations", v)} />
          </>
        )}

        {isSpline && (
          <>
            <Slider label="Sample Radius (px)" value={dbe.sampleRadius} min={1} max={32} step={1} disabled={isRunning} accent="emerald" onChange={(v) => updateDbe("sampleRadius", Math.round(v))} />
            <Slider label="Tolerance (sigma)" value={dbe.tolerance} min={0.1} max={5} step={0.1} disabled={isRunning} accent="emerald" format={(v) => v.toFixed(1)} onChange={(v) => updateDbe("tolerance", v)} />
            <Slider label="Smoothing" value={dbe.smoothing} min={0} max={1} step={0.01} disabled={isRunning} accent="emerald" format={(v) => v.toFixed(2)} onChange={(v) => updateDbe("smoothing", v)} />
            <Slider
              label="Auto Grid (per axis)"
              value={dbe.autoGrid ?? 0}
              min={0}
              max={30}
              step={1}
              disabled={isRunning}
              accent="emerald"
              format={(v) => (v === 0 ? "Off" : `${v}`)}
              onChange={(v) => updateDbe("autoGrid", v === 0 ? null : Math.round(v))}
            />
            <Toggle
              label="Use point regions as samples"
              checked={usePointRegions}
              disabled={isRunning}
              accent="emerald"
              badge={pointSamples.length > 0 ? `${pointSamples.length} pts` : null}
              onChange={setUsePointRegions}
            />
            <Toggle label="Reject stars" checked={dbe.rejectStars} disabled={isRunning} accent="emerald" onChange={(v) => updateDbe("rejectStars", v)} />
            <Toggle label="Keep image median" checked={dbe.normalize} disabled={isRunning} accent="emerald" onChange={(v) => updateDbe("normalize", v)} />
          </>
        )}

        <div className="flex items-center justify-between">
          <label htmlFor={modeId} className="text-xs text-zinc-400">Mode</label>
          <select id={modeId} value={isSpline ? dbe.mode : params.mode} onChange={(e) => setMode(e.target.value as DbeMode)} disabled={isRunning} className="ab-select">
            <option value="subtract">Subtract</option>
            <option value="divide">Divide</option>
          </select>
        </div>
      </div>

      <div title={busyTitle}>
        <RunButton label={isSpline ? "Extract Background (Spline)" : "Extract Background"} runningLabel="Extracting..." running={isRunning} disabled={!selectedFile || blocked} accent="emerald" onClick={handleRun} />
      </div>

      {isRunning && progress.active && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-emerald), #6ee7b7)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span className="flex items-center gap-2">
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
          <ResultGrid items={resultItems} columns={resultItems.length === 4 ? 4 : 3} />

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
                <CompareView originalUrl={result.inputUrl} resultUrl={result.correctedUrl} originalLabel="Original" resultLabel="Background Removed" accent="emerald" />
              ) : (
                <div className="relative w-full aspect-square rounded-lg overflow-hidden bg-zinc-900 border border-zinc-700/50">
                  <img src={showModel ? result.modelUrl : result.correctedUrl} alt={showModel ? "Background Model" : "Corrected"} className="absolute inset-0 w-full h-full object-contain" draggable={false} />
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
                  <div className="ab-compare-label left-2">{showModel ? "Background Model" : "Background Removed"}</div>
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
