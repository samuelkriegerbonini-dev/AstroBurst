import { useState, useCallback, useRef, useEffect } from "react";
import { useBackend } from "../../hooks/useBackend";

interface ArcsinhStretchPanelProps {
  selectedFile: { path: string; result?: any } | null;
  outputDir?: string;
  onPreviewUpdate?: (url: string | null | undefined) => void;
  onProcessingDone?: (result: any) => void;
  chainedFrom?: string;
}

const FACTOR_MIN = 1;
const FACTOR_MAX = 500;

function logToLinear(log: number): number {
  return Math.exp(log);
}

function linearToLog(val: number): number {
  return Math.log(Math.max(val, FACTOR_MIN));
}

export default function ArcsinhStretchPanel({
  selectedFile,
  outputDir = "./output",
  onPreviewUpdate,
  onProcessingDone,
  chainedFrom,
}: ArcsinhStretchPanelProps) {
  const { applyArcsinhStretch } = useBackend();
  const [factor, setFactor] = useState(50.0);
  const [isRunning, setIsRunning] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const [comparePosition, setComparePosition] = useState(50);
  const [showOriginal, setShowOriginal] = useState(false);
  const compareRef = useRef<HTMLDivElement>(null);
  const dragging = useRef(false);

  const logMin = linearToLog(FACTOR_MIN);
  const logMax = linearToLog(FACTOR_MAX);
  const logValue = linearToLog(factor);

  const handleSliderChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
    const logVal = parseFloat(e.target.value);
    const linearVal = logToLinear(logVal);
    setFactor(Math.round(linearVal * 10) / 10);
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
  }, [selectedFile?.path, factor, outputDir, applyArcsinhStretch, onPreviewUpdate, onProcessingDone]);

  const handleCompareMove = useCallback((e: React.MouseEvent | MouseEvent) => {
    if (!compareRef.current || !dragging.current) return;
    const rect = compareRef.current.getBoundingClientRect();
    const pct = Math.max(0, Math.min(100, ((e.clientX - rect.left) / rect.width) * 100));
    setComparePosition(pct);
  }, []);

  useEffect(() => {
    const up = () => { dragging.current = false; };
    window.addEventListener("mouseup", up);
    window.addEventListener("mousemove", handleCompareMove as any);
    return () => {
      window.removeEventListener("mouseup", up);
      window.removeEventListener("mousemove", handleCompareMove as any);
    };
  }, [handleCompareMove]);

  const originalUrl = selectedFile?.result?.previewUrl;
  const stretchedUrl = result?.previewUrl;
  const canCompare = originalUrl && stretchedUrl;

  return (
    <div className="flex flex-col gap-3 p-4">
      <div className="flex items-center gap-2 mb-1">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-amber-400">
          <path d="M4 20 C8 4, 16 4, 20 20" />
        </svg>
        <span className="text-sm font-semibold text-zinc-200 tracking-wide">
          Arcsinh Stretch
        </span>
        <span className="text-[10px] text-zinc-500 ml-auto">arcsinh(I*S)/arcsinh(S)</span>
      </div>

      {chainedFrom && (
        <div className="text-[10px] text-zinc-600 font-mono px-1">
          Input: {chainedFrom} output
        </div>
      )}

      {!selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">
          Select a FITS file to apply stretch.
        </div>
      )}

      <div className="flex flex-col gap-1">
        <div className="flex justify-between items-center">
          <label className="text-xs text-zinc-400">Stretch Factor (S)</label>
          <span className="text-xs font-mono text-zinc-300 bg-zinc-800 px-1.5 py-0.5 rounded">
            {factor.toFixed(1)}
          </span>
        </div>
        <input
          type="range"
          min={logMin}
          max={logMax}
          step={0.01}
          value={logValue}
          onChange={handleSliderChange}
          disabled={isRunning}
          className="w-full h-1.5 bg-zinc-700 rounded-full appearance-none cursor-pointer accent-amber-500
            [&::-webkit-slider-thumb]:w-3 [&::-webkit-slider-thumb]:h-3
            [&::-webkit-slider-thumb]:rounded-full [&::-webkit-slider-thumb]:bg-amber-400
            [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:shadow-md"
        />
        <div className="flex justify-between text-[9px] text-zinc-600 font-mono px-0.5">
          <span>1.0 (linear)</span>
          <span>500 (strong)</span>
        </div>
      </div>

      <div className="flex gap-1.5">
        {[5, 20, 50, 100, 200].map((preset) => (
          <button
            key={preset}
            onClick={() => setFactor(preset)}
            disabled={isRunning}
            className={`flex-1 py-1 rounded text-[10px] font-mono transition-all duration-150 ${
              Math.abs(factor - preset) < 0.5
                ? "bg-amber-500/20 text-amber-300 ring-1 ring-amber-500/30"
                : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
            }`}
          >
            {preset}
          </button>
        ))}
      </div>

      <button
        onClick={handleRun}
        disabled={isRunning || !selectedFile}
        className={`w-full py-2.5 rounded-lg text-sm font-medium transition-all duration-200 ${
          isRunning
            ? "bg-zinc-700 text-zinc-400 cursor-wait"
            : selectedFile
              ? "bg-amber-600 hover:bg-amber-500 text-white shadow-lg shadow-amber-900/30 active:scale-[0.98]"
              : "bg-zinc-700 text-zinc-500 cursor-not-allowed"
        }`}
      >
        {isRunning ? (
          <span className="flex items-center justify-center gap-2">
            <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
              <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" fill="none" />
              <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
            </svg>
            Stretching...
          </span>
        ) : `Apply Stretch (S=${factor.toFixed(0)})`}
      </button>

      {error && (
        <div className="text-xs text-red-400 bg-red-900/20 border border-red-800/30 rounded-lg px-3 py-2">
          {error}
        </div>
      )}

      {result && (
        <div className="flex flex-col gap-2">
          <div className="grid grid-cols-3 gap-2 text-[10px]">
            <div className="bg-zinc-900/60 rounded px-2 py-1.5 text-center">
              <div className="text-zinc-500">Factor</div>
              <div className="text-amber-300 font-mono">{result.stretch_factor?.toFixed(1)}</div>
            </div>
            <div className="bg-zinc-900/60 rounded px-2 py-1.5 text-center">
              <div className="text-zinc-500">Time</div>
              <div className="text-zinc-200 font-mono">
                {result.elapsed_ms ? `${(result.elapsed_ms / 1000).toFixed(2)}s` : "--"}
              </div>
            </div>
            <div className="bg-zinc-900/60 rounded px-2 py-1.5 text-center">
              <div className="text-zinc-500">Size</div>
              <div className="text-zinc-200 font-mono">
                {result.dimensions ? `${result.dimensions[0]}x${result.dimensions[1]}` : "--"}
              </div>
            </div>
          </div>

          {canCompare && (
            <div className="flex flex-col gap-1.5">
              <div className="flex items-center gap-2 text-[10px]">
                <span className="text-zinc-500">Before / After</span>
                <button
                  onClick={() => setShowOriginal(!showOriginal)}
                  className="text-amber-400/60 hover:text-amber-400 transition-colors"
                >
                  {showOriginal ? "Show result" : "Hold original"}
                </button>
              </div>
              <div
                ref={compareRef}
                className="relative rounded-lg overflow-hidden border border-zinc-800 cursor-col-resize select-none"
                onMouseDown={() => { dragging.current = true; }}
                style={{ height: 180 }}
              >
                <img
                  src={stretchedUrl}
                  alt="stretched"
                  className="absolute inset-0 w-full h-full object-cover"
                />
                <div
                  className="absolute inset-0 overflow-hidden"
                  style={{ width: `${comparePosition}%` }}
                >
                  <img
                    src={originalUrl}
                    alt="original"
                    className="w-full h-full object-cover"
                    style={{ width: compareRef.current?.offsetWidth || "100%" }}
                  />
                </div>
                <div
                  className="absolute top-0 bottom-0 w-0.5 bg-amber-400/80"
                  style={{ left: `${comparePosition}%` }}
                >
                  <div className="absolute top-1/2 -translate-y-1/2 -translate-x-1/2 w-4 h-4 rounded-full bg-amber-400 border-2 border-zinc-900" />
                </div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
