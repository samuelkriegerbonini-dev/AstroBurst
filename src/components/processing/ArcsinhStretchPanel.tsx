import { useState, useCallback } from "react";
import { applyArcsinhStretch, arcsinhStretchComposite } from "../../services/processing";
import type { ArcsinhResult } from "../../shared/types";
import { useRenderContext } from "../../context/PreviewContext";
import { useCompositeStf } from "../../context/CompositeContext";
import { useCompositeChain } from "../../hooks/useCompositeChain";
import { chainHoldsOutput } from "../../utils/processingChain";
import { COMPOSITE_RUN_KEY, compositeChainHolds } from "../../utils/compositeChain";
import { INPUT_CHANGED_MESSAGE, bustPreviewUrl, useProcessingRun } from "../../hooks/useProcessingRun";
import { COMPOSITE_RESTARTED_NOTICE, compositeModeNotice } from "./compositeProps";
import type { CompositePanelProps } from "./compositeProps";
import { Slider, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";

interface ArcsinhRun {
  res: ArcsinhResult;
  resultUrl: string | undefined;
  baseUrl: string | null;
  baseLabel: string;
  chainRestarted: boolean;
}

interface ArcsinhStretchPanelProps extends CompositePanelProps {
  selectedFile: { path: string; result?: unknown } | null;
  outputDir?: string;
  onProcessingDone?: (result: ArcsinhResult) => void;
  chainedFrom?: string;
  inputPreviewUrl?: string | null;
  inputLabel?: string;
  fileKey?: string | null;
  disabledReason?: string | null;
  disabledReasonId?: string;
}

const FACTOR_MIN = 1;
const FACTOR_MAX = 500;

function linearToLog(val: number): number {
  return Math.log(Math.max(val, FACTOR_MIN));
}

function logToLinear(log: number): number {
  return Math.exp(log);
}

const PRESETS = [5, 20, 50, 100, 200];

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-amber-400">
    <path d="M4 20 C8 4, 16 4, 20 20" />
  </svg>
);

export default function ArcsinhStretchPanel({ selectedFile, outputDir = "./output", onProcessingDone, chainedFrom, inputPreviewUrl, inputLabel, fileKey, compositeMode, compositeInput, onCompositeDone, fileName, disabledReason, disabledReasonId }: ArcsinhStretchPanelProps) {
  const [factor, setFactor] = useState(50.0);
  const { running: isRunning, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<ArcsinhRun>("stretch", compositeMode ? COMPOSITE_RUN_KEY : (fileKey ?? null));
  const { chain } = useRenderContext();
  const compositeChain = useCompositeChain();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const holds = runResult !== null && (compositeMode
    ? compositeChainHolds(compositeChain, "stretch", runResult.resultUrl)
    : chainHoldsOutput(chain, "stretch", runResult.res.fits_path));
  const result = holds ? runResult : null;
  const canRun = compositeMode ? compositeInput !== null : !!selectedFile?.path;

  const logMin = linearToLog(FACTOR_MIN);
  const logMax = linearToLog(FACTOR_MAX);
  const logValue = linearToLog(factor);

  const handleSliderChange = useCallback((v: number) => {
    setFactor(Math.round(logToLinear(v) * 10) / 10);
  }, []);

  const handleRun = useCallback(() => {
    if (compositeMode) {
      if (!compositeInput) return;
      const input = compositeInput;
      const displayStf = { r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked };
      void run(async () => {
        const res = await arcsinhStretchComposite(factor, outputDir, { chainInput: input.input, displayStf });
        onCompositeDone("stretch", "Stretch", res, displayStf);
        return {
          res,
          resultUrl: bustPreviewUrl(res.previewUrl, Date.now()),
          baseUrl: res.basePreviewUrl ?? input.previewUrl,
          baseLabel: res.basePreviewUrl ? "Composite" : input.label,
          chainRestarted: res.chain_restarted,
        };
      });
      return;
    }
    if (!selectedFile?.path) return;
    const path = selectedFile.path;
    const baseUrl = inputPreviewUrl ?? null;
    const baseLabel = inputLabel ?? "Original";
    void run(async (ctx) => {
      const res = await applyArcsinhStretch(path, outputDir, factor);
      if (!ctx.inputUnchanged("stretch")) throw new Error(INPUT_CHANGED_MESSAGE);
      onProcessingDone?.(res);
      return { res, resultUrl: bustPreviewUrl(res?.previewUrl, Date.now()), baseUrl, baseLabel, chainRestarted: false };
    });
  }, [compositeMode, compositeInput, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, selectedFile?.path, factor, outputDir, run, inputPreviewUrl, inputLabel, onProcessingDone, onCompositeDone]);

  return (
    <div className="flex flex-col gap-3 p-4">
      <SectionHeader icon={ICON} title="Arcsinh Stretch" subtitle="arcsinh(I*S)/arcsinh(S)" />
      <ChainBanner chainedFrom={chainedFrom} accent="amber" />

      {!selectedFile && !compositeMode && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to apply stretch.</div>
      )}
      {compositeMode && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          {compositeModeNotice(fileName)}
        </div>
      )}

      <div className="flex flex-col gap-1">
        <Slider
          label="Stretch Factor (S)"
          value={logValue}
          min={logMin}
          max={logMax}
          step={0.01}
          disabled={isRunning}
          accent="amber"
          format={() => factor.toFixed(1)}
          onChange={handleSliderChange}
        />
        <div className="flex justify-between text-[9px] text-zinc-600 font-mono px-0.5">
          <span>1.0 (linear)</span>
          <span>500 (strong)</span>
        </div>
      </div>

      <div className="flex gap-1.5">
        {PRESETS.map((preset) => (
          <button
            key={preset}
            onClick={() => setFactor(preset)}
            disabled={isRunning}
            className={`flex-1 py-1.5 rounded-md text-[10px] font-mono transition-all duration-150 ${
              Math.abs(factor - preset) < 0.5
                ? "bg-amber-500/20 text-amber-300 ring-1 ring-amber-500/30"
                : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
            }`}
          >
            {preset}
          </button>
        ))}
      </div>

      <div title={disabledReason ?? busyTitle}>
        <RunButton label={`Apply Stretch (S=${factor.toFixed(0)})`} runningLabel="Stretching..." running={isRunning} disabled={!canRun || blocked || !!disabledReason} describedBy={disabledReason ? disabledReasonId : undefined} accent="amber" onClick={handleRun} />
      </div>
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in">
          {result.chainRestarted && (
            <div className="text-[10px] text-amber-300 bg-amber-900/20 border border-amber-800/30 rounded-lg px-3 py-1.5">
              {COMPOSITE_RESTARTED_NOTICE}
            </div>
          )}
          <ResultGrid items={[
            { label: "Factor", value: result.res.stretch_factor?.toFixed(1) },
            { label: "Time", value: result.res.elapsed_ms ? `${(result.res.elapsed_ms / 1000).toFixed(2)}s` : null },
            { label: "Size", value: result.res.dimensions ? `${result.res.dimensions[0]}x${result.res.dimensions[1]}` : null },
          ]} />

          {result.baseUrl && result.resultUrl && (
            <CompareView originalUrl={result.baseUrl} resultUrl={result.resultUrl} originalLabel={result.baseLabel} resultLabel="Stretched" accent="amber" height={180} />
          )}
        </div>
      )}
    </div>
  );
}
