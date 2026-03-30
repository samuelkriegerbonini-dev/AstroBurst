import { useState, useCallback, useMemo } from "react";
import type { WizardState, BlendWeight, FrequencyBin } from "../wizard.types";
import { BLEND_PRESETS } from "../wizard.types";
import { blendChannels } from "../../../services/compose";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton, Toggle } from "../../ui";

const CANONICAL_WAVELENGTH: Record<string, number> = {
  sii: 673, ha: 656, nii: 658, oiii: 502,
  r: 620, g: 530, b: 470, l: 550,
};

function binWavelength(bin: FrequencyBin): number {
  if (bin.wavelength) return bin.wavelength;
  return CANONICAL_WAVELENGTH[bin.id] ?? 550;
}

function resolvePresetWeights(
  preset: { weights: BlendWeight[] },
  filledBins: FrequencyBin[],
): BlendWeight[] | null {
  const exact = preset.weights.filter((w) =>
    filledBins.some((b) => b.id === w.channelId)
  );
  if (exact.length > 0) return exact;

  if (filledBins.length < 2) return null;

  const presetWithWl = preset.weights.map((w) => ({
    ...w,
    wl: CANONICAL_WAVELENGTH[w.channelId] ?? 550,
  }));
  const sortedPreset = [...presetWithWl].sort((a, b) => b.wl - a.wl);

  const sortedBins = [...filledBins].sort((a, b) => binWavelength(b) - binWavelength(a));

  const resolved: BlendWeight[] = sortedPreset
    .slice(0, sortedBins.length)
    .map((pw, i) => ({
      channelId: sortedBins[i].id,
      r: pw.r,
      g: pw.g,
      b: pw.b,
    }));

  return resolved.length >= 2 ? resolved : null;
}

interface BlendStepProps {
  state: WizardState;
  onWeightsChange: (weights: BlendWeight[], preset: string) => void;
  onCompositeReady: (previewUrl: string | null, stfR?: any, stfG?: any, stfB?: any, lumFitsPath?: string | null) => void;
}

function resolveChannelPath(state: WizardState, binId: string): string | null {
  if (state.alignedPaths[binId]) return state.alignedPaths[binId];
  if (state.backgroundPaths[binId]) return state.backgroundPaths[binId];
  if (state.stackedPaths[binId]) return state.stackedPaths[binId];
  const bin = state.bins.find((b) => b.id === binId);
  if (bin && bin.files.length > 0) return bin.files[0];
  return null;
}

