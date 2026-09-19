import { useState, useCallback } from "react";
import { X } from "lucide-react";
import { applyLhe, applyLheComposite } from "../../services/localContrast";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { useCompositeActions, useCompositePreview } from "../../context/CompositeContext";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types/fits.types";
import { DEFAULT_LHE_CONFIG, LHE_LIMITS, LHE_PROGRESS_EVENT } from "../../shared/types/localContrast";
import type { LheConfig, LheHistogramBits, LocalContrastResult } from "../../shared/types/localContrast";

interface LocalContrastPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: LocalContrastResult) => void;
  chainedFrom?: string;
}

const ACCENT = "teal";
const HIST_BITS: LheHistogramBits[] = [8, 10, 12];

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-teal-400">
    <rect x="3" y="3" width="18" height="18" rx="2" opacity="0.4" />
    <path d="M7 16l3-5 3 3 4-7" />
  </svg>
);

export default function LocalContrastPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }: LocalContrastPanelProps) {
  const { isShowingComposite } = useCompositePreview();
  const { setCompositePreviewUrl } = useCompositeActions();
  const progress = useProgress(LHE_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [config, setConfig] = useState<LheConfig>(DEFAULT_LHE_CONFIG);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<LocalContrastResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const update = useCallback(<K extends keyof LheConfig>(key: K, value: LheConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  }, []);

  const canRun = isShowingComposite || !!selectedFile?.path;

  const handleRun = useCallback(async () => {
    if (!canRun) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    resetProgress();
    try {
      if (isShowingComposite) {
        const res = await applyLheComposite(outputDir, config);
        setResult(res);
        if (res.previewUrl) setCompositePreviewUrl(res.previewUrl);
        onProcessingDone?.(res);
      } else if (selectedFile?.path) {
        const res = await applyLhe(selectedFile.path, outputDir, config);
        setResult(res);
        onPreviewUpdate?.(res.previewUrl);
        onProcessingDone?.(res);
      }
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRunning(false);
    }
  }, [canRun, isShowingComposite, selectedFile?.path, outputDir, config, resetProgress, setCompositePreviewUrl, onPreviewUpdate, onProcessingDone]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Local Histogram Equalization" subtitle="CLAHE on lightness" />
      <ChainBanner chainedFrom={chainedFrom} accent={ACCENT} />

      {!canRun && (
        <div className="text-xs text-zinc-500 italic px-1">Select a stretched FITS file or show a stretched composite.</div>
      )}
      {isShowingComposite && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          Composite mode: equalizes the luminance of the stretched RGB composite and preserves hue.
        </div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Kernel Radius" value={config.kernelRadius} min={LHE_LIMITS.kernelRadius.min} max={LHE_LIMITS.kernelRadius.max} step={LHE_LIMITS.kernelRadius.step} scale="log" disabled={isRunning} accent={ACCENT} format={(v) => `${Math.round(v)}px`} onChange={(v) => update("kernelRadius", Math.round(v))} />
        <Slider label="Contrast Limit" value={config.contrastLimit} min={LHE_LIMITS.contrastLimit.min} max={LHE_LIMITS.contrastLimit.max} step={LHE_LIMITS.contrastLimit.step} scale="log" disabled={isRunning} accent={ACCENT} format={(v) => v.toFixed(1)} onChange={(v) => update("contrastLimit", Math.round(v * 2) / 2)} />
        <Slider label="Amount" value={config.amount} min={LHE_LIMITS.amount.min} max={LHE_LIMITS.amount.max} step={LHE_LIMITS.amount.step} disabled={isRunning} accent={ACCENT} format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => update("amount", v)} />

        <div className="flex flex-col gap-1">
          <span className="text-xs text-zinc-400">Histogram Resolution</span>
          <div className="flex gap-1.5">
            {HIST_BITS.map((bits) => (
              <button
                key={bits}
                onClick={() => update("histBits", bits)}
                disabled={isRunning}
                className={`flex-1 py-1 rounded-md text-[10px] font-mono transition-all duration-150 ${
                  config.histBits === bits
                    ? "bg-teal-500/20 text-teal-300 ring-1 ring-teal-500/30"
                    : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
                }`}
              >
                {bits}-bit
              </button>
            ))}
          </div>
        </div>

        <Toggle label="Circular Kernel" checked={config.circular} disabled={isRunning} accent={ACCENT} onChange={(v) => update("circular", v)} />
      </div>

      <RunButton label="Run Local Contrast" runningLabel="Equalizing..." running={isRunning} disabled={!canRun} accent={ACCENT} onClick={handleRun} />

      {isRunning && !isShowingComposite && progress.active && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-teal), #5eead4)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span className="flex items-center gap-2">
              {progress.percent}%
              <button
                onClick={() => { cancelProgress(LHE_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel local contrast"
                aria-label="Cancel local contrast"
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
            { label: "Kernel", value: `${config.kernelRadius}px` },
            { label: "Limit", value: config.contrastLimit.toFixed(1) },
            { label: "Time", value: result.elapsed_ms != null ? `${(result.elapsed_ms / 1000).toFixed(2)}s` : null },
            { label: "Size", value: result.dimensions ? `${result.dimensions[0]}x${result.dimensions[1]}` : null },
          ]} columns={4} />

          {!isShowingComposite && originalUrl && resultUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Original" resultLabel="Equalized" accent={ACCENT} />
          )}
        </div>
      )}
    </div>
  );
}
