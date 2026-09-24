import { useState, useCallback, useEffect } from "react";
import { X } from "lucide-react";
import { waveletDenoise } from "../../services/processing";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { INPUT_CHANGED_MESSAGE, bustPreviewUrl, isCancelMessage, useProcessingRun } from "../../hooks/useProcessingRun";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types";
import { useRenderContext } from "../../context/PreviewContext";
import { chainHoldsOutput } from "../../utils/processingChain";
import { WAVELET_PROGRESS_EVENT } from "../../shared/types/processing";

const DEFAULT_THRESHOLDS = [3.0, 2.5, 2.0, 1.5, 1.0];
const DEFAULT_BIAS = 0;
const BIAS_MIN = -1;
const BIAS_MAX = 3;
const BIAS_STEP = 0.05;
const SCALE_LABELS = ["Fine detail", "Small structures", "Medium structures", "Large structures", "Very large"];

interface WaveletResult {
  previewUrl?: string;
  fits_path?: string;
  scales_processed?: number;
  noise_estimate?: number;
  elapsed_ms?: number;
}

interface WaveletRun {
  res: WaveletResult;
  resultUrl: string | undefined;
  baseUrl: string | null;
  baseLabel: string;
}

interface WaveletPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onProcessingDone?: (result: WaveletResult) => void;
  chainedFrom?: string | null;
  inputPreviewUrl?: string | null;
  inputLabel?: string;
  fileKey?: string | null;
}

const ICON = (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-sky-400">
    <path d="M2 12c0 0 2-4 5-4s3 8 6 8 5-4 5-4" />
    <path d="M2 12c0 0 2 4 5 4s3-8 6-8 5 4 5 4" opacity="0.3" />
  </svg>
);

function fitToScales(prev: number[], numScales: number, defaults: readonly number[], fallback: number): number[] {
  if (prev.length === numScales) return prev;
  const next: number[] = [];
  for (let i = 0; i < numScales; i++) {
    next.push(prev[i] ?? defaults[i] ?? fallback);
  }
  return next;
}

function scaleLabel(idx: number): string {
  return SCALE_LABELS[idx] || `Scale ${idx + 1}`;
}

function formatBias(v: number): string {
  return `${v > 0 ? "+" : ""}${v.toFixed(2)}`;
}

