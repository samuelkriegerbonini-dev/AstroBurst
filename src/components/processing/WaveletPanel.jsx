import { ArrowRight } from "lucide-react";
import { useState, useCallback, useRef, useEffect } from "react";
import { useBackend } from "../../hooks/useBackend";

const DEFAULT_THRESHOLDS = [3.0, 2.5, 2.0, 1.5, 1.0];
const SCALE_LABELS = ["Fine detail", "Small structures", "Medium structures", "Large structures", "Very large"];

export default function WaveletPanel({ selectedFile, outputDir = "./output", onPreviewUpdate, onProcessingDone, chainedFrom }) {
  const { waveletDenoise } = useBackend();
  const [numScales, setNumScales] = useState(5);
  const [thresholds, setThresholds] = useState([...DEFAULT_THRESHOLDS]);
  const [linear, setLinear] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState(null);
  const [error, setError] = useState(null);
  const [comparePosition, setComparePosition] = useState(50);
  const compareRef = useRef(null);
  const dragging = useRef(false);

  const updateThreshold = useCallback((idx, value) => {
    setThresholds((prev) => {
      const next = [...prev];
      next[idx] = value;
      return next;
    });
  }, []);

  useEffect(() => {
    setThresholds((prev) => {
      if (prev.length === numScales) return prev;
      const next = [];
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
    } catch (err) {
      setError(err.message || String(err));
    } finally {
      setIsRunning(false);
    }
  }, [selectedFile, outputDir, numScales, thresholds, linear, waveletDenoise]);

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
        <div className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-sky-500/8 border border-sky-500/15 text-[10px] text-sky-400/80 mb-2">
          <ArrowRight size={10} />
          Using output from <span className="font-medium">{({background: "Background Extraction", denoise: "Wavelet Denoise", deconvolution: "Deconvolution"})[chainedFrom] || chainedFrom}</span>
        </div>
      )}
      <div className="flex items-center gap-2 mb-1">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-sky-400">
          <path d="M2 12c0 0 2-4 5-4s3 8 6 8 5-4 5-4" />
          <path d="M2 12c0 0 2 4 5 4s3-8 6-8 5 4 5 4" opacity="0.3" />
        </svg>
        <span className="text-sm font-semibold text-zinc-200 tracking-wide">
          Wavelet Noise Reduction
        </span>
      </div>

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">
          Select a FITS file to enable noise reduction.
        </div>
      )}

      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-1">
          <div className="flex justify-between items-center">
            <label className="text-xs text-zinc-400">Scales</label>
            <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">
              {numScales}
            </span>
          </div>
          <input
            type="range"
            min={2}
            max={8}
            step={1}
            value={numScales}
            onChange={(e) => setNumScales(parseInt(e.target.value))}
            disabled={isRunning}
            className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-sky-500
              [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
              [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-sky-400
              [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:shadow-md"
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <label className="text-xs text-zinc-400">Threshold per scale (sigma)</label>
          {thresholds.slice(0, numScales).map((val, idx) => (
            <div key={idx} className="flex items-center gap-2">
              <span className="text-[10px] text-zinc-500 w-24 truncate">
                {SCALE_LABELS[idx] || `Scale ${idx + 1}`}
              </span>
              <input
                type="range"
                min={0}
                max={5}
                step={0.1}
                value={val}
                onChange={(e) => updateThreshold(idx, parseFloat(e.target.value))}
                disabled={isRunning}
                className="flex-1 h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-sky-500
                  [&::-webkit-slider-thumb]:w-2.5 [&::-webkit-slider-thumb]:h-2.5
                  [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-sky-400
                  [&::-webkit-slider-thumb]:appearance-none"
              />
              <span className="text-[10px] font-mono text-zinc-300 w-6 text-right">
                {val.toFixed(1)}
              </span>
            </div>
          ))}
        </div>

        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Soft threshold (linear)</label>
          <button
            onClick={() => setLinear(!linear)}
            disabled={isRunning}
            className={`relative w-9 h-5 rounded-full transition-colors duration-200 ${
              linear ? "bg-sky-500" : "bg-zinc-600"
            }`}
          >
            <span
              className={`absolute top-0.5 left-0.5 w-4 h-4 bg-white rounded-full shadow transition-transform duration-200 ${
                linear ? "translate-x-4" : "translate-x-0"
              }`}
            />
          </button>
        </div>
      </div>

      <button
        onClick={handleRun}
        disabled={isRunning || !selectedFile}
        className={`w-full py-2.5 rounded-lg text-sm font-medium transition-all duration-200
          ${isRunning
          ? "bg-zinc-700 text-zinc-400 cursor-wait"
          : selectedFile
            ? "bg-sky-600 hover:bg-sky-500 text-white shadow-lg shadow-sky-900/30 active:scale-[0.98]"
            : "bg-zinc-700 text-zinc-500 cursor-not-allowed"
        }`}
      >
        {isRunning ? (
          <span className="flex items-center justify-center gap-2">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            Denoising...
          </span>
        ) : (
          "Run Noise Reduction"
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
              <div className="text-zinc-500">Scales</div>
              <div className="text-zinc-200 font-mono">{result.scales_processed}</div>
            </div>
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">Noise est.</div>
              <div className="text-zinc-200 font-mono">{result.noise_estimate?.toExponential(2)}</div>
            </div>
            <div className="bg-zinc-800/60 rounded-lg px-2 py-1.5 text-center">
              <div className="text-zinc-500">Time</div>
              <div className="text-zinc-200 font-mono">{(result.elapsed_ms / 1000).toFixed(1)}s</div>
            </div>
          </div>

          {originalUrl && resultUrl && (
            <div className="flex flex-col gap-2">
              <span className="text-xs text-zinc-400">Before / After</span>
              <div
                ref={compareRef}
                className="relative w-full aspect-square rounded-lg overflow-hidden cursor-col-resize bg-zinc-900 border border-zinc-700/50"
                onMouseDown={handleMouseDown}
              >
                <img
                  src={resultUrl}
                  alt="Denoised"
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
                    style={{
                      width: compareRef.current ? `${compareRef.current.offsetWidth}px` : "100%",
                      maxWidth: "none",
                    }}
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
                  Denoised
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
