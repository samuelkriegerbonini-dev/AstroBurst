import { useState, useCallback, useMemo } from "react";
import type { WizardState } from "../wizard";
import { alignChannels } from "../../../services/compose";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton } from "../../ui";

interface AlignStepProps {
  state: WizardState;
  onAligned: (paths: Record<string, string>) => void;
}

function resolveChannelPath(state: WizardState, binId: string): string | null {
  if (state.backgroundPaths[binId]) return state.backgroundPaths[binId];
  if (state.stackedPaths[binId]) return state.stackedPaths[binId];
  const bin = state.bins.find((b) => b.id === binId);
  if (bin && bin.files.length > 0) return bin.files[0];
  return null;
}

export default function AlignStep({ state, onAligned }: AlignStepProps) {
  const [method, setMethod] = useState("phase_correlation");
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState("");

  const activeBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 0),
    [state.bins],
  );

  const channelPaths = useMemo(() => {
    const entries: { binId: string; path: string }[] = [];
    for (const bin of activeBins) {
      const p = resolveChannelPath(state, bin.id);
      if (p) entries.push({ binId: bin.id, path: p });
    }
    return entries;
  }, [activeBins, state]);

  const handleAlign = useCallback(async () => {
    if (channelPaths.length < 2) return;
    setLoading(true);
    setError("");
    try {
      const paths = channelPaths.map((c) => c.path);
      const dir = await getOutputDir();
      const res = await alignChannels(paths, dir, method);
      setResult(res);
      if (res.channels) {
        const aligned: Record<string, string> = {};
        res.channels.forEach((ch: any, i: number) => {
          if (ch.path && channelPaths[i]) {
            aligned[channelPaths[i].binId] = ch.path;
          }
        });
        onAligned(aligned);
      }
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [channelPaths, method, onAligned]);

  if (channelPaths.length < 2) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-600 text-xs">
        Need at least 2 channels to align.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Method</label>
        <select value={method} onChange={(e) => setMethod(e.target.value)} className="ab-select">
          <option value="phase_correlation">Phase Correlation (sub-pixel)</option>
          <option value="affine">Star-based Affine (rotation)</option>
        </select>
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-[9px] text-zinc-600 uppercase tracking-wider">Channels to align</span>
        {channelPaths.map((c, i) => {
          const bin = activeBins.find((b) => b.id === c.binId);
          const ch = result?.channels?.[i];
          const offset = ch?.offset;
          return (
            <div key={c.binId} className="flex items-center justify-between py-1">
              <div className="flex items-center gap-1.5">
                <span className="w-2 h-2 rounded-full" style={{ background: bin?.color }} />
                <span className="text-[10px] text-zinc-300">{bin?.shortLabel}</span>
                {i === 0 && <span className="text-[8px] text-sky-400/60 ml-1">REF</span>}
              </div>
              <div className="flex items-center gap-2">
                {ch?.matched_stars > 0 && (
                  <span className="text-[8px] font-mono text-sky-400/50">
                    {ch.inliers}/{ch.matched_stars} stars, {ch.residual_px?.toFixed(2)}px
                  </span>
                )}
                {ch?.confidence > 0 && ch?.matched_stars === 0 && (
                  <span className="text-[8px] font-mono text-sky-400/50">
                    conf={ch.confidence?.toFixed(3)}
                  </span>
                )}
                {offset && (
                  <span className="text-[9px] font-mono text-zinc-600">
                    [{offset[0]?.toFixed(1)}, {offset[1]?.toFixed(1)}]
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>

      <RunButton
        label="Align Channels"
        runningLabel="Aligning..."
        running={loading}
        disabled={channelPaths.length < 2}
        accent="sky"
        onClick={handleAlign}
      />

      {result && (
        <div className="text-[9px] text-zinc-500">
          {result.align_method}, {result.dimensions?.[0]}x{result.dimensions?.[1]}, {result.elapsed_ms}ms
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
