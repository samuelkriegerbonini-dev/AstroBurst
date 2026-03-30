import { useState, useCallback } from "react";
import { applyArcsinhStretch } from "../../services/processing";
import { Slider, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";

interface ArcsinhStretchPanelProps {
  selectedFile: { path: string; result?: any } | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: any) => void;
  chainedFrom?: string;
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

export default function ArcsinhStretchPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }: ArcsinhStretchPanelProps) {
  const [factor, setFactor] = useState(50.0);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);

  const logMin = linearToLog(FACTOR_MIN);
  const logMax = linearToLog(FACTOR_MAX);
  const logValue = linearToLog(factor);

  const handleSliderChange = useCallback((v: number) => {
    setFactor(Math.round(logToLinear(v) * 10) / 10);
  }, []);

  const handleRun = useCallback(async () => {
    if (!selectedFile?.path) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await applyArcsinhStretch(selectedFile.path, outputDir, factor);
      setResult(res);
      onPreviewUpdate?.(res?.previewUrl);
      onProcessingDone?.(res);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile?.path, factor, outputDir, onPreviewUpdate, onProcessingDone]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const stretchedUrl = result?.previewUrl;

  return (
    <div className="flex flex-col gap-3 p-4">
      <SectionHeader icon={ICON} title="Arcsinh Stretch" subtitle="arcsinh(I*S)/arcsinh(S)" />
      <ChainBanner chainedFrom={chainedFrom} accent="amber" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to apply stretch.</div>
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

      <RunButton label={`Apply Stretch (S=${factor.toFixed(0)})`} runningLabel="Stretching..." running={isRunning} disabled={!selectedFile} accent="amber" onClick={handleRun} />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in">
          <ResultGrid items={[
            { label: "Factor", value: result.stretch_factor?.toFixed(1) },
            { label: "Time", value: result.elapsed_ms ? `${(result.elapsed_ms / 1000).toFixed(2)}s` : null },
            { label: "Size", value: result.dimensions ? `${result.dimensions[0]}x${result.dimensions[1]}` : null },
          ]} />

          {originalUrl && stretchedUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={stretchedUrl} originalLabel="Original" resultLabel="Stretched" accent="amber" height={180} />
          )}
        </div>
      )}
    </div>
  );
}
