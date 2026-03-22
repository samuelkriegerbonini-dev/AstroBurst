import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { Maximize2 } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, SectionHeader } from "../ui";
import ProgressBar from "../file/ProgressBar";
import type { ProcessedFile } from "../../shared/types";

interface DrizzlePanelProps {
  files?: ProcessedFile[];
  onDrizzle?: (paths: string[], options: Record<string, any>) => void;
  result?: any;
  isLoading?: boolean;
  progress?: number;
  progressStage?: string;
}

const ICON = <Maximize2 size={14} className="text-sky-400" />;

export default function DrizzlePanel({
  files = [],
  onDrizzle,
  result = null,
  isLoading = false,
  progress = 0,
  progressStage = "",
}: DrizzlePanelProps) {
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());
  const [scale, setScale] = useState(2.0);
  const [pixfrac, setPixfrac] = useState(0.8);
  const [kernel, setKernel] = useState("square");
  const [sigmaLow, setSigmaLow] = useState(2.5);
  const [sigmaHigh, setSigmaHigh] = useState(3.0);
  const [align, setAlign] = useState(true);
  const [alignmentMethod, setAlignmentMethod] = useState("phase_correlation");
  const [saveFits, setSaveFits] = useState(false);
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

  const donePaths = useMemo(
    () => files.filter((f) => f.status === "done" && f.path).map((f) => f.path),
    [files],
  );

  const togglePath = useCallback((path: string) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const selectAll = useCallback(() => {
    setSelectedPaths(new Set(donePaths));
  }, [donePaths]);

  const deselectAll = useCallback(() => {
    setSelectedPaths(new Set());
  }, []);

  const canDrizzle = selectedPaths.size >= 2 && !isLoading;

  const handleRun = useCallback(() => {
    if (!canDrizzle || !onDrizzle) return;
    const paths = Array.from(selectedPaths);
    onDrizzle(paths, {
      scale,
      pixfrac,
      kernel,
      sigmaLow,
      sigmaHigh,
      align,
      alignmentMethod: align ? alignmentMethod : undefined,
      saveFits,
    });
  }, [canDrizzle, onDrizzle, selectedPaths, scale, pixfrac, kernel, sigmaLow, sigmaHigh, align, alignmentMethod, saveFits]);

  const estimatedDims = useMemo(() => {
    const first = files.find((f) => selectedPaths.has(f.path) && f.result?.dimensions);
    if (!first?.result?.dimensions) return null;
    const [w, h] = first.result.dimensions;
    return `~${Math.ceil(w * scale)}x${Math.ceil(h * scale)}`;
  }, [files, selectedPaths, scale]);

  if (donePaths.length < 2) return null;

  return (
    <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3 px-4">
      <SectionHeader icon={ICON} title="Drizzle Stack" subtitle={`${selectedPaths.size}/${donePaths.length} selected`} />

      <div className="flex gap-1.5 mb-1">
        <button onClick={selectAll} className="text-[10px] text-sky-400 hover:underline">All</button>
        <span className="text-zinc-600 text-[10px]">|</span>
        <button onClick={deselectAll} className="text-[10px] text-zinc-500 hover:underline">None</button>
      </div>

      <div className="max-h-32 overflow-y-auto flex flex-col gap-0.5">
        {donePaths.map((p) => {
          const f = files.find((ff) => ff.path === p);
          return (
            <label key={p} className="flex items-center gap-2 text-xs text-zinc-300 cursor-pointer hover:bg-zinc-800/30 rounded px-1.5 py-0.5">
              <input
                type="checkbox"
                checked={selectedPaths.has(p)}
                onChange={() => togglePath(p)}
                className="accent-sky-500"
              />
              <span className="truncate">{f?.name || p.split("/").pop()}</span>
            </label>
          );
        })}
      </div>

      <Slider label="Scale" value={scale} min={1.0} max={4.0} step={0.5} accent="sky" format={(v) => `${v}x`} onChange={setScale} />
      <Slider label="Pixfrac" value={pixfrac} min={0.1} max={1.0} step={0.1} accent="sky" format={(v) => v.toFixed(1)} onChange={setPixfrac} />

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Kernel</label>
        <select value={kernel} onChange={(e) => setKernel(e.target.value)} className="ab-select">
          <option value="square">Square</option>
          <option value="gaussian">Gaussian</option>
          <option value="lanczos3">Lanczos3</option>
        </select>
      </div>

      <Slider label="Sigma Low" value={sigmaLow} min={1.0} max={5.0} step={0.5} accent="sky" format={(v) => v.toFixed(1)} onChange={setSigmaLow} />
      <Slider label="Sigma High" value={sigmaHigh} min={1.0} max={5.0} step={0.5} accent="sky" format={(v) => v.toFixed(1)} onChange={setSigmaHigh} />

      <Toggle label="Align frames" checked={align} accent="sky" onChange={setAlign} />
      {align && (
        <div className="flex items-center justify-between pl-4">
          <label className="text-xs text-zinc-400">Method</label>
          <select value={alignmentMethod} onChange={(e) => setAlignmentMethod(e.target.value)} className="ab-select">
            <option value="phase_correlation">Phase Correlation</option>
            <option value="zncc">ZNCC</option>
          </select>
        </div>
      )}

      <Toggle label="Save FITS output" checked={saveFits} accent="sky" onChange={setSaveFits} />

      {estimatedDims && (
        <div className="text-[10px] text-zinc-500">
          Output: {estimatedDims}
        </div>
      )}

      {isLoading ? (
        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <span className="text-[11px] text-sky-300">{progressStage || `Processing ${selectedPaths.size} frames…`}</span>
            <span className="text-[10px] text-zinc-500 font-mono">{elapsed}s</span>
          </div>
          <ProgressBar value={progress} variant="blue" indeterminate={progress <= 0} />
        </div>
      ) : (
        <RunButton
          label={`Drizzle (${scale}x)`}
          runningLabel="Processing..."
          running={false}
          disabled={!canDrizzle}
          accent="sky"
          onClick={handleRun}
        />
      )}

      {result && !isLoading && (
        <div className="flex flex-col gap-2 animate-fade-in bg-sky-500/5 border border-sky-500/20 rounded-lg p-3">
          {result.previewUrl && (
            <img src={result.previewUrl} alt="Drizzle result" className="w-full rounded border border-zinc-700" />
          )}
          <ResultGrid items={[
            { label: "Input", value: `${result.frame_count || selectedPaths.size} frames` },
            { label: "Output", value: result.output_dims ? `${result.output_dims[0]}x${result.output_dims[1]}` : "--" },
            { label: "Scale", value: `${result.scale || scale}x` },
            { label: "Rejected", value: `${result.rejected_pixels?.toLocaleString() || 0} px` },
          ]} />
          <div className="text-[10px] text-zinc-500">{result.elapsed_ms} ms</div>
        </div>
      )}
    </div>
  );
}
