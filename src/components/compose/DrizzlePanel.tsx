import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { Layers, Maximize2, Info } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, SectionHeader } from "../ui";
import ProgressBar from "../file/ProgressBar";
import type { ProcessedFile } from "../../shared/types";

interface DrizzlePanelProps {
  files?: ProcessedFile[];
  onDrizzle?: (paths: string[], options: Record<string, any>) => void;
  result?: any;
  isLoading?: boolean;
}

const ICON = <Layers size={14} className="text-cyan-400" />;

export default function DrizzlePanel({
  files = [],
  onDrizzle,
  result = null,
  isLoading = false,
}: DrizzlePanelProps) {
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [scale, setScale] = useState(2.0);
  const [pixfrac, setPixfrac] = useState(0.7);
  const [kernel, setKernel] = useState("square");
  const [sigmaLow, setSigmaLow] = useState(3.0);
  const [sigmaHigh, setSigmaHigh] = useState(3.0);
  const [align, setAlign] = useState(true);
  const [alignmentMethod, setAlignmentMethod] = useState("fft");
  const [elapsed, setElapsed] = useState("0");
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (isLoading) {
      setElapsed("0");
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

  const toggleFile = useCallback((path: string) => {
    setSelectedPaths((prev) =>
      prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path]
    );
  }, []);

  const selectAll = useCallback(() => setSelectedPaths(files.map((f) => f.path)), [files]);
  const clearAll = useCallback(() => setSelectedPaths([]), []);

  const canDrizzle = selectedPaths.length >= 2;

  const estimatedOutputRes = useMemo(() => {
    if (result) return `${result.output_dims[0]}×${result.output_dims[1]}`;
    const firstFile = files[0];
    if (firstFile?.result?.dimensions) {
      return `~${Math.ceil(firstFile.result.dimensions[0] * scale)}×${Math.ceil(firstFile.result.dimensions[1] * scale)}`;
    }
    return null;
  }, [result, files, scale]);

  const handleDrizzle = useCallback(() => {
    if (!canDrizzle || !onDrizzle) return;
    onDrizzle(selectedPaths, { scale, pixfrac, kernel, sigmaLow, sigmaHigh, align, alignmentMethod });
  }, [canDrizzle, onDrizzle, selectedPaths, scale, pixfrac, kernel, sigmaLow, sigmaHigh, align, alignmentMethod]);

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Drizzle Stack" subtitle={selectedPaths.length > 0 ? `${selectedPaths.length} selected` : undefined} />

      <div className="space-y-1">
        <div className="flex items-center justify-between">
          <span className="text-[9px] text-zinc-600">Select frames to stack</span>
          <div className="flex gap-2">
            <button onClick={selectAll} className="text-[9px] text-zinc-500 hover:text-zinc-300">All</button>
            <button onClick={clearAll} className="text-[9px] text-zinc-500 hover:text-zinc-300">Clear</button>
          </div>
        </div>
        <div className="max-h-28 overflow-y-auto space-y-0.5 custom-scrollbar">
          {files.map((f) => (
            <label key={f.path || f.id} className="flex items-center gap-1.5 text-[10px] text-zinc-400 cursor-pointer hover:text-zinc-300 py-0.5">
              <input type="checkbox" checked={selectedPaths.includes(f.path)} onChange={() => toggleFile(f.path)} className="w-3 h-3 accent-cyan-500" />
              <span className="truncate">{f.name || f.path}</span>
            </label>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Scale</label>
          <select value={scale} onChange={(e) => setScale(parseFloat(e.target.value))} className="ab-select">
            <option value={1.5}>1.5× (Subtle)</option>
            <option value={2}>2.0× (Standard)</option>
            <option value={3}>3.0× (Aggressive)</option>
          </select>
        </div>

        <Slider label="Pixfrac" value={pixfrac} min={0.1} max={1.0} step={0.05} accent="sky" format={(v) => v.toFixed(2)} onChange={setPixfrac} />

        <div className="flex items-center justify-between">
          <label className="text-xs text-zinc-400">Kernel</label>
          <select value={kernel} onChange={(e) => setKernel(e.target.value)} className="ab-select">
            <option value="square">Square (Variable Pixel)</option>
            <option value="gaussian">Gaussian</option>
            <option value="lanczos3">Lanczos-3</option>
          </select>
        </div>

        <div className="flex items-center gap-2">
          <label className="text-[10px] text-zinc-500 w-14">Sigma</label>
          <div className="flex-1 flex items-center gap-1">
            <input type="number" min="1" max="10" step="0.5" value={sigmaLow} onChange={(e) => setSigmaLow(parseFloat(e.target.value))} className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center" />
            <span className="text-[9px] text-zinc-600">low</span>
            <input type="number" min="1" max="10" step="0.5" value={sigmaHigh} onChange={(e) => setSigmaHigh(parseFloat(e.target.value))} className="w-12 bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-[10px] text-zinc-300 outline-none text-center" />
            <span className="text-[9px] text-zinc-600">high</span>
          </div>
        </div>

        <Toggle label="Sub-pixel alignment" checked={align} accent="sky" onChange={setAlign} />

        {align && (
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Method</label>
            <select value={alignmentMethod} onChange={(e) => setAlignmentMethod(e.target.value)} className="ab-select">
              <option value="fft">Phase Correlation (FFT)</option>
              <option value="zncc">ZNCC (Spatial)</option>
            </select>
          </div>
        )}
      </div>

      {estimatedOutputRes && (
        <div className="flex items-center gap-1.5 text-[10px] text-zinc-500">
          <Maximize2 size={9} />
          Output: {estimatedOutputRes}
        </div>
      )}

      {isLoading ? (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-cyan-300">Drizzle stacking {selectedPaths.length} frames…</span>
            <span className="text-[10px] text-zinc-500 font-mono">{elapsed}s</span>
          </div>
          <ProgressBar value={0} variant="blue" indeterminate />
        </div>
      ) : (
        <RunButton label={`Drizzle Stack (${scale}×)`} runningLabel="Stacking..." running={false} disabled={!canDrizzle} accent="sky" onClick={handleDrizzle} />
      )}

      {!canDrizzle && !isLoading && selectedPaths.length > 0 && selectedPaths.length < 2 && (
        <div className="flex items-center gap-1.5 text-[10px] text-amber-400/70">
          <Info size={9} />
          Select at least 2 frames
        </div>
      )}

      {result && !isLoading && (
        <div className="flex flex-col gap-3 animate-fade-in border-t border-zinc-800/50 pt-3">
          {result.previewUrl && (
            <img src={result.previewUrl} alt="Drizzle result" className="w-full rounded border border-zinc-700" />
          )}
          <ResultGrid columns={2} items={[
            { label: "Input", value: result.input_dims ? `${result.input_dims[0]}×${result.input_dims[1]}` : "--" },
            { label: "Output", value: result.output_dims ? `${result.output_dims[0]}×${result.output_dims[1]}` : "--" },
          ]} />
          <div className="text-[10px] text-zinc-500">
            {result.frame_count} frames · {result.elapsed_ms}ms · {result.scale || scale}× · {result.rejected_pixels?.toLocaleString() || 0} rejected
          </div>
          {result.weightMapUrl && (
            <details className="text-[10px]">
              <summary className="text-zinc-500 cursor-pointer hover:text-zinc-300">Weight map</summary>
              <img src={result.weightMapUrl} alt="Weight map" className="w-full rounded border border-zinc-700 mt-1" />
            </details>
          )}
        </div>
      )}
    </div>
  );
}
