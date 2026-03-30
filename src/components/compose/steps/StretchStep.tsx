import { useState, useCallback } from "react";
import type { WizardState } from "../wizard.types";
import { Slider, RunButton, Toggle } from "../../ui";
import { restretchComposite } from "../../../services/compose.service";
import { maskedStretch, applyArcsinhStretch } from "../../../services/processing.service";
import { getPreviewUrl } from "../../../infrastructure/tauri/client";

interface StretchStepProps {
  state: WizardState;
  onStretchChange: (mode: WizardState["stretchMode"], factor?: number, target?: number) => void;
  onResult: (png: string | null) => void;
}

interface ChannelStf {
  shadow: number;
  midtone: number;
  highlight: number;
}

const DEFAULT_STF: ChannelStf = { shadow: 0, midtone: 0.5, highlight: 1 };

function resolveAnyChannelPath(state: WizardState): string | null {
  for (const bin of state.bins) {
    if (state.alignedPaths[bin.id]) return state.alignedPaths[bin.id];
    if (state.backgroundPaths[bin.id]) return state.backgroundPaths[bin.id];
    if (state.stackedPaths[bin.id]) return state.stackedPaths[bin.id];
    if (bin.files.length > 0) return bin.files[0];
  }
  return null;
}

export default function StretchStep({ state, onStretchChange, onResult }: StretchStepProps) {
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState("");
  const [linked, setLinked] = useState(state.linkedStf);
  const [stfR, setStfR] = useState<ChannelStf>({ ...DEFAULT_STF });
  const [stfG, setStfG] = useState<ChannelStf>({ ...DEFAULT_STF });
  const [stfB, setStfB] = useState<ChannelStf>({ ...DEFAULT_STF });

  const handleLinkedChange = useCallback((v: boolean) => {
    setLinked(v);
    if (v) {
      setStfG({ ...stfR });
      setStfB({ ...stfR });
    }
  }, [stfR]);

  const updateChannel = useCallback((ch: "r" | "g" | "b", param: keyof ChannelStf, val: number) => {
    const update = (prev: ChannelStf) => ({ ...prev, [param]: val });
    if (linked) {
      const synced = update(stfR);
      setStfR(synced);
      setStfG(synced);
      setStfB(synced);
    } else {
      if (ch === "r") setStfR(update);
      if (ch === "g") setStfG(update);
      if (ch === "b") setStfB(update);
    }
  }, [linked, stfR]);

  const handleModeChange = useCallback((mode: WizardState["stretchMode"]) => {
    onStretchChange(mode, state.stretchFactor, state.targetBackground);
  }, [state.stretchFactor, state.targetBackground, onStretchChange]);

  const handleFactorChange = useCallback((v: number) => {
    onStretchChange(state.stretchMode, v, state.targetBackground);
  }, [state.stretchMode, state.targetBackground, onStretchChange]);

  const handleTargetChange = useCallback((v: number) => {
    onStretchChange(state.stretchMode, state.stretchFactor, v);
  }, [state.stretchMode, state.stretchFactor, onStretchChange]);

  const handleRun = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      let res: any;

      if (state.compositeReady) {
        res = await restretchComposite("./output", stfR, stfG, stfB);
        if (res?.png_path) {
          const url = await getPreviewUrl(res.png_path);
          onResult(url);
        }
      } else if (state.stretchMode === "masked") {
        const path = resolveAnyChannelPath(state);
        if (!path) throw new Error("No channel path found");
        res = await maskedStretch(path, "./output", {
          iterations: 10,
          targetBackground: state.targetBackground,
          maskGrowth: state.maskGrowth,
          protectionAmount: state.maskProtection,
        });
        if (res?.previewUrl || res?.png_path) {
          onResult(res.previewUrl ?? res.png_path);
        }
      } else if (state.stretchMode === "arcsinh") {
        const path = resolveAnyChannelPath(state);
        if (!path) throw new Error("No channel path found");
        res = await applyArcsinhStretch(path, "./output", state.stretchFactor);
        if (res?.previewUrl || res?.png_path) {
          onResult(res.previewUrl ?? res.png_path);
        }
      } else {
        res = await restretchComposite("./output", stfR, stfG, stfB);
        if (res?.png_path) {
          const url = await getPreviewUrl(res.png_path);
          onResult(url);
        }
      }

      setResult(res);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [state, stfR, stfG, stfB, onResult]);

  return (
    <div className="flex flex-col gap-3 p-3">
      {state.compositeReady ? (
        <>
          <div className="text-[10px] text-emerald-400/70 bg-emerald-500/5 border border-emerald-500/10 rounded-md px-2 py-1.5">
            Operating on blended composite (R/G/B cached). Adjust STF params to re-stretch.
          </div>

          <Toggle label="Link channels" checked={linked} accent="amber" onChange={handleLinkedChange} />

          {linked ? (
            <div className="flex flex-col gap-2">
              <Slider label="Shadow" value={stfR.shadow} min={0} max={0.5} step={0.001} accent="amber"
                format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "shadow", v)} />
              <Slider label="Midtone" value={stfR.midtone} min={0.01} max={1} step={0.01} accent="amber"
                format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "midtone", v)} />
              <Slider label="Highlight" value={stfR.highlight} min={0.5} max={1} step={0.001} accent="amber"
                format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "highlight", v)} />
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              <div className="flex flex-col gap-1.5">
                <span className="text-[10px] font-medium text-red-400">R Channel</span>
                <Slider label="Shadow" value={stfR.shadow} min={0} max={0.5} step={0.001} accent="red"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "shadow", v)} />
                <Slider label="Midtone" value={stfR.midtone} min={0.01} max={1} step={0.01} accent="red"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "midtone", v)} />
                <Slider label="Highlight" value={stfR.highlight} min={0.5} max={1} step={0.001} accent="red"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "highlight", v)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <span className="text-[10px] font-medium text-green-400">G Channel</span>
                <Slider label="Shadow" value={stfG.shadow} min={0} max={0.5} step={0.001} accent="green"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("g", "shadow", v)} />
                <Slider label="Midtone" value={stfG.midtone} min={0.01} max={1} step={0.01} accent="green"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("g", "midtone", v)} />
                <Slider label="Highlight" value={stfG.highlight} min={0.5} max={1} step={0.001} accent="green"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("g", "highlight", v)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <span className="text-[10px] font-medium text-blue-400">B Channel</span>
                <Slider label="Shadow" value={stfB.shadow} min={0} max={0.5} step={0.001} accent="blue"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("b", "shadow", v)} />
                <Slider label="Midtone" value={stfB.midtone} min={0.01} max={1} step={0.01} accent="blue"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("b", "midtone", v)} />
                <Slider label="Highlight" value={stfB.highlight} min={0.5} max={1} step={0.001} accent="blue"
                  format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("b", "highlight", v)} />
              </div>
            </div>
          )}
        </>
      ) : (
        <>
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Stretch Mode</label>
            <select value={state.stretchMode} onChange={(e) => handleModeChange(e.target.value as WizardState["stretchMode"])} className="ab-select">
              <option value="masked">Masked Stretch (star-protected)</option>
              <option value="arcsinh">Arcsinh Stretch</option>
              <option value="auto_stf">Auto STF</option>
            </select>
          </div>

          {state.stretchMode === "masked" && (
            <div className="flex flex-col gap-2">
              <Slider label="Target Background" value={state.targetBackground} min={0.05} max={0.5} step={0.01} accent="amber"
                format={(v) => v.toFixed(2)} onChange={handleTargetChange} />
              <div className="text-[9px] text-zinc-600">
                Uses star mask (growth={state.maskGrowth.toFixed(1)}, protection={((state.maskProtection) * 100).toFixed(0)}%)
              </div>
            </div>
          )}

          {state.stretchMode === "arcsinh" && (
            <Slider label="Stretch Factor" value={state.stretchFactor} min={1} max={500} step={1} accent="amber"
              format={(v) => `${v}`} onChange={handleFactorChange} />
          )}

          {state.stretchMode === "auto_stf" && (
            <div className="text-[10px] text-zinc-500">
              Auto STF will compute optimal shadow/midtone/highlight per channel based on image statistics.
            </div>
          )}
        </>
      )}

      <RunButton
        label={state.compositeReady ? "Re-stretch Composite" : `Apply ${state.stretchMode === "masked" ? "Masked" : state.stretchMode === "arcsinh" ? "Arcsinh" : "Auto STF"} Stretch`}
        runningLabel="Stretching..."
        running={loading}
        accent="amber"
        onClick={handleRun}
      />

      {result && (
        <div className="text-[9px] text-zinc-500">
          {result.elapsed_ms}ms
          {result.iterations_run && `, ${result.iterations_run} iterations`}
          {result.converged !== undefined && `, ${result.converged ? "converged" : "not converged"}`}
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
