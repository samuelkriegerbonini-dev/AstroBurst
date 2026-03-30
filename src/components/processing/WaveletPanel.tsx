import { useState, useCallback, useEffect } from "react";
import { waveletDenoise } from "../../services/processing";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types";

const DEFAULT_THRESHOLDS = [3.0, 2.5, 2.0, 1.5, 1.0];
const SCALE_LABELS = ["Fine detail", "Small structures", "Medium structures", "Large structures", "Very large"];

interface WaveletResult {
  previewUrl?: string;
  fits_path?: string;
  scales_processed?: number;
  noise_estimate?: number;
  elapsed_ms?: number;
}

interface WaveletPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | undefined) => void;
  onProcessingDone?: (result: WaveletResult) => void;
  chainedFrom?: string | null;
}

const ICON = (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-sky-400">
    <path d="M2 12c0 0 2-4 5-4s3 8 6 8 5-4 5-4" />
    <path d="M2 12c0 0 2 4 5 4s3-8 6-8 5 4 5 4" opacity="0.3" />
  </svg>
);

export default function WaveletPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }: WaveletPanelProps) {
  const [numScales, setNumScales] = useState(5);
  const [thresholds, setThresholds] = useState<number[]>([...DEFAULT_THRESHOLDS]);
  const [linear, setLinear] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<WaveletResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const updateThreshold = useCallback((idx: number, value: number) => {
    setThresholds((prev) => {
      const next = [...prev];
      next[idx] = value;
      return next;
    });
  }, []);

  useEffect(() => {
    setThresholds((prev) => {
      if (prev.length === numScales) return prev;
      const next: number[] = [];
      for (let i = 0; i < numScales; i++) {
        next.push(prev[i] ?? DEFAULT_THRESHOLDS[i] ?? 1.0);
      }
      return next;
    });
  }, [numScales]);

  const handleRun = useCallback(async () => {
    if (!selectedFile?.path) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await waveletDenoise(selectedFile.path, outputDir, {
        numScales,
        thresholds: thresholds.slice(0, numScales),
        linear,
      });
      setResult(res);
      onPreviewUpdate?.(res?.previewUrl);
      onProcessingDone?.(res);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, outputDir, numScales, thresholds, linear, onPreviewUpdate, onProcessingDone]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;

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
                {SCALE_LABELS[idx] || `Scale ${idx + 1}`}
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

        <Toggle label="Soft threshold (linear)" checked={linear} disabled={isRunning} accent="sky" onChange={setLinear} />
      </div>

      <RunButton label="Run Noise Reduction" runningLabel="Denoising..." running={isRunning} disabled={!selectedFile} accent="sky" onClick={handleRun} />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <ResultGrid items={[
            { label: "Scales", value: result.scales_processed },
            { label: "Noise est.", value: result.noise_estimate?.toExponential(2) },
            { label: "Time", value: `${((result.elapsed_ms ?? 0) / 1000).toFixed(1)}s` },
          ]} />

          {originalUrl && resultUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Original" resultLabel="Denoised" accent="sky" />
          )}
        </div>
      )}
    </div>
  );
}
