import { useState, useCallback } from "react";
import { extractBackground } from "../../services/processing.service";
import { Slider, Toggle, RunButton, ResultGrid, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types";

interface BackgroundResult {
  previewUrl?: string;
  modelUrl?: string;
  corrected_fits?: string;
  sample_count?: number;
  rms_residual?: number;
  elapsed_ms?: number;
}

interface BackgroundParams {
  gridSize: number;
  polyDegree: number;
  sigmaClip: number;
  iterations: number;
  mode: string;
}

interface BackgroundPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | undefined) => void;
  onProcessingDone?: (result: BackgroundResult) => void;
  chainedFrom?: string | null;
}

const ICON = (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-emerald-400">
    <rect x="3" y="3" width="18" height="18" rx="2" />
    <path d="M3 15h18M3 9h18" opacity="0.3" />
    <path d="M9 3v18M15 3v18" opacity="0.3" />
  </svg>
);

export default function BackgroundPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }: BackgroundPanelProps) {
  const [params, setParams] = useState<BackgroundParams>({
    gridSize: 8,
    polyDegree: 3,
    sigmaClip: 2.5,
    iterations: 3,
    mode: "subtract",
  });
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<BackgroundResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showModel, setShowModel] = useState(false);

  const update = useCallback(<K extends keyof BackgroundParams>(key: K, value: BackgroundParams[K]) => {
    setParams((prev) => ({ ...prev, [key]: value }));
  }, []);

  const handleRun = useCallback(async () => {
    if (!selectedFile?.path) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    try {
      const res = await extractBackground(selectedFile.path, outputDir, {
        gridSize: params.gridSize,
        polyDegree: params.polyDegree,
        sigmaClip: params.sigmaClip,
        iterations: params.iterations,
        mode: params.mode,
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

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Background Extraction" />
      <ChainBanner chainedFrom={chainedFrom} accent="emerald" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to enable background extraction.</div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Grid Size" value={params.gridSize} min={3} max={24} step={1} disabled={isRunning} accent="emerald" onChange={(v) => update("gridSize", v)} />
        <Slider label="Polynomial Degree" value={params.polyDegree} min={1} max={5} step={1} disabled={isRunning} accent="emerald" onChange={(v) => update("polyDegree", v)} />
        <Slider label="Sigma Clip" value={params.sigmaClip} min={1} max={5} step={0.1} disabled={isRunning} accent="emerald" format={(v) => v.toFixed(1)} onChange={(v) => update("sigmaClip", v)} />
        <Slider label="Iterations" value={params.iterations} min={1} max={10} step={1} disabled={isRunning} accent="emerald" onChange={(v) => update("iterations", v)} />

        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Mode</label>
          <select value={params.mode} onChange={(e) => update("mode", e.target.value)} disabled={isRunning} className="ab-select">
            <option value="subtract">Subtract</option>
            <option value="divide">Divide</option>
          </select>
        </div>
      </div>

      <RunButton label="Extract Background" runningLabel="Extracting..." running={isRunning} disabled={!selectedFile} accent="emerald" onClick={handleRun} />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <ResultGrid items={[
            { label: "Samples", value: result.sample_count },
            { label: "RMS", value: result.rms_residual?.toExponential(2) },
            { label: "Time", value: `${((result.elapsed_ms ?? 0) / 1000).toFixed(1)}s` },
          ]} />

          {(result.previewUrl || result.modelUrl) && (
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <button onClick={() => setShowModel(false)} className={`text-xs px-2.5 py-1 rounded-md transition-all ${!showModel ? "bg-emerald-600/20 text-emerald-300 ring-1 ring-emerald-500/30" : "text-zinc-500 hover:text-zinc-300"}`}>
                  Corrected
                </button>
                <button onClick={() => setShowModel(true)} className={`text-xs px-2.5 py-1 rounded-md transition-all ${showModel ? "bg-emerald-600/20 text-emerald-300 ring-1 ring-emerald-500/30" : "text-zinc-500 hover:text-zinc-300"}`}>
                  Model
                </button>
              </div>
              <div className="relative w-full aspect-square rounded-lg overflow-hidden bg-zinc-900 border border-zinc-700/50">
                <img src={showModel ? result.modelUrl : result.previewUrl} alt={showModel ? "Background Model" : "Corrected"} className="absolute inset-0 w-full h-full object-contain" draggable={false} />
                <div className="ab-compare-label left-2">{showModel ? "Background Model" : "Background Removed"}</div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