export default function WaveletPanel({ selectedFile, outputDir = "./output", onProcessingDone, chainedFrom, inputPreviewUrl, inputLabel, fileKey }: WaveletPanelProps) {
  const progress = useProgress(WAVELET_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [numScales, setNumScales] = useState(5);
  const [thresholds, setThresholds] = useState<number[]>([...DEFAULT_THRESHOLDS]);
  const [layerBias, setLayerBias] = useState<number[]>(() => Array(5).fill(DEFAULT_BIAS));
  const [linear, setLinear] = useState(true);
  const { running: isRunning, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<WaveletRun>("denoise", fileKey ?? null);
  const { chain } = useRenderContext();
  const result = runResult && chainHoldsOutput(chain, "denoise", runResult.res.fits_path) ? runResult : null;

  const updateThreshold = useCallback((idx: number, value: number) => {
    setThresholds((prev) => {
      const next = [...prev];
      next[idx] = value;
      return next;
    });
  }, []);

  const updateBias = useCallback((idx: number, value: number) => {
    setLayerBias((prev) => {
      const next = [...prev];
      next[idx] = value;
      return next;
    });
  }, []);

  const resetBias = useCallback(() => {
    setLayerBias(Array(numScales).fill(DEFAULT_BIAS));
  }, [numScales]);

  useEffect(() => {
    setThresholds((prev) => fitToScales(prev, numScales, DEFAULT_THRESHOLDS, 1.0));
    setLayerBias((prev) => fitToScales(prev, numScales, [], DEFAULT_BIAS));
  }, [numScales]);

  const hasBias = layerBias.slice(0, numScales).some((b) => b !== DEFAULT_BIAS);

  const handleRun = useCallback(() => {
    if (!selectedFile?.path) return;
    const path = selectedFile.path;
    const baseUrl = inputPreviewUrl ?? null;
    const baseLabel = inputLabel ?? "Original";
    resetProgress();
    void run(async (ctx) => {
      const res = await waveletDenoise(path, outputDir, {
        numScales,
        thresholds: thresholds.slice(0, numScales),
        linear,
        layerBias: layerBias.slice(0, numScales),
      });
      if (!ctx.inputUnchanged("denoise")) throw new Error(INPUT_CHANGED_MESSAGE);
      onProcessingDone?.(res);
      return { res, resultUrl: bustPreviewUrl(res?.previewUrl, Date.now()), baseUrl, baseLabel };
    }, isCancelMessage).finally(resetProgress);
  }, [selectedFile, outputDir, numScales, thresholds, linear, layerBias, resetProgress, run, inputPreviewUrl, inputLabel, onProcessingDone]);

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Wavelet Noise Reduction" />
      <ChainBanner chainedFrom={chainedFrom} accent="sky" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to enable noise reduction.</div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Scales" value={numScales} min={2} max={8} step={1} disabled={isRunning} accent="sky" onChange={setNumScales} />

        <div className="flex flex-col gap-1.5">
          <label className="text-xs text-zinc-400">Threshold per scale (sigma)</label>
          {thresholds.slice(0, numScales).map((val, idx) => (
            <div key={idx} className="flex items-center gap-2">
              <span className="text-[10px] text-zinc-500 w-24 truncate">
                {scaleLabel(idx)}
              </span>
              <div className="flex-1">
                <Slider
                  label=""
                  value={val}
                  min={0}
                  max={5}
                  step={0.1}
                  disabled={isRunning}
                  accent="sky"
                  format={(v) => v.toFixed(1)}
                  onChange={(v) => updateThreshold(idx, v)}
                />
              </div>
              <span className="text-[10px] font-mono text-zinc-300 w-6 text-right">{val.toFixed(1)}</span>
            </div>
          ))}
        </div>

        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Detail bias per scale</label>
            {hasBias && (
              <button
                type="button"
                className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors"
                disabled={isRunning}
                onClick={resetBias}
              >
                Reset
              </button>
            )}
          </div>
          {layerBias.slice(0, numScales).map((val, idx) => (
            <div key={idx} className="flex items-center gap-2">
              <span className="text-[10px] text-zinc-500 w-24 truncate">
                {scaleLabel(idx)}
              </span>
              <div className="flex-1">
                <Slider
                  label=""
                  value={val}
                  min={BIAS_MIN}
                  max={BIAS_MAX}
                  step={BIAS_STEP}
                  disabled={isRunning}
                  accent="sky"
                  format={formatBias}
                  onChange={(v) => updateBias(idx, v)}
                />
              </div>
              <span className="text-[10px] font-mono text-zinc-300 w-10 text-right">{formatBias(val)}</span>
            </div>
          ))}
          <span className="text-[10px] text-zinc-600">0 keeps the layer, positive sharpens, negative softens (-1 removes it).</span>
        </div>

        <Toggle label="Soft threshold (linear)" checked={linear} disabled={isRunning} accent="sky" onChange={setLinear} />
      </div>

      <div title={busyTitle}>
        <RunButton label="Run Noise Reduction" runningLabel="Denoising..." running={isRunning} disabled={!selectedFile || blocked} accent="sky" onClick={handleRun} />
      </div>

      {isRunning && progress.active && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-sky), #7dd3fc)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span className="flex items-center gap-2">
              {progress.percent}%
              <button
                onClick={() => { cancelProgress(WAVELET_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel noise reduction"
                aria-label="Cancel noise reduction"
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
          <ResultGrid items={[
            { label: "Scales", value: result.res.scales_processed },
            { label: "Noise est.", value: result.res.noise_estimate?.toExponential(2) },
            { label: "Time", value: `${((result.res.elapsed_ms ?? 0) / 1000).toFixed(1)}s` },
          ]} />

          {result.baseUrl && result.resultUrl && (
            <CompareView originalUrl={result.baseUrl} resultUrl={result.resultUrl} originalLabel={result.baseLabel} resultLabel="Denoised" accent="sky" />
          )}
        </div>
      )}
    </div>
  );
}