export default function BlendStep({ state, onWeightsChange, onCompositeReady }: BlendStepProps) {
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState("");
  const [autoStretch, setAutoStretch] = useState(true);
  const [linkedStf, setLinkedStf] = useState(state.linkedStf);

  const filledBins = useMemo(() => state.bins.filter((b) => b.files.length > 0), [state.bins]);

  const handlePreset = useCallback((presetId: string) => {
    const preset = BLEND_PRESETS[presetId];
    if (!preset) return;
    const resolved = resolvePresetWeights(preset, filledBins);
    if (resolved) onWeightsChange(resolved, presetId);
  }, [filledBins, onWeightsChange]);

  const handleWeightChange = useCallback((channelId: string, axis: "r" | "g" | "b", value: number) => {
    const next = state.blendWeights.map((w) =>
      w.channelId === channelId ? { ...w, [axis]: value } : w
    );
    onWeightsChange(next, "custom");
  }, [state.blendWeights, onWeightsChange]);

  const activeWeights = useMemo(() => {
    const existing = new Set(state.blendWeights.map((w) => w.channelId));
    const base = [...state.blendWeights];
    for (const bin of filledBins) {
      if (!existing.has(bin.id)) {
        base.push({ channelId: bin.id, r: 0, g: 0, b: 0 });
      }
    }
    return base.filter((w) => filledBins.some((b) => b.id === w.channelId));
  }, [state.blendWeights, filledBins]);

  const handleRunBlend = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const channelOrder: string[] = [];
      const paths: string[] = [];
      for (const bin of filledBins) {
        const p = resolveChannelPath(state, bin.id);
        if (p) {
          channelOrder.push(bin.id);
          paths.push(p);
        }
      }

      if (paths.length < 2) {
        throw new Error("Need at least 2 channel paths to blend");
      }

      const backendWeights = activeWeights
        .filter((w) => w.r > 0 || w.g > 0 || w.b > 0)
        .map((w) => {
          const idx = channelOrder.indexOf(w.channelId);
          if (idx === -1) return null;
          return { channelIdx: idx, r: w.r, g: w.g, b: w.b };
        })
        .filter(Boolean) as { channelIdx: number; r: number; g: number; b: number }[];

      if (backendWeights.length === 0) {
        throw new Error("No weights assigned. Adjust the weight matrix.");
      }

      const dir = await getOutputDir();
      const res = await blendChannels(paths, backendWeights, dir, {
        preset: state.blendPreset,
        autoStretch,
        linkedStf,
      });

      setResult(res);

      const stfR = res.stf_r ?? null;
      const stfG = res.stf_g ?? null;
      const stfB = res.stf_b ?? null;
      const previewUrl = res.previewUrl ?? res.png_path ?? null;
      const lumFitsPath = res.lum_fits_path ?? null;

      onCompositeReady(previewUrl, stfR, stfG, stfB, lumFitsPath);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [filledBins, state, activeWeights, autoStretch, linkedStf, onCompositeReady]);

  if (filledBins.length < 2) {
    return (
      <div className="flex items-center justify-center py-12 text-zinc-600 text-xs">
        Assign at least 2 channels in Step 1 to enable blending.
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex flex-wrap gap-1.5">
        {Object.entries(BLEND_PRESETS).map(([id, preset]) => {
          const isActive = state.blendPreset === id;
          return (
            <button key={id} onClick={() => handlePreset(id)}
              className={`px-2.5 py-1.5 rounded-md text-[10px] font-medium transition-all ${
                isActive
                  ? "bg-amber-500/20 text-amber-300 ring-1 ring-amber-500/30"
                  : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
              }`}>
              <div className="font-semibold">{preset.label}</div>
              <div className="text-[8px] opacity-60">{preset.desc}</div>
            </button>
          );
        })}
      </div>

      <div className="flex flex-col gap-0.5">
        <div className="grid grid-cols-[1fr_60px_60px_60px] gap-1 px-1 text-[9px] text-zinc-600 uppercase tracking-wider">
          <span>Channel</span>
          <span className="text-center text-red-400/60">R</span>
          <span className="text-center text-green-400/60">G</span>
          <span className="text-center text-blue-400/60">B</span>
        </div>

        {activeWeights.map((w) => {
          const bin = state.bins.find((b) => b.id === w.channelId);
          if (!bin) return null;
          return (
            <div key={w.channelId} className="grid grid-cols-[1fr_60px_60px_60px] gap-1 items-center py-1 border-b border-zinc-800/20">
              <div className="flex items-center gap-1.5">
                <span className="w-2 h-2 rounded-full" style={{ background: bin.color }} />
                <span className="text-[10px] text-zinc-300">{bin.shortLabel}</span>
              </div>
              {(["r", "g", "b"] as const).map((axis) => {
                const val = w[axis];
                const colors = { r: "#ef4444", g: "#22c55e", b: "#3b82f6" };
                return (
                  <div key={axis} className="flex flex-col items-center gap-0.5">
                    <input
                      type="range"
                      min={0} max={1} step={0.05}
                      value={val}
                      onChange={(e) => handleWeightChange(w.channelId, axis, parseFloat(e.target.value))}
                      className="w-full h-1 rounded-full appearance-none cursor-pointer"
                      style={{
                        background: `linear-gradient(to right, ${colors[axis]}${Math.round(val * 255).toString(16).padStart(2, "0")} ${val * 100}%, rgba(63,63,70,0.3) ${val * 100}%)`,
                      }}
                    />
                    <span className="text-[8px] font-mono text-zinc-600">{val.toFixed(2)}</span>
                  </div>
                );
              })}
            </div>
          );
        })}
      </div>

      <div className="flex items-center gap-4 pt-1">
        <Toggle label="Auto Stretch" checked={autoStretch} accent="amber" onChange={setAutoStretch} />
        {autoStretch && (
          <Toggle label="Linked STF" checked={linkedStf} accent="amber" onChange={setLinkedStf} />
        )}
      </div>

      <RunButton
        label="Run Blend"
        runningLabel="Blending..."
        running={loading}
        accent="amber"
        onClick={handleRunBlend}
      />

      <div className="flex items-center gap-2 pt-1">
        <div className="flex-1 flex items-center gap-1 text-[9px] text-zinc-600 flex-wrap">
          {activeWeights.filter((w) => w.r > 0 || w.g > 0 || w.b > 0).map((w) => {
            const bin = state.bins.find((b) => b.id === w.channelId);
            if (!bin) return null;
            const parts = [];
            if (w.r > 0) parts.push(`${(w.r * 100).toFixed(0)}%R`);
            if (w.g > 0) parts.push(`${(w.g * 100).toFixed(0)}%G`);
            if (w.b > 0) parts.push(`${(w.b * 100).toFixed(0)}%B`);
            return (
              <span key={w.channelId} className="flex items-center gap-0.5">
                <span className="w-1.5 h-1.5 rounded-full" style={{ background: bin.color }} />
                {bin.shortLabel}&rarr;{parts.join("+")}
              </span>
            );
          })}
        </div>
      </div>

      {result && (
        <div className="text-[9px] text-zinc-500">
          {result.channel_count} channels, {result.dimensions?.[0]}x{result.dimensions?.[1]}, {result.elapsed_ms}ms
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
