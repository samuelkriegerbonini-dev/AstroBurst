import { useState, useCallback } from "react";
import { X } from "lucide-react";
import { applyHdrmt, applyHdrmtComposite } from "../../services/localContrast";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { useCompositeActions, useCompositePreview } from "../../context/CompositeContext";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types/fits.types";
import { DEFAULT_HDR_CONFIG, HDR_LIMITS, HDR_PROGRESS_EVENT } from "../../shared/types/localContrast";
import type { HdrConfig, LocalContrastResult } from "../../shared/types/localContrast";

interface HdrPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: LocalContrastResult) => void;
  chainedFrom?: string;
}

const ACCENT = "violet";
const LINEAR_INPUT_MARKER = "non-linear";

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-violet-400">
    <path d="M3 18c4-8 6-12 9-12s5 4 9 12" />
    <path d="M3 18h18" opacity="0.4" />
    <path d="M8 18c2-3 3-4 4-4s2 1 4 4" opacity="0.6" />
  </svg>
);

function isLinearInputError(message: string | null): boolean {
  return !!message && message.includes(LINEAR_INPUT_MARKER);
}

export default function HdrPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }: HdrPanelProps) {
  const { isShowingComposite } = useCompositePreview();
  const { setCompositePreviewUrl } = useCompositeActions();
  const progress = useProgress(HDR_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [config, setConfig] = useState<HdrConfig>(DEFAULT_HDR_CONFIG);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<LocalContrastResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const update = useCallback(<K extends keyof HdrConfig>(key: K, value: HdrConfig[K]) => {
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
        const res = await applyHdrmtComposite(outputDir, config);
        setResult(res);
        if (res.previewUrl) setCompositePreviewUrl(res.previewUrl);
        onProcessingDone?.(res);
      } else if (selectedFile?.path) {
        const res = await applyHdrmt(selectedFile.path, outputDir, config);
        setResult(res);
        onPreviewUpdate?.(res.previewUrl);
        onProcessingDone?.(res);
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      if (!/cancel/i.test(msg)) setError(msg);
    } finally {
      setIsRunning(false);
      resetProgress();
    }
  }, [canRun, isShowingComposite, selectedFile?.path, outputDir, config, resetProgress, setCompositePreviewUrl, onPreviewUpdate, onProcessingDone]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="HDR Multiscale Transform" subtitle="Large-scale range compression" />
      <ChainBanner chainedFrom={chainedFrom} accent={ACCENT} />

      {!canRun && (
        <div className="text-xs text-zinc-500 italic px-1">Select a stretched FITS file or show a stretched composite.</div>
      )}
      {isShowingComposite && (
        <div className="text-[10px] text-violet-300 bg-violet-900/20 border border-violet-800/30 rounded-lg px-3 py-1.5">
          Composite mode: compresses the stretched RGB composite{config.toLightness ? " on its luminance, preserving hue." : " channel by channel."}
        </div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Layers" value={config.layers} min={HDR_LIMITS.layers.min} max={HDR_LIMITS.layers.max} step={HDR_LIMITS.layers.step} disabled={isRunning} accent={ACCENT} hint="fewer = stronger" format={(v) => `${Math.round(v)}`} onChange={(v) => update("layers", Math.round(v))} />
        <Slider label="Iterations" value={config.iterations} min={HDR_LIMITS.iterations.min} max={HDR_LIMITS.iterations.max} step={HDR_LIMITS.iterations.step} disabled={isRunning} accent={ACCENT} format={(v) => `${Math.round(v)}`} onChange={(v) => update("iterations", Math.round(v))} />
        <Slider label="Overdrive" value={config.overdrive} min={HDR_LIMITS.overdrive.min} max={HDR_LIMITS.overdrive.max} step={HDR_LIMITS.overdrive.step} disabled={isRunning} accent={ACCENT} format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => update("overdrive", v)} />

        <Toggle label="Invert (compress dark structures)" checked={config.inverted} disabled={isRunning} accent={ACCENT} onChange={(v) => update("inverted", v)} />
        {isShowingComposite && (
          <Toggle label="Apply to Lightness" checked={config.toLightness} disabled={isRunning} accent={ACCENT} onChange={(v) => update("toLightness", v)} />
        )}
        <Toggle label="Deringing (protect star cores)" checked={config.deringing} disabled={isRunning} accent={ACCENT} onChange={(v) => update("deringing", v)} />
        {config.deringing && (
          <Slider label="Deringing Amount" value={config.deringingAmount} min={HDR_LIMITS.deringingAmount.min} max={HDR_LIMITS.deringingAmount.max} step={HDR_LIMITS.deringingAmount.step} disabled={isRunning} accent={ACCENT} format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => update("deringingAmount", v)} />
        )}
      </div>

      <RunButton label="Run HDRMT" runningLabel="Compressing..." running={isRunning} disabled={!canRun} accent={ACCENT} onClick={handleRun} />

      {isRunning && !isShowingComposite && progress.active && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-violet), #c4b5fd)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span className="flex items-center gap-2">
              {progress.percent}%
              <button
                onClick={() => { cancelProgress(HDR_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel HDRMT"
                aria-label="Cancel HDRMT"
                className="text-zinc-500 hover:text-red-400 transition-colors"
              >
                <X size={11} />
              </button>
            </span>
          </div>
        </div>
      )}

      <ErrorAlert message={error} />
      {isLinearInputError(error) && (
        <div className="text-[10px] text-amber-300 bg-amber-900/20 border border-amber-800/30 rounded-lg px-3 py-1.5">
          HDRMT works on stretched data. Apply a stretch (Arcsinh, GHS or Masked) first, then run HDRMT on its output.
        </div>
      )}

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <ResultGrid items={[
            { label: "Layers", value: config.layers },
            { label: "Iterations", value: config.iterations },
            { label: "Time", value: result.elapsed_ms != null ? `${(result.elapsed_ms / 1000).toFixed(2)}s` : null },
            { label: "Size", value: result.dimensions ? `${result.dimensions[0]}x${result.dimensions[1]}` : null },
          ]} columns={4} />

          {!isShowingComposite && originalUrl && resultUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Original" resultLabel="HDRMT" accent={ACCENT} />
          )}
        </div>
      )}
    </div>
  );
}
