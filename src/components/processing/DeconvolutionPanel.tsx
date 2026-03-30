import { useState, useCallback } from "react";
import { deconvolveRL } from "../../services/processing";
import { useProgress } from "../../hooks/useProgress";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import type { ProcessedFile } from "../../shared/types";

function enforceOdd(value: number): number {
  const v = Math.round(value);
  return v % 2 === 0 ? v + 1 : v;
}

interface DeconvResult {
  previewUrl?: string;
  fits_path?: string;
  iterations_run?: number;
  convergence?: number;
  elapsed_ms?: number;
}

interface DeconvParams {
  iterations: number;
  psfSigma: number;
  psfSize: number;
  regularization: number;
  deringing: boolean;
  deringThreshold: number;
  useEmpiricalPsf: boolean;
}

interface DeconvolutionPanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | undefined) => void;
  onProcessingDone?: (result: DeconvResult) => void;
  chainedFrom?: string | null;
  psfKernel?: number[][] | null;
}

const ICON = (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-indigo-400">
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
  </svg>
);

export default function DeconvolutionPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom, psfKernel }: DeconvolutionPanelProps) {
  const progress = useProgress("deconv-progress");
  const [params, setParams] = useState<DeconvParams>({
    iterations: 20,
    psfSigma: 2.0,
    psfSize: 15,
    regularization: 0.001,
    deringing: true,
    deringThreshold: 0.1,
    useEmpiricalPsf: false,
  });
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<DeconvResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const update = useCallback(<K extends keyof DeconvParams>(key: K, value: DeconvParams[K]) => {
    setParams((prev) => ({ ...prev, [key]: value }));
  }, []);

  const handleRun = useCallback(async () => {
    if (!selectedFile?.path) return;
    setIsRunning(true);
    setError(null);
    setResult(null);
    progress.reset();
    try {
      const res = await deconvolveRL(selectedFile.path, outputDir, {
        iterations: params.iterations,
        psfSigma: params.psfSigma,
        psfSize: enforceOdd(params.psfSize),
        regularization: params.regularization,
        deringing: params.deringing,
        deringThreshold: params.deringThreshold,
        useEmpiricalPsf: params.useEmpiricalPsf,
      });
      setResult(res);
      onPreviewUpdate?.(res?.previewUrl);
      onProcessingDone?.(res);
    } catch (err: unknown) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, outputDir, params, progress, onPreviewUpdate, onProcessingDone]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Richardson-Lucy Deconvolution" subtitle="FFT-accelerated" />
      <ChainBanner chainedFrom={chainedFrom} accent="indigo" />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to enable deconvolution.</div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Iterations" value={params.iterations} min={1} max={200} step={1} disabled={isRunning} accent="indigo" onChange={(v) => update("iterations", v)} />

        {!params.useEmpiricalPsf && (
          <>
            <Slider label="PSF Sigma" value={params.psfSigma} min={0.5} max={10} step={0.1} disabled={isRunning} accent="indigo" format={(v) => v.toFixed(1)} onChange={(v) => update("psfSigma", v)} />
            <Slider label="PSF Size" value={params.psfSize} min={3} max={31} step={2} disabled={isRunning} accent="indigo" onChange={(v) => update("psfSize", v)} />
          </>
        )}

        <Slider label="Regularization" value={params.regularization} min={0} max={0.1} step={0.001} disabled={isRunning} accent="indigo" format={(v) => v.toFixed(3)} onChange={(v) => update("regularization", v)} />

        <Toggle label="Deringing" checked={params.deringing} disabled={isRunning} accent="indigo" onChange={(v) => update("deringing", v)} />

        {params.deringing && (
          <Slider label="Dering Threshold" value={params.deringThreshold} min={0} max={1} step={0.01} disabled={isRunning} accent="indigo" format={(v) => v.toFixed(2)} onChange={(v) => update("deringThreshold", v)} />
        )}

        <Toggle label="Empirical PSF" checked={params.useEmpiricalPsf} disabled={isRunning} accent="violet" badge={psfKernel ? "ready" : null} onChange={(v) => update("useEmpiricalPsf", v)} />

        {params.useEmpiricalPsf && !psfKernel && (
          <div className="text-[10px] text-amber-400/80 bg-amber-900/20 border border-amber-800/20 rounded px-2.5 py-1.5">
            Go to PSF tab first to estimate the PSF from stars, then come back here.
          </div>
        )}
      </div>

      <RunButton label="Run Deconvolution" runningLabel="Deconvolving..." running={isRunning} disabled={!selectedFile} accent="indigo" onClick={handleRun} />

      {isRunning && progress.active && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-indigo), #818cf8)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span>{progress.percent}%</span>
          </div>
        </div>
      )}

      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <ResultGrid items={[
            { label: "Iterations", value: result.iterations_run },
            { label: "Convergence", value: result.convergence?.toExponential(2) },
            { label: "Time", value: `${((result.elapsed_ms ?? 0) / 1000).toFixed(1)}s` },
          ]} />

          {result.iterations_run != null && result.iterations_run < params.iterations && (
            <div className="text-[10px] text-emerald-400 bg-emerald-900/20 border border-emerald-800/30 rounded-lg px-3 py-1.5">
              Early stop: converged at iteration {result.iterations_run}/{params.iterations}
            </div>
          )}

          {originalUrl && resultUrl && (
            <CompareView originalUrl={originalUrl} resultUrl={resultUrl} originalLabel="Original" resultLabel="Deconvolved" accent="indigo" />
          )}
        </div>
      )}
    </div>
  );
}
