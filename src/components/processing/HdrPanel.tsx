import { useState, useCallback } from "react";
import { X } from "lucide-react";
import { applyHdrmt, applyHdrmtComposite } from "../../services/localContrast";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { INPUT_CHANGED_MESSAGE, bustPreviewUrl, isCancelMessage, useProcessingRun } from "../../hooks/useProcessingRun";
import { useCompositeStf } from "../../context/CompositeContext";
import { useRenderContext } from "../../context/PreviewContext";
import { useCompositeChain } from "../../hooks/useCompositeChain";
import { chainHoldsOutput } from "../../utils/processingChain";
import { COMPOSITE_RUN_KEY, compositeChainHolds } from "../../utils/compositeChain";
import { COMPOSITE_RESTARTED_NOTICE, unchangedFileClause } from "./compositeProps";
import type { CompositePanelProps } from "./compositeProps";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types/fits.types";
import { DEFAULT_HDR_CONFIG, HDR_LIMITS, HDR_PROGRESS_EVENT } from "../../shared/types/localContrast";
import type { HdrConfig, LocalContrastResult } from "../../shared/types/localContrast";

interface HdrRun {
  res: LocalContrastResult;
  resultUrl: string | undefined;
  baseUrl: string | null;
  baseLabel: string;
  chainRestarted: boolean;
  layers: number;
  iterations: number;
}

interface HdrPanelProps extends CompositePanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onProcessingDone?: (result: LocalContrastResult) => void;
  chainedFrom?: string;
  inputPreviewUrl?: string | null;
  inputLabel?: string;
  fileKey?: string | null;
  disabledReason?: string | null;
  disabledReasonId?: string;
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

