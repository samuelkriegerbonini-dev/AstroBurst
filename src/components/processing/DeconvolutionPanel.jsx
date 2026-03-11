import { ArrowRight } from "lucide-react";
import { useState, useCallback, useRef, useEffect } from "react";
import { useBackend } from "../../hooks/useBackend";
import { useProgress } from "../../hooks/useProgress";

const SLIDER_CONFIGS = {
  iterations: { min: 1, max: 200, step: 1, default: 20, label: "Iterations" },
  psfSigma: { min: 0.5, max: 10.0, step: 0.1, default: 2.0, label: "PSF Sigma" },
  psfSize: { min: 3, max: 31, step: 2, default: 15, label: "PSF Size" },
  regularization: { min: 0.0, max: 0.1, step: 0.001, default: 0.001, label: "Regularization" },
  deringThreshold: { min: 0.0, max: 1.0, step: 0.01, default: 0.1, label: "Dering Threshold" },
};

function enforceOdd(value) {
  const v = Math.round(value);
  return v % 2 === 0 ? v + 1 : v;
}

export default function DeconvolutionPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom, psfKernel }) {
  const { deconvolveRL } = useBackend();
  const progress = useProgress("deconv-progress");
  const [params, setParams] = useState({
    iterations: SLIDER_CONFIGS.iterations.default,
    psfSigma: SLIDER_CONFIGS.psfSigma.default,
    psfSize: SLIDER_CONFIGS.psfSize.default,
    regularization: SLIDER_CONFIGS.regularization.default,
    deringing: true,
    deringThreshold: SLIDER_CONFIGS.deringThreshold.default,
    useEmpiricalPsf: false,
  });
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState(null);
  const [error, setError] = useState(null);
  const [comparePosition, setComparePosition] = useState(50);
  const [showOriginal, setShowOriginal] = useState(false);
  const compareRef = useRef(null);
  const dragging = useRef(false);

  const updateParam = useCallback((key, value) => {
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
    } catch (err) {
      setError(err.message || String(err));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, outputDir, params, deconvolveRL, progress]);

  const handleMouseDown = useCallback(() => {
    dragging.current = true;
  }, []);

  const handleMouseMove = useCallback((e) => {
    if (!dragging.current || !compareRef.current) return;
    const rect = compareRef.current.getBoundingClientRect();
    const x = ((e.clientX - rect.left) / rect.width) * 100;
    setComparePosition(Math.max(0, Math.min(100, x)));
  }, []);

  const handleMouseUp = useCallback(() => {
    dragging.current = false;
  }, []);

  useEffect(() => {
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, [handleMouseMove, handleMouseUp]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const resultUrl = result?.previewUrl;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      {chainedFrom && (
        <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-indigo-500/8 border border-indigo-500/15 text-[10px] text-indigo-400/80 mb-2">
          <ArrowRight size={10} />
          Using output from <span className="font-medium">{({background: "Background Extraction", denoise: "Wavelet Denoise", deconvolution: "Deconvolution"})[chainedFrom] || chainedFrom}</span>
        </div>
      )}
      <div className="flex items-center gap-2 mb-1">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-indigo-400">
          <circle cx="12" cy="12" r="3" />
          <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
        </svg>
        <span className="text-sm font-semibold text-zinc-200 tracking-wide">
          Richardson-Lucy Deconvolution
        </span>
        <span className="text-[10px] text-zinc-500 ml-auto">FFT-accelerated</span>
      </div>

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">
          Select a FITS file to enable deconvolution.
        </div>
      )}

      <div className="flex flex-col gap-3">
        {Object.entries(SLIDER_CONFIGS).map(([key, cfg]) => {
          if (key === "deringThreshold" && !params.deringing) return null;
          if ((key === "psfSigma" || key === "psfSize") && params.useEmpiricalPsf) return null;
          return (
            <div key={key} className="flex flex-col gap-1">
              <div className="flex justify-between items-center">
                <label className="text-xs text-zinc-400">{cfg.label}</label>
                <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">
                  {key === "regularization"
                    ? params[key].toFixed(3)
                    : key === "psfSigma"
                      ? params[key].toFixed(1)
                      : key === "deringThreshold"
                        ? params[key].toFixed(2)
                        : params[key]}
                </span>
              </div>
              <input
                type="range"
                min={cfg.min}
                max={cfg.max}
                step={cfg.step}
                value={params[key]}
                onChange={(e) => updateParam(key, parseFloat(e.target.value))}
                disabled={isRunning}
                className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-indigo-500
                  [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
                  [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-indigo-400
                  [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:shadow-md"
              />
            </div>
          );
        })}

        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Deringing</label>
          <button
            onClick={() => updateParam("deringing", !params.deringing)}
            disabled={isRunning}
            className={`relative w-9 h-5 rounded-full transition-colors duration-200 ${
              params.deringing ? "bg-indigo-500" : "bg-zinc-600"
            }`}
          >
            <span
              className={`absolute top-0.5 left-0.5 w-4 h-4 bg-white rounded-full shadow transition-transform duration-200 ${
                params.deringing ? "translate-x-4" : "translate-x-0"
              }`}
            />
          </button>
        </div>

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1.5">
            <label className="text-xs text-zinc-400">Empirical PSF</label>
            {psfKernel && (
              <span className="text-[9px] text-emerald-400 bg-emerald-900/30 px-1.5 py-0.5 rounded">ready</span>
            )}
          </div>
          <button
            onClick={() => updateParam("useEmpiricalPsf", !params.useEmpiricalPsf)}
            disabled={isRunning}
            className={`relative w-9 h-5 rounded-full transition-colors duration-200 ${
              params.useEmpiricalPsf ? "bg-violet-500" : "bg-zinc-600"
            }`}
          >
            <span
              className={`absolute top-0.5 left-0.5 w-4 h-4 bg-white rounded-full shadow transition-transform duration-200 ${
                params.useEmpiricalPsf ? "translate-x-4" : "translate-x-0"
              }`}
            />
          </button>
        </div>

        {params.useEmpiricalPsf && !psfKernel && (
          <div className="text-[10px] text-amber-400/80 bg-amber-900/20 border border-amber-800/20 rounded px-2.5 py-1.5">
            Go to PSF tab first to estimate the PSF from stars, then come back here.
          </div>
        )}
      </div>

      <button
        onClick={handleRun}
        disabled={isRunning || !selectedFile}
        className={`w-full py-2.5 rounded-lg text-sm font-medium transition-all duration-200
          ${isRunning
          ? "bg-zinc-700 text-zinc-400 cursor-wait"
          : selectedFile
            ? "bg-indigo-600 hover:bg-indigo-500 text-white shadow-lg shadow-indigo-900/30 active:scale-[0.98]"
            : "bg-zinc-700 text-zinc-500 cursor-not-allowed"
        }`}
      >
        {isRunning ? (
          <span className="flex items-center justify-center gap-2">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            Deconvolving...
          </span>
        ) : (
          "Run Deconvolution"
        )}
      </button>

      {isRunning && progress.active && (
        <div className="flex flex-col gap-1.5">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div
              className="h-full bg-indigo-500 rounded-full transition-all duration-300"
              style={{ width: `${progress.percent}%` }}
            />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span>{progress.percent}%</span>
          </div>
        </div>
      )}

      {error && (
        <div className="text-xs text-red-400 bg-red-900/20 border border-red-800/30 rounded-lg px-3 py-2">
          {error}
        </div>
      )}

      {result && (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-3 gap-2 text-xs">
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">Iterations</div>
              <div className="text-zinc-200 font-mono">{result.iterations_run}</div>
            </div>
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">Convergence</div>
              <div className="text-zinc-200 font-mono">{result.convergence?.toExponential(2)}</div>
            </div>
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">Time</div>
              <div className="text-zinc-200 font-mono">{(result.elapsed_ms / 1000).toFixed(1)}s</div>
            </div>
          </div>

          {result.iterations_run < params.iterations && (
            <div className="text-[10px] text-emerald-400 bg-emerald-900/20 border border-emerald-800/30 rounded-lg px-3 py-1.5">
              Early stop: converged at iteration {result.iterations_run}/{params.iterations}
            </div>
          )}

          {originalUrl && resultUrl && (
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <span className="text-xs text-zinc-400">Before / After</span>
                <button
                  onClick={() => setShowOriginal(!showOriginal)}
                  className="text-xs text-indigo-400 hover:text-indigo-300 transition-colors"
                >
                  {showOriginal ? "Show Result" : "Hold Original"}
                </button>
              </div>

              <div
                ref={compareRef}
                className="relative w-full aspect-square rounded-lg overflow-hidden cursor-col-resize bg-zinc-900 border border-zinc-700/50"
                onMouseDown={handleMouseDown}
              >
                <img
                  src={resultUrl}
                  alt="Deconvolved"
                  className="absolute inset-0 w-full h-full object-contain"
                  draggable={false}
                />
                <div
                  className="absolute inset-0 overflow-hidden"
                  style={{ width: `${comparePosition}%` }}
                >
                  <img
                    src={originalUrl}
                    alt="Original"
                    className="absolute inset-0 w-full h-full object-contain"
                    style={{ width: `${(100 / comparePosition) * 100}%`, maxWidth: "none" }}
                    draggable={false}
                  />
                </div>
                <div
                  className="absolute top-0 bottom-0 w-0.5 bg-white/70 shadow-lg"
                  style={{ left: `${comparePosition}%` }}
                >
                  <div className="absolute top-1/2 -translate-y-1/2 -translate-x-1/2 w-6 h-6 rounded-full bg-white/90 shadow-md flex items-center justify-center">
                    <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                      <path d="M3 6H1M11 6H9M3 6L5 4M3 6L5 8M9 6L7 4M9 6L7 8" stroke="#333" strokeWidth="1.5" strokeLinecap="round" />
                    </svg>
                  </div>
                </div>
                <div className="absolute top-2 left-2 text-[10px] font-medium text-white/60 bg-black/40 px-1.5 py-0.5 rounded">
                  Original
                </div>
                <div className="absolute top-2 right-2 text-[10px] font-medium text-white/60 bg-black/40 px-1.5 py-0.5 rounded">
                  Deconvolved
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
