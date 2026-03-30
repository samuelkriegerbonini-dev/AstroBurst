import { useState, useCallback } from "react";
import { maskedStretch } from "../../services/processing";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types";

interface MaskedStretchResult {
  previewUrl?: string;
  fits_path?: string;
  iterations_run?: number;
  final_background?: number;
  stars_masked?: number;
  mask_coverage?: number;
  converged?: boolean;
  elapsed_ms?: number;
  dimensions?: number[];
}

interface MaskedStretchParams {
  iterations: number;
  targetBackground: number;
  maskGrowth: number;
  maskSoftness: number;
  protectionAmount: number;
  luminanceProtect: boolean;
}

interface MaskedStretchPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: MaskedStretchResult) => void;
  chainedFrom?: string;
}

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-rose-400">
    <path d="M12 3v18M3 12h18" opacity="0.3" />
    <circle cx="12" cy="12" r="9" />
    <circle cx="12" cy="12" r="3" opacity="0.5" />
    <path d="M5.5 5.5l3 3M15.5 15.5l3 3M5.5 18.5l3-3M15.5 8.5l3-3" opacity="0.2" />
  </svg>
);

const BG_PRESETS = [
  { label: "Dark", value: 0.15 },
  { label: "Normal", value: 0.25 },
  { label: "Bright", value: 0.35 },
];

export default function MaskedStretchPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }: MaskedStretchPanelProps) {
  const [params, setParams] = useState<MaskedStretchParams>({
    iterations: 10,
    targetBackground: 0.25,
    maskGrowth: 2.5,
    maskSoftness: 4.0,
    protectionAmount: 0.85,
    luminanceProtect: true,
  });
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<MaskedStretchResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const update = useCallback(<K extends keyof MaskedStretchParams>(key: K, value: MaskedStretchParams[K]) => {
    setParams((prev) => ({ ...prev, [key]: value }));
  }, []);

  const handleRun = useCallback(async () => {
    if (!selectedFile?.path) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await maskedStretch(selectedFile.path, outputDir, {
        iterations: params.iterations,
        targetBackground: params.targetBackground,
        maskGrowth: params.maskGrowth,
        maskSoftness: params.maskSoftness,
        protectionAmount: params.protectionAmount,
        luminanceProtect: params.luminanceProtect,
      });
      setResult(res);
      onPreviewUpdate?.(res?.previewUrl);
      onProcessingDone?.(res);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, outputDir, params, onPreviewUpdate, onProcessingDone]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Masked Stretch" subtitle="Star-protected MTF" />
      <ChainBanner chainedFrom={chainedFrom} accent="rose" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to apply masked stretch.</div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Iterations" value={params.iterations} min={1} max={50} step={1} disabled={isRunning} accent="rose" onChange={(v) => update("iterations", v)} />

        <div className="flex flex-col gap-1">
          <Slider label="Target Background" value={params.targetBackground} min={0.05} max={0.50} step={0.01} disabled={isRunning} accent="rose" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => update("targetBackground", v)} />
          <div className="flex gap-1.5">
            {BG_PRESETS.map((p) => (
              <button
                key={p.label}
                onClick={() => update("targetBackground", p.value)}
                disabled={isRunning}
                className={`flex-1 py-1 rounded-md text-[10px] font-mono transition-all duration-150 ${
                  Math.abs(params.targetBackground - p.value) < 0.01
                    ? "bg-rose-500/20 text-rose-300 ring-1 ring-rose-500/30"
                    : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
        </div>

        <Slider label="Star Protection" value={params.protectionAmount} min={0} max={1} step={0.05} disabled={isRunning} accent="rose" format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => update("protectionAmount", v)} />

        <Slider label="Mask Growth" value={params.maskGrowth} min={1.0} max={5.0} step={0.1} disabled={isRunning} accent="rose" format={(v) => `${v.toFixed(1)}x FWHM`} onChange={(v) => update("maskGrowth", v)} />

        <Slider label="Mask Softness" value={params.maskSoftness} min={0} max={10} step={0.5} disabled={isRunning} accent="rose" format={(v) => `${v.toFixed(1)}px`} onChange={(v) => update("maskSoftness", v)} />

        <Toggle label="Luminance Protection" checked={params.luminanceProtect} disabled={isRunning} accent="rose" onChange={(v) => update("luminanceProtect", v)} />
      </div>

      <RunButton label="Run Masked Stretch" runningLabel="Stretching..." running={isRunning} disabled={!selectedFile} accent="rose" onClick={handleRun} />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <ResultGrid items={[
            { label: "Iterations", value: result.iterations_run },
            { label: "Background", value: result.final_background != null ? `${(result.final_background * 100).toFixed(1)}%` : null },
            { label: "Stars Masked", value: result.stars_masked },
            { label: "Mask Coverage", value: result.mask_coverage != null ? `${(result.mask_coverage * 100).toFixed(1)}%` : null },
            { label: "Converged", value: result.converged ? "Yes" : "No" },
            { label: "Time", value: result.elapsed_ms ? `${(result.elapsed_ms / 1000).toFixed(1)}s` : null },
          ]} />

          {result.converged && (
            <div className="text-[10px] text-emerald-400 bg-emerald-900/20 border border-emerald-800/30 rounded-lg px-3 py-1.5">
              Converged at iteration {result.iterations_run}/{params.iterations}
            </div>
          )}

          {originalUrl && resultUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Original" resultLabel="Masked Stretch" accent="rose" />
          )}
        </div>
      )}
    </div>
  );
}