export default function HdrPanel({ selectedFile, outputDir = "./output", onProcessingDone, chainedFrom, inputPreviewUrl, inputLabel, fileKey, compositeMode, compositeInput, onCompositeDone, fileName, disabledReason, disabledReasonId }: HdrPanelProps) {
  const progress = useProgress(HDR_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [config, setConfig] = useState<HdrConfig>(DEFAULT_HDR_CONFIG);
  const { running: isRunning, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<HdrRun>("hdr", compositeMode ? COMPOSITE_RUN_KEY : (fileKey ?? null));
  const { chain } = useRenderContext();
  const compositeChain = useCompositeChain();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const holds = runResult !== null && (compositeMode
    ? compositeChainHolds(compositeChain, "localContrast", runResult.resultUrl)
    : chainHoldsOutput(chain, "localContrast", runResult.res.fits_path));
  const result = holds ? runResult : null;

  const update = useCallback(<K extends keyof HdrConfig>(key: K, value: HdrConfig[K]) => {
    setConfig((prev) => ({ ...prev, [key]: value }));
  }, []);

  const canRun = compositeMode ? compositeInput !== null : !!selectedFile?.path;

  const handleRun = useCallback(() => {
    const runConfig = config;
    const snapshot = { layers: runConfig.layers, iterations: runConfig.iterations };
    if (compositeMode) {
      if (!compositeInput) return;
      const input = compositeInput;
      const displayStf = { r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked };
      resetProgress();
      void run(async () => {
        const res = await applyHdrmtComposite(outputDir, runConfig, { chainInput: input.input, displayStf });
        onCompositeDone("localContrast", "HDRMT", res, displayStf);
        return {
          ...snapshot,
          res,
          resultUrl: bustPreviewUrl(res.previewUrl, Date.now()),
          baseUrl: res.basePreviewUrl ?? input.previewUrl,
          baseLabel: res.basePreviewUrl ? "Composite" : input.label,
          chainRestarted: res.chain_restarted,
        };
      }, isCancelMessage).finally(resetProgress);
      return;
    }
    const path = selectedFile?.path;
    if (!path) return;
    const baseUrl = inputPreviewUrl ?? null;
    const baseLabel = inputLabel ?? "Original";
    resetProgress();
    void run(async (ctx) => {
      const res = await applyHdrmt(path, outputDir, runConfig);
      if (!ctx.inputUnchanged("localContrast")) throw new Error(INPUT_CHANGED_MESSAGE);
      onProcessingDone?.(res);
      return { ...snapshot, res, resultUrl: bustPreviewUrl(res.previewUrl, Date.now()), baseUrl, baseLabel, chainRestarted: false };
    }, isCancelMessage).finally(resetProgress);
  }, [compositeMode, compositeInput, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, selectedFile?.path, outputDir, config, inputPreviewUrl, inputLabel, resetProgress, run, onProcessingDone, onCompositeDone]);

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="HDR Multiscale Transform" subtitle="Large-scale range compression" />
      <ChainBanner chainedFrom={chainedFrom} accent={ACCENT} />

      {!canRun && (
        <div className="text-xs text-zinc-500 italic px-1">Select a stretched FITS file or show a stretched composite.</div>
      )}
      {compositeMode && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          Composite mode: compresses the stretched RGB composite{config.toLightness ? " on its luminance, preserving hue" : " channel by channel"}{unchangedFileClause(fileName)}
        </div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Layers" value={config.layers} min={HDR_LIMITS.layers.min} max={HDR_LIMITS.layers.max} step={HDR_LIMITS.layers.step} disabled={isRunning} accent={ACCENT} hint="fewer = stronger" format={(v) => `${Math.round(v)}`} onChange={(v) => update("layers", Math.round(v))} />
        <Slider label="Iterations" value={config.iterations} min={HDR_LIMITS.iterations.min} max={HDR_LIMITS.iterations.max} step={HDR_LIMITS.iterations.step} disabled={isRunning} accent={ACCENT} format={(v) => `${Math.round(v)}`} onChange={(v) => update("iterations", Math.round(v))} />
        <Slider label="Overdrive" value={config.overdrive} min={HDR_LIMITS.overdrive.min} max={HDR_LIMITS.overdrive.max} step={HDR_LIMITS.overdrive.step} disabled={isRunning} accent={ACCENT} format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => update("overdrive", v)} />

        <Toggle label="Invert (compress dark structures)" checked={config.inverted} disabled={isRunning} accent={ACCENT} onChange={(v) => update("inverted", v)} />
        {compositeMode && (
          <Toggle label="Apply to Lightness" checked={config.toLightness} disabled={isRunning} accent={ACCENT} onChange={(v) => update("toLightness", v)} />
        )}
        <Toggle label="Deringing (protect star cores)" checked={config.deringing} disabled={isRunning} accent={ACCENT} onChange={(v) => update("deringing", v)} />
        {config.deringing && (
          <Slider label="Deringing Amount" value={config.deringingAmount} min={HDR_LIMITS.deringingAmount.min} max={HDR_LIMITS.deringingAmount.max} step={HDR_LIMITS.deringingAmount.step} disabled={isRunning} accent={ACCENT} format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => update("deringingAmount", v)} />
        )}
      </div>

      <div title={disabledReason ?? busyTitle}>
        <RunButton label="Run HDRMT" runningLabel="Compressing..." running={isRunning} disabled={!canRun || blocked || !!disabledReason} describedBy={disabledReason ? disabledReasonId : undefined} accent={ACCENT} onClick={handleRun} />
      </div>

      {isRunning && progress.active && (
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
          {result.chainRestarted && (
            <div className="text-[10px] text-amber-300 bg-amber-900/20 border border-amber-800/30 rounded-lg px-3 py-1.5">
              {COMPOSITE_RESTARTED_NOTICE}
            </div>
          )}
          <ResultGrid items={[
            { label: "Layers", value: result.layers },
            { label: "Iterations", value: result.iterations },
            { label: "Time", value: result.res.elapsed_ms != null ? `${(result.res.elapsed_ms / 1000).toFixed(2)}s` : null },
            { label: "Size", value: result.res.dimensions ? `${result.res.dimensions[0]}x${result.res.dimensions[1]}` : null },
          ]} columns={4} />

          {result.baseUrl && result.resultUrl && (
            <CompareView originalUrl={result.baseUrl} resultUrl={result.resultUrl} originalLabel={result.baseLabel} resultLabel="HDRMT" accent={ACCENT} />
          )}
        </div>
      )}
    </div>
  );
}
