import { useState, useCallback, useRef, useEffect } from "react";
import { Layers, Loader2, Maximize2, Info } from "lucide-react";
import ProgressBar from "./ProgressBar";

export default function DrizzlePanel({
  files = [],
  onDrizzle,
  result = null,
  isLoading = false,
  progress = 0,
}) {
  const [selectedPaths, setSelectedPaths] = useState([]);
  const [scale, setScale] = useState(2.0);
  const [pixfrac, setPixfrac] = useState(0.7);
  const [kernel, setKernel] = useState("square");
  const [sigmaLow, setSigmaLow] = useState(3.0);
  const [sigmaHigh, setSigmaHigh] = useState(3.0);
  const [align, setAlign] = useState(true);
  const [showWeightMap, setShowWeightMap] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const timerRef = useRef(null);

  useEffect(() => {
    if (isLoading) {
      setElapsed(0);
      const start = Date.now();
      timerRef.current = setInterval(() => {
        setElapsed(((Date.now() - start) / 1000).toFixed(1));
      }, 100);
    } else {
      if (timerRef.current) clearInterval(timerRef.current);
    }
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [isLoading]);

  const toggleFile = useCallback((path) => {
    setSelectedPaths((prev) =>
      prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path],
    );
  }, []);

  const selectAll = useCallback(() => {
    setSelectedPaths(files.map((f) => f.path));
  }, [files]);

  const deselectAll = useCallback(() => {
    setSelectedPaths([]);
  }, []);

  const canDrizzle = selectedPaths.length >= 2;

  const handleDrizzle = useCallback(() => {
    if (!canDrizzle || !onDrizzle) return;
    onDrizzle(selectedPaths, {
      scale,
      pixfrac,
      kernel,
      sigmaLow,
      sigmaHigh,
      align,
    });
  }, [selectedPaths, scale, pixfrac, kernel, sigmaLow, sigmaHigh, align, onDrizzle, canDrizzle]);

  const outputResolution = result
    ? `${result.output_dims[0]}×${result.output_dims[1]}`
    : files.length > 0
      ? `~${Math.ceil((files[0]?.dimensions?.[0] || 1024) * scale)}×${Math.ceil((files[0]?.dimensions?.[1] || 1024) * scale)}`
      : null;

  return (
    <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Layers size={12} className="text-indigo-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">
            Drizzle Stack
          </span>
        </div>
        <div className="flex items-center gap-1">
          {selectedPaths.length > 0 && (
            <span className="text-[10px] text-indigo-300 bg-indigo-500/20 px-1.5 py-0.5 rounded">
              {selectedPaths.length} frames
            </span>
          )}
        </div>
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className="space-y-1">
          <div className="flex items-center justify-between">
            <label className="text-[10px] text-zinc-500">Frames</label>
            <div className="flex gap-2">
              <button
                onClick={selectAll}
                className="text-[9px] text-zinc-500 hover:text-zinc-300"
              >
                All
              </button>
              <button
                onClick={deselectAll}
                className="text-[9px] text-zinc-500 hover:text-zinc-300"
              >
                None
              </button>
            </div>
          </div>
          <div className="max-h-28 overflow-y-auto space-y-0.5 custom-scrollbar">
            {files.map((f) => (
              <label
                key={f.path || f.id}
                className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer hover:text-zinc-300 py-0.5"
              >
                <input
                  type="checkbox"
                  checked={selectedPaths.includes(f.path)}
                  onChange={() => toggleFile(f.path)}
                  className="w-3 h-3 accent-indigo-500"
                />
                <span className="truncate">{f.name || f.path}</span>
              </label>
            ))}
            {files.length === 0 && (
              <div className="text-[10px] text-zinc-600 py-2 text-center">
                No FITS files loaded
              </div>
            )}
          </div>
        </div>

        <div className="border-t border-zinc-800/50 pt-2 space-y-1.5">
          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Scale</label>
            <select
              value={scale}
              onChange={(e) => setScale(parseFloat(e.target.value))}
              className="flex-1 bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-[10px] text-zinc-300 outline-none"
            >
              <option value="1.5">1.5× (Subtle)</option>
              <option value="2.0">2.0× (Standard)</option>
              <option value="3.0">3.0× (Aggressive)</option>
            </select>
          </div>

          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Pixfrac</label>
            <input
              type="range"
              min="0.1"
              max="1.0"
              step="0.05"
              value={pixfrac}
              onChange={(e) => setPixfrac(parseFloat(e.target.value))}
              className="flex-1 h-1 accent-indigo-500"
            />
            <span className="text-[10px] text-zinc-300 font-mono w-8 text-right">
              {pixfrac.toFixed(2)}
            </span>
          </div>

          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Kernel</label>
            <select
              value={kernel}
              onChange={(e) => setKernel(e.target.value)}
              className="flex-1 bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-[10px] text-zinc-300 outline-none"
            >
              <option value="square">Square (Variable Pixel)</option>
              <option value="gaussian">Gaussian</option>
              <option value="lanczos3">Lanczos-3</option>
            </select>
          </div>

          <div className="flex items-center gap-2">
            <label className="text-[10px] text-zinc-500 w-14">Sigma</label>
            <div className="flex-1 flex items-center gap-1">
              <input
                type="number"
                min="1"
                max="10"
                step="0.5"
                value={sigmaLow}
                onChange={(e) => setSigmaLow(parseFloat(e.target.value))}
                className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center"
              />
              <span className="text-[9px] text-zinc-600">low</span>
              <input
                type="number"
                min="1"
                max="10"
                step="0.5"
                value={sigmaHigh}
                onChange={(e) => setSigmaHigh(parseFloat(e.target.value))}
                className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center"
              />
              <span className="text-[9px] text-zinc-600">high</span>
            </div>
          </div>

          <label className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer">
            <input
              type="checkbox"
              checked={align}
              onChange={(e) => setAlign(e.target.checked)}
              className="w-3 h-3 accent-indigo-500"
            />
            Sub-pixel alignment (ZNCC)
          </label>
        </div>

        {outputResolution && (
          <div className="flex items-center gap-1.5 text-[10px] text-zinc-500">
            <Maximize2 size={9} />
            Output: {outputResolution}
          </div>
        )}

        {isLoading ? (
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2 text-[11px] text-indigo-300">
                <Loader2 size={12} className="animate-spin" />
                Drizzling {selectedPaths.length} frames…
              </div>
              <span className="text-[10px] text-zinc-500 font-mono">{elapsed}s</span>
            </div>
            <ProgressBar
              value={progress}
              variant="blue"
              indeterminate={progress <= 0}
            />
          </div>
        ) : (
          <button
            onClick={handleDrizzle}
            disabled={!canDrizzle}
            className="w-full flex items-center justify-center gap-2 bg-indigo-600/20 hover:bg-indigo-600/30 text-indigo-300 border border-indigo-600/30 rounded px-3 py-1.5 text-xs font-medium transition-colors disabled:opacity-50"
          >
            <Layers size={12} />
            Drizzle ({selectedPaths.length} frames, {scale}×)
          </button>
        )}

        {!canDrizzle && !isLoading && selectedPaths.length > 0 && selectedPaths.length < 2 && (
          <div className="flex items-center gap-1.5 text-[10px] text-amber-400/70">
            <Info size={9} />
            Drizzle requires at least 2 dithered frames
          </div>
        )}

        {result && !isLoading && (
          <div className="space-y-1.5">
            <div className="relative">
              <img
                src={showWeightMap ? result.weightMapUrl : result.previewUrl}
                alt={showWeightMap ? "Weight map" : "Drizzle result"}
                className="w-full rounded border border-zinc-700"
              />
              {result.weightMapUrl && (
                <button
                  onClick={() => setShowWeightMap((p) => !p)}
                  className="absolute top-1 right-1 text-[9px] bg-black/60 hover:bg-black/80 text-zinc-300 rounded px-1.5 py-0.5 backdrop-blur-sm transition-colors"
                >
                  {showWeightMap ? "Image" : "Weights"}
                </button>
              )}
            </div>

            <div className="grid grid-cols-2 gap-1 text-[10px]">
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-zinc-500">Input</div>
                <div className="text-zinc-300 font-mono">
                  {result.input_dims?.[0]}×{result.input_dims?.[1]}
                </div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-indigo-400">Output</div>
                <div className="text-zinc-300 font-mono">
                  {result.output_dims?.[0]}×{result.output_dims?.[1]}
                </div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-zinc-500">Frames</div>
                <div className="text-zinc-300 font-mono">{result.frame_count}</div>
              </div>
              <div className="bg-zinc-900/80 rounded px-2 py-1">
                <div className="text-zinc-500">Rejected</div>
                <div className="text-zinc-300 font-mono">{result.rejected_pixels?.toLocaleString()}</div>
              </div>
            </div>

            {result.offsets && result.offsets.length > 1 && (
              <details className="text-[10px]">
                <summary className="text-zinc-500 cursor-pointer hover:text-zinc-400">
                  Sub-pixel offsets ({result.offsets.length} frames)
                </summary>
                <div className="mt-1 space-y-0.5 pl-2">
                  {result.offsets.map((off, i) => (
                    <div key={i} className="text-zinc-400 font-mono">
                      #{i}: dx={off.dx?.toFixed(3)}, dy={off.dy?.toFixed(3)}
                    </div>
                  ))}
                </div>
              </details>
            )}

            <div className="text-[10px] text-zinc-500">
              {result.elapsed_ms} ms · {result.scale}× scale
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
