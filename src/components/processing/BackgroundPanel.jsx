import { useState, useCallback, useRef, useEffect } from "react";
import { useBackend } from "../../hooks/useBackend";
import { ArrowRight } from "lucide-react";

const SLIDER_CONFIGS = {
  gridSize: { min: 3, max: 24, step: 1, default: 8, label: "Grid Size" },
  polyDegree: { min: 1, max: 5, step: 1, default: 3, label: "Polynomial Degree" },
  sigmaClip: { min: 1.0, max: 5.0, step: 0.1, default: 2.5, label: "Sigma Clip" },
  iterations: { min: 1, max: 10, step: 1, default: 3, label: "Iterations" },
};

function ChainBanner({ chainedFrom }) {
  if (!chainedFrom) return null;
  const labels = { background: "Background Extraction", denoise: "Wavelet Denoise", deconvolution: "Deconvolution" };
  return (
    <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-emerald-500/8 border border-emerald-500/15 text-[10px] text-emerald-400/80 mb-2">
      <ArrowRight size={10} />
      Using output from <span className="font-medium">{labels[chainedFrom] || chainedFrom}</span>
    </div>
  );
}

export default function BackgroundPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }) {
  const { extractBackground } = useBackend();
  const [params, setParams] = useState({
    gridSize: SLIDER_CONFIGS.gridSize.default,
    polyDegree: SLIDER_CONFIGS.polyDegree.default,
    sigmaClip: SLIDER_CONFIGS.sigmaClip.default,
    iterations: SLIDER_CONFIGS.iterations.default,
    mode: "subtract",
  });
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState(null);
  const [error, setError] = useState(null);
  const [showModel, setShowModel] = useState(false);

  const updateParam = useCallback((key, value) => {
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
    } catch (err) {
      setError(err.message || String(err));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, outputDir, params, extractBackground, onPreviewUpdate, onProcessingDone]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const correctedUrl = result?.previewUrl;
  const modelUrl = result?.modelUrl;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <div className="flex items-center gap-2 mb-1">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-emerald-400">
          <rect x="3" y="3" width="18" height="18" rx="2" />
          <path d="M3 15h18M3 9h18" opacity="0.3" />
          <path d="M9 3v18M15 3v18" opacity="0.3" />
        </svg>
        <span className="text-sm font-semibold text-zinc-200 tracking-wide">
          Background Extraction
        </span>
      </div>

      <ChainBanner chainedFrom={chainedFrom} />

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">
          Select a FITS file to enable background extraction.
        </div>
      )}

      <div className="flex flex-col gap-3">
        {Object.entries(SLIDER_CONFIGS).map(([key, cfg]) => (
          <div key={key} className="flex flex-col gap-1">
            <div className="flex justify-between items-center">
              <label className="text-xs text-zinc-400">{cfg.label}</label>
              <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">
                {key === "sigmaClip" ? params[key].toFixed(1) : params[key]}
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
              className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-emerald-500
                [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
                [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-emerald-400
                [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:shadow-md"
            />
          </div>
        ))}

        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Mode</label>
          <select
            value={params.mode}
            onChange={(e) => updateParam("mode", e.target.value)}
            disabled={isRunning}
            className="bg-zinc-800 border border-zinc-700 rounded px-2 py-1 text-xs text-zinc-300 outline-none"
          >
            <option value="subtract">Subtract</option>
            <option value="divide">Divide</option>
          </select>
        </div>
      </div>

      <button
        onClick={handleRun}
        disabled={isRunning || !selectedFile}
        className={`w-full py-2.5 rounded-lg text-sm font-medium transition-all duration-200
          ${isRunning
          ? "bg-zinc-700 text-zinc-400 cursor-wait"
          : selectedFile
            ? "bg-emerald-600 hover:bg-emerald-500 text-white shadow-lg shadow-emerald-900/30 active:scale-[0.98]"
            : "bg-zinc-700 text-zinc-500 cursor-not-allowed"
        }`}
      >
        {isRunning ? (
          <span className="flex items-center justify-center gap-2">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            Extracting...
          </span>
        ) : (
          "Extract Background"
        )}
      </button>

      {error && (
        <div className="text-xs text-red-400 bg-red-900/20 border border-red-800/30 rounded-lg px-3 py-2">
          {error}
        </div>
      )}

      {result && (
        <div className="flex flex-col gap-3">
          <div className="grid grid-cols-3 gap-2 text-xs">
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">Samples</div>
              <div className="text-zinc-200 font-mono">{result.sample_count}</div>
            </div>
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">RMS</div>
              <div className="text-zinc-200 font-mono">{result.rms_residual?.toExponential(2)}</div>
            </div>
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">Time</div>
              <div className="text-zinc-200 font-mono">{(result.elapsed_ms / 1000).toFixed(1)}s</div>
            </div>
          </div>

          {(correctedUrl || modelUrl) && (
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <button
                  onClick={() => setShowModel(false)}
                  className={`text-xs px-2 py-1 rounded transition-colors ${
                    !showModel ? "bg-emerald-600 text-white" : "bg-zinc-700 text-zinc-400 hover:text-zinc-200"
                  }`}
                >
                  Corrected
                </button>
                <button
                  onClick={() => setShowModel(true)}
                  className={`text-xs px-2 py-1 rounded transition-colors ${
                    showModel ? "bg-emerald-600 text-white" : "bg-zinc-700 text-zinc-400 hover:text-zinc-200"
                  }`}
                >
                  Model
                </button>
              </div>
              <div className="relative w-full aspect-square rounded-lg overflow-hidden bg-zinc-900 border border-zinc-700/50">
                <img
                  src={showModel ? modelUrl : correctedUrl}
                  alt={showModel ? "Background Model" : "Corrected"}
                  className="absolute inset-0 w-full h-full object-contain"
                  draggable={false}
                />
                <div className="absolute top-2 left-2 text-[10px] font-medium text-white/60 bg-black/40 px-1.5 py-0.5 rounded">
                  {showModel ? "Background Model" : "Background Removed"}
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
