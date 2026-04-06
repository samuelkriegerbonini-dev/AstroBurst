import { useState, useCallback, useMemo } from "react";
import { Loader2 } from "lucide-react";
import type { WizardState } from "../wizard";
import { stackFrames } from "../../../services/stacking";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton } from "../../ui";

interface StackStepProps {
  state: WizardState;
  onStacked: (channelId: string, path: string) => void;
}

export default function StackStep({ state, onStacked }: StackStepProps) {
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [results, setResults] = useState<Record<string, any>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});

  const stackableBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 1),
    [state.bins],
  );

  const singleBins = useMemo(
    () => state.bins.filter((b) => b.files.length === 1),
    [state.bins],
  );

  const handleStack = useCallback(async (binId: string, paths: string[]) => {
    if (!paths || paths.length < 2) {
      setErrors((prev) => ({ ...prev, [binId]: `Need at least 2 files, got ${paths?.length ?? 0}` }));
      return;
    }
    setLoading((prev) => ({ ...prev, [binId]: true }));
    setErrors((prev) => ({ ...prev, [binId]: "" }));
    try {
      const result = await stackFrames(paths, await getOutputDir(), {
        sigmaLow: 3.0,
        sigmaHigh: 3.0,
        align: true,
        name: `stacked_${binId}`,
      });
      setResults((prev) => ({ ...prev, [binId]: result }));
      if (result.fits_path) {
        onStacked(binId, result.fits_path);
      }
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      console.error(`[AstroBurst] Stack failed for ${binId}:`, msg);
      setErrors((prev) => ({ ...prev, [binId]: msg }));
    } finally {
      setLoading((prev) => ({ ...prev, [binId]: false }));
    }
  }, [onStacked]);

  const handleStackAll = useCallback(async () => {
    const bins = stackableBins.slice();
    const promises = bins.map((bin) => handleStack(bin.id, bin.files));
    await Promise.allSettled(promises);
  }, [stackableBins, handleStack]);

  if (stackableBins.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-zinc-600 text-xs gap-2">
        <span>No channels have multiple files to stack.</span>
        {singleBins.length > 0 && (
          <span className="text-[10px] text-zinc-700">
            {singleBins.length} channel(s) with single files will be used directly.
          </span>
        )}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <span className="text-xs text-zinc-400">
          {stackableBins.length} channel(s) to stack
        </span>
        <RunButton
          label="Stack All"
          runningLabel="Stacking..."
          running={Object.values(loading).some(Boolean)}
          accent="blue"
          onClick={handleStackAll}
          small
        />
      </div>

      {stackableBins.map((bin) => {
        const isLoading = loading[bin.id];
        const result = results[bin.id];
        const error = errors[bin.id];
        const isStacked = !!state.stackedPaths[bin.id] || !!result;
        return (
          <div key={bin.id} className="flex flex-col gap-1.5 p-2 rounded-lg border"
            style={{
              borderColor: isStacked ? `${bin.color}40` : "rgba(63,63,70,0.3)",
              background: isStacked ? `${bin.color}08` : "rgba(24,24,27,0.3)",
            }}>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-1.5">
                <span className="w-2 h-2 rounded-full" style={{ background: bin.color }} />
                <span className="text-[10px] font-medium text-zinc-300">{bin.shortLabel}</span>
                <span className="text-[9px] text-zinc-600">{bin.files.length} frames</span>
              </div>
              <button
                onClick={() => handleStack(bin.id, bin.files)}
                disabled={isLoading}
                className="flex items-center gap-1 px-2 py-0.5 rounded text-[9px] bg-blue-600/20 text-blue-400 hover:bg-blue-600/30 disabled:opacity-40 transition-all"
              >
                {isLoading ? <Loader2 size={9} className="animate-spin" /> : null}
                {isLoading ? "Stacking..." : isStacked ? "Re-stack" : "Stack"}
              </button>
            </div>
            {bin.files.length <= 4 && (
              <div className="flex flex-col gap-0.5">
                {bin.files.map((f, i) => (
                  <span key={i} className="text-[8px] text-zinc-700 font-mono truncate">
                    {f.split(/[/\\]/).pop()}
                  </span>
                ))}
              </div>
            )}
            {result && (
              <div className="text-[9px] text-zinc-500">
                {result.frame_count} frames, {result.rejected_pixels ?? 0} rejected px, {result.elapsed_ms ?? "?"}ms
              </div>
            )}
            {error && <div className="text-[9px] text-red-400">{error}</div>}
          </div>
        );
      })}

      {singleBins.length > 0 && (
        <div className="text-[9px] text-zinc-600 pt-1 border-t border-zinc-800/30">
          {singleBins.map((b) => b.shortLabel).join(", ")} have single files (no stacking needed).
        </div>
      )}
    </div>
  );
}
