import { useState, useCallback, useMemo } from "react";
import { Loader2 } from "lucide-react";
import type { WizardState } from "../wizard.types";
import { extractBackground } from "../../../services/processing";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton, Slider } from "../../ui";

interface BackgroundStepProps {
  state: WizardState;
  onBackground: (channelId: string, path: string) => void;
}

function resolveChannelPath(state: WizardState, binId: string): string | null {
  if (state.stackedPaths[binId]) return state.stackedPaths[binId];
  if (state.backgroundPaths[binId]) return state.backgroundPaths[binId];
  const bin = state.bins.find((b) => b.id === binId);
  if (bin && bin.files.length > 0) return bin.files[0];
  return null;
}

export default function BackgroundStep({ state, onBackground }: BackgroundStepProps) {
  const [gridSize, setGridSize] = useState(8);
  const [polyDegree, setPolyDegree] = useState(3);
  const [sigmaClip, setSigmaClip] = useState(2.5);
  const [loading, setLoading] = useState<Record<string, boolean>>({});
  const [results, setResults] = useState<Record<string, any>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});

  const activeBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 0),
    [state.bins],
  );

  const handleExtract = useCallback(async (binId: string) => {
    const path = resolveChannelPath(state, binId);
    if (!path) {
      setErrors((prev) => ({ ...prev, [binId]: `No path found for channel ${binId}` }));
      return;
    }
    setLoading((prev) => ({ ...prev, [binId]: true }));
    setErrors((prev) => ({ ...prev, [binId]: "" }));
    try {
      const result = await extractBackground(path, await getOutputDir(), {
        gridSize,
        polyDegree,
        sigmaClip,
        iterations: 3,
        mode: "subtract",
      });
      setResults((prev) => ({ ...prev, [binId]: result }));
      if (result.corrected_fits) {
        onBackground(binId, result.corrected_fits);
      }
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      console.error(`[AstroBurst] BG extraction failed for ${binId} (${path}):`, msg);
      setErrors((prev) => ({ ...prev, [binId]: msg }));
    } finally {
      setLoading((prev) => ({ ...prev, [binId]: false }));
    }
  }, [state, gridSize, polyDegree, sigmaClip, onBackground]);

  const handleExtractAll = useCallback(async () => {
    const bins = activeBins.slice();
    const promises = bins.map((bin) => handleExtract(bin.id));
    await Promise.allSettled(promises);
  }, [activeBins, handleExtract]);

  if (activeBins.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-600 text-xs">
        No channels assigned yet.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex flex-col gap-2">
        <Slider label="Grid Size" value={gridSize} min={3} max={32} step={1} accent="emerald"
          format={(v) => `${v}`} onChange={setGridSize} />
        <Slider label="Poly Degree" value={polyDegree} min={1} max={5} step={1} accent="emerald"
          format={(v) => `${v}`} onChange={setPolyDegree} />
        <Slider label="Sigma Clip" value={sigmaClip} min={1.0} max={5.0} step={0.1} accent="emerald"
          format={(v) => v.toFixed(1)} onChange={setSigmaClip} />
      </div>

      <div className="flex items-center justify-between pt-1">
        <span className="text-xs text-zinc-400">{activeBins.length} channel(s)</span>
        <RunButton
          label="Extract All"
          runningLabel="Extracting..."
          running={Object.values(loading).some(Boolean)}
          accent="emerald"
          onClick={handleExtractAll}
          small
        />
      </div>

      {activeBins.map((bin) => {
        const isLoading = loading[bin.id];
        const result = results[bin.id];
        const error = errors[bin.id];
        const done = !!state.backgroundPaths[bin.id] || !!result;
        const path = resolveChannelPath(state, bin.id);
        return (
          <div key={bin.id} className="flex flex-col gap-1 p-2 rounded-lg border"
            style={{
              borderColor: done ? `${bin.color}40` : "rgba(63,63,70,0.3)",
              background: done ? `${bin.color}08` : "rgba(24,24,27,0.3)",
            }}>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-1.5">
                <span className="w-2 h-2 rounded-full" style={{ background: bin.color }} />
                <span className="text-[10px] font-medium text-zinc-300">{bin.shortLabel}</span>
                {!path && (
                  <span className="text-[8px] text-red-400/60">no path</span>
                )}
              </div>
              <button
                onClick={() => handleExtract(bin.id)}
                disabled={isLoading || !path}
                className="flex items-center gap-1 px-2 py-0.5 rounded text-[9px] bg-emerald-600/20 text-emerald-400 hover:bg-emerald-600/30 disabled:opacity-40 transition-all"
              >
                {isLoading ? <Loader2 size={9} className="animate-spin" /> : null}
                {isLoading ? "Extracting..." : done ? "Re-extract" : "Extract"}
              </button>
            </div>
            {path && (
              <div className="text-[8px] text-zinc-700 font-mono truncate">{path.split(/[/\\]/).pop()}</div>
            )}
            {result && (
              <div className="text-[9px] text-zinc-500">
                {result.sample_count} samples, RMS {result.rms_residual?.toFixed(4)}, {result.elapsed_ms}ms
              </div>
            )}
            {error && <div className="text-[9px] text-red-400">{error}</div>}
          </div>
        );
      })}
    </div>
  );
}
