import { useState, useCallback, useEffect, useRef } from "react";
import { Layers, GripVertical, ArrowDown, CheckCircle2 } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, ErrorAlert, SectionHeader } from "../ui";
import { stackFrames } from "../../services/stacking";
import { getOutputDir } from "../../infrastructure/tauri";
import type { ProcessedFile } from "../../shared/types";
import type { StackConfig } from "./StackingTab";

interface StackingPanelProps {
  files: ProcessedFile[];
  onResult?: (result: any) => void;
  injectedPaths?: string[];
  stackConfig?: StackConfig;
  onStackConfigChange?: (config: Partial<StackConfig>) => void;
}

const ICON = <Layers size={14} className="text-amber-400" />;

export default function StackingPanel({
  files = [],
  onResult,
  injectedPaths = [],
  stackConfig,
  onStackConfigChange,
}: StackingPanelProps) {
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

  const selectNone = useCallback(() => setSelectedPaths([]), []);

  const handleStack = useCallback(async () => {
    if (selectedPaths.length < 2) return;
    setIsStacking(true);
    setError(null);
    setResult(null);
    try {
      const res = await stackFrames(selectedPaths, await getOutputDir(), {
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
  }, [selectedPaths, sigmaLow, sigmaHigh, maxIterations, align, onResult]);

  const injectedOnly = injectedPaths.filter((p) => !files.some((f) => f.path === p));

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <div className="flex items-center justify-between">
        <SectionHeader icon={ICON} title="Frames to Stack" subtitle={selectedPaths.length > 0 ? `${selectedPaths.length} selected` : undefined} />
        <div className="flex gap-2">
          <button onClick={selectAll} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">All</button>
          <button onClick={selectNone} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">None</button>
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
                <button key={path} onClick={() => toggleFile(path)} className={`flex items-center gap-2 px-2.5 py-1.5 rounded text-[11px] transition-all text-left ${isSelected ? "bg-emerald-500/10 text-zinc-200 ring-1 ring-emerald-500/30" : "text-zinc-500 hover:bg-zinc-800/40 hover:text-zinc-300"}`}>
                  <GripVertical size={10} className="text-zinc-700 shrink-0" />
                  <span className={`w-3 h-3 rounded-sm border flex items-center justify-center shrink-0 ${isSelected ? "bg-emerald-500/20 border-emerald-500" : "border-zinc-600"}`}>
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
            <button key={f.id} onClick={() => toggleFile(f.path)} className={`flex items-center gap-2 px-2.5 py-1.5 rounded text-[11px] transition-all text-left ${isSelected ? "bg-amber-500/10 text-zinc-200 ring-1 ring-amber-500/30" : "text-zinc-500 hover:bg-zinc-800/40 hover:text-zinc-300"}`}>
              <GripVertical size={10} className="text-zinc-700 shrink-0" />
              <span className={`w-3 h-3 rounded-sm border flex items-center justify-center shrink-0 ${isSelected ? "bg-amber-500/20 border-amber-500" : "border-zinc-600"}`}>
                {isSelected && <CheckCircle2 size={10} className="text-amber-400" />}
              </span>
              <span className="truncate">{f.name}</span>
            </button>
          );
        })}
      </div>

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <span className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">Sigma Clipping</span>
        <Slider label="Sigma Low" value={sigmaLow} min={1.0} max={6.0} step={0.1} accent="amber" format={(v) => v.toFixed(1)} onChange={(v) => onStackConfigChange?.({ sigmaLow: v })} />
        <Slider label="Sigma High" value={sigmaHigh} min={1.0} max={6.0} step={0.1} accent="amber" format={(v) => v.toFixed(1)} onChange={(v) => onStackConfigChange?.({ sigmaHigh: v })} />
        <Slider label="Max Iterations" value={maxIterations} min={1} max={20} step={1} accent="amber" onChange={(v) => onStackConfigChange?.({ maxIterations: v })} />
        <Toggle label="Auto-align before stacking" checked={align} accent="amber" onChange={(v) => onStackConfigChange?.({ align: v })} />
      </div>

      <RunButton label={`Stack ${selectedPaths.length} Frames`} runningLabel="Stacking..." running={isStacking} disabled={selectedPaths.length < 2} accent="amber" onClick={handleStack} />
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            Stacking Complete
          </div>
          <ResultGrid columns={3} items={[
            { label: "Dimensions", value: result.dimensions ? `${result.dimensions[0]}×${result.dimensions[1]}` : "--" },
            { label: "Frames", value: result.frame_count },
            { label: "Rejected", value: result.rejected_pixels > 0 ? result.rejected_pixels.toLocaleString() : "0" },
          ]} />
        </div>
      )}
    </div>
  );
}
