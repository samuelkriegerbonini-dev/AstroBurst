import { useState, useCallback, useEffect, useRef } from "react";
import { Loader2, Layers, Play, CheckCircle2, AlertCircle, GripVertical, ArrowDown } from "lucide-react";
import { useBackend } from "../../hooks/useBackend";
import type { ProcessedFile } from "../../utils/types";
import type { StackConfig } from "./StackingTab";

interface StackingPanelProps {
  files: ProcessedFile[];
  onResult?: (result: any) => void;
  injectedPaths?: string[];
  stackConfig?: StackConfig;
  onStackConfigChange?: (config: Partial<StackConfig>) => void;
}

export default function StackingPanel({
                                        files = [],
                                        onResult,
                                        injectedPaths = [],
                                        stackConfig,
                                        onStackConfigChange,
                                      }: StackingPanelProps) {
  const { stackFrames } = useBackend();
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [isStacking, setIsStacking] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const prevInjectedRef = useRef<string[]>([]);

  const sigmaLow = stackConfig?.sigmaLow ?? 3.0;
  const sigmaHigh = stackConfig?.sigmaHigh ?? 3.0;
  const maxIterations = stackConfig?.maxIterations ?? 5;
  const align = stackConfig?.align ?? true;

  useEffect(() => {
    if (injectedPaths.length === 0) return;
    const newPaths = injectedPaths.filter((p) => !prevInjectedRef.current.includes(p));
    if (newPaths.length === 0) return;
    prevInjectedRef.current = injectedPaths;
    setSelectedPaths((prev) => {
      const merged = [...prev];
      for (const p of newPaths) {
        if (!merged.includes(p)) merged.push(p);
      }
      return merged;
    });
  }, [injectedPaths]);

  const toggleFile = useCallback((path: string) => {
    setSelectedPaths((prev) =>
      prev.includes(path) ? prev.filter((p) => p !== path) : [...prev, path],
    );
  }, []);

  const selectAll = useCallback(() => {
    const allPaths = [
      ...files.map((f) => f.path),
      ...injectedPaths.filter((p) => !files.some((f) => f.path === p)),
    ];
    setSelectedPaths(allPaths);
  }, [files, injectedPaths]);

  const selectNone = useCallback(() => {
    setSelectedPaths([]);
  }, []);

  const handleStack = useCallback(async () => {
    if (selectedPaths.length < 2) return;
    setIsStacking(true);
    setError(null);
    setResult(null);
    try {
      const res = await stackFrames(selectedPaths, "./output", {
        sigmaLow,
        sigmaHigh,
        maxIterations,
        align,
      });
      setResult(res);
      onResult?.(res);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setIsStacking(false);
    }
  }, [selectedPaths, sigmaLow, sigmaHigh, maxIterations, align, stackFrames, onResult]);

  const injectedOnly = injectedPaths.filter((p) => !files.some((f) => f.path === p));

  return (
    <div className="flex flex-col gap-3">
      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4">
        <div className="flex items-center justify-between mb-2">
          <h4 className="text-xs font-semibold text-amber-400 uppercase tracking-wider flex items-center gap-1.5">
            <Layers size={12} />
            Frames to Stack
            {selectedPaths.length > 0 && (
              <span className="text-zinc-500">({selectedPaths.length})</span>
            )}
          </h4>
          <div className="flex gap-2">
            <button onClick={selectAll} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">
              All
            </button>
            <button onClick={selectNone} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">
              None
            </button>
          </div>
        </div>

        <div className="flex flex-col gap-1 max-h-[160px] overflow-y-auto">
          {injectedOnly.length > 0 && (
            <>
              <div className="flex items-center gap-1.5 px-2 py-1 text-[10px] text-emerald-400/80">
                <ArrowDown size={10} />
                From Calibration
              </div>
              {injectedOnly.map((path) => {
                const isSelected = selectedPaths.includes(path);
                const name = path.split(/[/\\]/).pop() || path;
                return (
                  <button
                    key={path}
                    onClick={() => toggleFile(path)}
                    className={`flex items-center gap-2 px-2.5 py-1.5 rounded text-[11px] transition-all text-left ${
                      isSelected
                        ? "bg-emerald-500/10 text-zinc-200 ring-1 ring-emerald-500/30"
                        : "text-zinc-500 hover:bg-zinc-800/40 hover:text-zinc-300"
                    }`}
                  >
                    <GripVertical size={10} className="text-zinc-700 shrink-0" />
                    <span
                      className={`w-3 h-3 rounded-sm border flex items-center justify-center shrink-0 ${
                        isSelected ? "bg-emerald-500/20 border-emerald-500" : "border-zinc-600"
                      }`}
                    >
                      {isSelected && <CheckCircle2 size={10} className="text-emerald-400" />}
                    </span>
                    <span className="truncate">{name}</span>
                    <span className="ml-auto text-[9px] text-emerald-500/60 shrink-0">calibrated</span>
                  </button>
                );
              })}
            </>
          )}

          {files.map((f) => {
            const isSelected = selectedPaths.includes(f.path);
            return (
              <button
                key={f.id}
                onClick={() => toggleFile(f.path)}
                className={`flex items-center gap-2 px-2.5 py-1.5 rounded text-[11px] transition-all text-left ${
                  isSelected
                    ? "bg-amber-500/10 text-zinc-200 ring-1 ring-amber-500/30"
                    : "text-zinc-500 hover:bg-zinc-800/40 hover:text-zinc-300"
                }`}
              >
                <GripVertical size={10} className="text-zinc-700 shrink-0" />
                <span
                  className={`w-3 h-3 rounded-sm border flex items-center justify-center shrink-0 ${
                    isSelected ? "bg-amber-500/20 border-amber-500" : "border-zinc-600"
                  }`}
                >
                  {isSelected && <CheckCircle2 size={10} className="text-amber-400" />}
                </span>
                <span className="truncate">{f.name}</span>
              </button>
            );
          })}
        </div>
      </div>

      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 p-4 space-y-3">
        <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
          Sigma Clipping
        </h4>

        <div>
          <div className="flex items-center justify-between mb-1">
            <label className="text-[10px] text-zinc-400">Sigma Low</label>
            <span className="text-[10px] font-mono text-zinc-500">{sigmaLow.toFixed(1)}</span>
          </div>
          <input
            type="range"
            min={1.0}
            max={6.0}
            step={0.1}
            value={sigmaLow}
            onChange={(e) => onStackConfigChange?.({ sigmaLow: parseFloat(e.target.value) })}
            className="w-full accent-amber-500"
          />
        </div>

        <div>
          <div className="flex items-center justify-between mb-1">
            <label className="text-[10px] text-zinc-400">Sigma High</label>
            <span className="text-[10px] font-mono text-zinc-500">{sigmaHigh.toFixed(1)}</span>
          </div>
          <input
            type="range"
            min={1.0}
            max={6.0}
            step={0.1}
            value={sigmaHigh}
            onChange={(e) => onStackConfigChange?.({ sigmaHigh: parseFloat(e.target.value) })}
            className="w-full accent-amber-500"
          />
        </div>

        <div>
          <div className="flex items-center justify-between mb-1">
            <label className="text-[10px] text-zinc-400">Max Iterations</label>
            <span className="text-[10px] font-mono text-zinc-500">{maxIterations}</span>
          </div>
          <input
            type="range"
            min={1}
            max={20}
            step={1}
            value={maxIterations}
            onChange={(e) => onStackConfigChange?.({ maxIterations: parseInt(e.target.value) })}
            className="w-full accent-amber-500"
          />
        </div>

        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={align}
            onChange={(e) => onStackConfigChange?.({ align: e.target.checked })}
            className="accent-amber-500"
          />
          <span className="text-[11px] text-zinc-300">Auto-align before stacking</span>
        </label>
      </div>

      <button
        onClick={handleStack}
        disabled={selectedPaths.length < 2 || isStacking}
        className="flex items-center justify-center gap-2 rounded-lg px-4 py-2.5 text-sm font-medium transition-all disabled:opacity-30 disabled:cursor-not-allowed"
        style={{
          background: "rgba(245,158,11,0.15)",
          color: "#fbbf24",
          border: "1px solid rgba(245,158,11,0.25)",
        }}
      >
        {isStacking ? <Loader2 size={14} className="animate-spin" /> : <Play size={14} />}
        {isStacking ? "Stacking..." : `Stack ${selectedPaths.length} Frames`}
      </button>

      {error && (
        <div className="flex items-start gap-2 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 text-xs text-red-300">
          <AlertCircle size={14} className="shrink-0 mt-0.5" />
          {error}
        </div>
      )}

      {result && (
        <div className="bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium mb-1.5">
            <CheckCircle2 size={12} />
            Stacking Complete
          </div>
          <div className="text-[10px] font-mono text-zinc-400 space-y-0.5">
            <div>{result.dimensions?.[0]}x{result.dimensions?.[1]}</div>
            <div>{result.frame_count} frames combined</div>
            {result.rejected_pixels > 0 && (
              <div>{result.rejected_pixels.toLocaleString()} pixels rejected</div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
