import { useState, useCallback, useMemo } from "react";
import { Loader2 } from "lucide-react";
import type { WizardState } from "../wizard";
import { resolveChannelPath as resolveWizardPath } from "../wizard";
import { extractBackground, extractBackgroundBatch } from "../../../services/processing";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton, Slider } from "../../ui";

interface BackgroundStepProps {
  state: WizardState;
  onBackground: (channelId: string, path: string) => void;
}

function resolveChannelPath(state: WizardState, binId: string): string | null {
  return resolveWizardPath(state, binId, "cropped");
}

export default function BackgroundStep({ state, onBackground }: BackgroundStepProps) {
  const [gridSize, setGridSize] = useState(8);
  const [polyDegree, setPolyDegree] = useState(3);
  const [sigmaClip, setSigmaClip] = useState(2.5);
  const [mode, setMode] = useState<"independent" | "linked" | "neutralize" | "deband_rows" | "deband_cols" | "deband_both">("independent");
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
        binId,
      });
      setResults((prev) => ({ ...prev, [binId]: result }));
      const key = result.cache_key || result.corrected_fits;
      if (key) {
        onBackground(binId, key);
      }
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      console.error(`[AstroBurst] BG extraction failed for ${binId} (${path}):`, msg);
      setErrors((prev) => ({ ...prev, [binId]: msg }));
    } finally {
      setLoading((prev) => ({ ...prev, [binId]: false }));
    }
  }, [state, gridSize, polyDegree, sigmaClip, onBackground]);

  const handleExtractAllBatch = useCallback(async (batchMode: string) => {
    const paths: string[] = [];
    const binIds: string[] = [];
    for (const bin of activeBins) {
      const p = resolveChannelPath(state, bin.id);
      if (p) {
        paths.push(p);
        binIds.push(bin.id);
      }
    }
    if (paths.length === 0) return;

    setLoading((prev) => {
      const next = { ...prev };
      binIds.forEach((id) => { next[id] = true; });
      return next;
    });
    setErrors({});
    try {
      const res = await extractBackgroundBatch(paths, binIds, await getOutputDir(), {
        gridSize,
        polyDegree,
        sigmaClip,
        iterations: 3,
        mode: batchMode,
      });
      const nextResults: Record<string, any> = {};
      for (const r of res.results ?? []) {
        nextResults[r.bin_id] = {
          sample_count: r.sample_count,
          rms_residual: res.rms_residual,
          elapsed_ms: res.elapsed_ms,
        };
        if (r.cache_key) onBackground(r.bin_id, r.cache_key);
      }
      setResults((prev) => ({ ...prev, ...nextResults }));
    } catch (e: any) {
      const msg = e?.message ?? String(e);
      console.error(`[AstroBurst] Batch BG extraction (${batchMode}) failed:`, msg);
      setErrors((prev) => {
        const next = { ...prev };
        binIds.forEach((id) => { next[id] = msg; });
        return next;
      });
    } finally {
      setLoading((prev) => {
        const next = { ...prev };
        binIds.forEach((id) => { next[id] = false; });
        return next;
      });
    }
  }, [activeBins, state, gridSize, polyDegree, sigmaClip, onBackground]);

  const handleExtractAll = useCallback(async () => {
    if (mode === "independent") {
      const bins = activeBins.slice();
      const promises = bins.map((bin) => handleExtract(bin.id));
      await Promise.allSettled(promises);
      return;
    }
    const batchMode = mode === "linked" ? "subtract" : mode;
    return handleExtractAllBatch(batchMode);
  }, [mode, activeBins, handleExtract, handleExtractAllBatch]);

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

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Mode</label>
        <select value={mode} onChange={(e) => setMode(e.target.value as typeof mode)} className="ab-select">
          <option value="independent">Per-channel (independent)</option>
          <option value="linked">Linked (shared gradient)</option>
          <option value="neutralize">Neutralize (remove pedestal)</option>
          <option value="deband_rows">De-band rows (horizontal 1/f)</option>
          <option value="deband_cols">De-band columns (vertical 1/f)</option>
          <option value="deband_both">De-band both axes</option>
        </select>
      </div>
      <div className="text-[9px] text-zinc-600">
        {mode === "linked" && "Fits one gradient on the channel mean and removes the same surface from every channel, preserving color balance."}
        {mode === "neutralize" && "Removes only a constant sky pedestal per channel (no spatial model). Safest for JWST banding / 1-f noise."}
        {mode === "independent" && "Fits and removes a separate gradient per channel."}
        {mode.startsWith("deband") && "Removes row/column striping (JWST 1/f noise) by equalizing per-line sigma-clipped background. Preserves overall level and color. Sigma Clip controls source rejection; grid/poly ignored."}
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
        const source = state.croppedPaths[bin.id]
          ? "cropped"
          : state.alignedPaths[bin.id]
            ? "aligned"
            : state.stackedPaths[bin.id]
              ? "stacked"
              : "raw";
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
                <span className="text-[8px] text-zinc-600">{source}</span>
                {!path && (
                  <span className="text-[8px] text-red-400/60">no path</span>
                )}
              </div>
              <button
                onClick={() => handleExtract(bin.id)}
                disabled={isLoading || !path || mode !== "independent"}
                title={mode !== "independent" ? "Use Extract All for shared modes" : undefined}
                className="flex items-center gap-1 px-2 py-0.5 rounded text-[9px] bg-emerald-600/20 text-emerald-400 hover:bg-emerald-600/30 disabled:opacity-40 transition-all"
              >
                {isLoading ? <Loader2 size={9} className="animate-spin" /> : null}
                {isLoading ? "Extracting..." : done ? "Re-extract" : "Extract"}
              </button>
            </div>
            {path && (
              <div className="text-[8px] text-zinc-700 font-mono truncate">
                {path.startsWith("__wizard_ch_") ? `${bin.shortLabel} (cache)` : path.split(/[/\\]/).pop()}
              </div>
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
