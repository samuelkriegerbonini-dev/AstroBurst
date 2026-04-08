import { useState, useCallback, useMemo } from "react";
import { Scissors } from "lucide-react";
import type { WizardState } from "../wizard";
import { cropChannels } from "../../../services/compose";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton } from "../../ui";

interface CropStepProps {
  state: WizardState;
  onCropped: (paths: Record<string, string>) => void;
}

export default function CropStep({ state, onCropped }: CropStepProps) {
  const [mode, setMode] = useState<"auto" | "manual">("auto");
  const [top, setTop] = useState(0);
  const [bottom, setBottom] = useState(0);
  const [left, setLeft] = useState(0);
  const [right, setRight] = useState(0);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState("");
  const [skipped, setSkipped] = useState(false);

  const alignedEntries = useMemo(() => {
    return Object.entries(state.alignedPaths).filter(([, p]) => !!p);
  }, [state.alignedPaths]);

  const handleCrop = useCallback(async () => {
    if (alignedEntries.length === 0) return;
    setLoading(true);
    setError("");
    setSkipped(false);
    try {
      const paths = alignedEntries.map(([, p]) => p);
      const binIds = alignedEntries.map(([binId]) => binId);
      const dir = await getOutputDir();
      const res = await cropChannels(
        paths,
        dir,
        mode === "auto" ? undefined : top,
        mode === "auto" ? undefined : bottom,
        mode === "auto" ? undefined : left,
        mode === "auto" ? undefined : right,
        mode === "auto",
        binIds,
      );
      setResult(res);

      const cropped: Record<string, string> = {};
      const keys = res.cache_keys;
      if (keys && keys.length > 0) {
        alignedEntries.forEach(([binId], i) => {
          if (keys[i]) {
            cropped[binId] = keys[i];
          }
        });
      } else if (res.paths) {
        alignedEntries.forEach(([binId], i) => {
          if (res.paths[i]) {
            cropped[binId] = res.paths[i];
          }
        });
      }
      onCropped(cropped);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [alignedEntries, mode, top, bottom, left, right, onCropped]);

  const handleSkip = useCallback(() => {
    setSkipped(true);
    const passthrough: Record<string, string> = {};
    for (const [binId, path] of alignedEntries) {
      passthrough[binId] = path;
    }
    onCropped(passthrough);
  }, [alignedEntries, onCropped]);

  if (alignedEntries.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-600 text-xs">
        Run Alignment first to enable cropping.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <Scissors size={12} className="text-cyan-400" />
          <span className="text-xs text-zinc-300">Crop aligned channels</span>
        </div>
        <select value={mode} onChange={(e) => setMode(e.target.value as "auto" | "manual")} className="ab-select">
          <option value="auto">Auto-detect borders</option>
          <option value="manual">Manual margins</option>
        </select>
      </div>

      <div className="text-[10px] text-zinc-500">
        {alignedEntries.length} aligned channel(s) ready.
        {mode === "auto"
          ? " Auto-crop will find the intersection of valid data across all channels."
          : " Specify pixel margins to remove from each edge."}
      </div>

      {mode === "manual" && (
        <div className="grid grid-cols-2 gap-2">
          {([["Top", top, setTop], ["Bottom", bottom, setBottom], ["Left", left, setLeft], ["Right", right, setRight]] as const).map(
            ([label, val, setter]) => (
              <div key={label} className="flex items-center justify-between gap-2">
                <label className="text-[10px] text-zinc-400 w-12">{label}</label>
                <input
                  type="number"
                  min={0}
                  value={val}
                  onChange={(e) => setter(Math.max(0, parseInt(e.target.value) || 0))}
                  className="ab-input w-20 text-right"
                />
              </div>
            ),
          )}
        </div>
      )}

      <div className="flex items-center gap-2">
        <RunButton
          label="Apply Crop"
          runningLabel="Cropping..."
          running={loading}
          accent="cyan"
          onClick={handleCrop}
        />
        <button
          onClick={handleSkip}
          disabled={loading}
          className="px-3 py-1.5 rounded text-[10px] text-zinc-400 hover:text-zinc-200 border border-zinc-700/50 hover:border-zinc-600 transition-all disabled:opacity-40"
        >
          Skip
        </button>
      </div>

      {skipped && (
        <div className="text-[9px] text-zinc-500">
          Crop skipped. Aligned paths passed through directly.
        </div>
      )}

      {result && !skipped && (
        <div className="text-[9px] text-zinc-500">
          Cropped to {result.dimensions?.[0]}x{result.dimensions?.[1]}, margins [{result.crop_top}, {result.crop_bottom}, {result.crop_left}, {result.crop_right}], {result.elapsed_ms}ms
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}

      <div className="flex flex-col gap-1 pt-1 border-t border-zinc-800/30">
        <span className="text-[9px] text-zinc-600 uppercase tracking-wider">Channels</span>
        {alignedEntries.map(([binId, path]) => {
          const bin = state.bins.find((b) => b.id === binId);
          const hasCropped = !!state.croppedPaths[binId];
          const displayName = path.startsWith("__wizard_ch_")
            ? bin?.shortLabel ?? binId
            : path.split(/[/\\]/).pop();
          return (
            <div key={binId} className="flex items-center gap-1.5 py-0.5">
              <span className="w-2 h-2 rounded-full" style={{ background: bin?.color }} />
              <span className="text-[10px] text-zinc-300">{bin?.shortLabel}</span>
              <span className="text-[8px] text-zinc-700 font-mono truncate flex-1">{displayName}</span>
              {hasCropped && <span className="text-[8px] text-cyan-400/60">cropped</span>}
            </div>
          );
        })}
      </div>
    </div>
  );
}
