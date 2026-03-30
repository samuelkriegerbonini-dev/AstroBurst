import { useState, useCallback, useMemo, lazy, Suspense } from "react";
import { Loader2 } from "lucide-react";
import type { WizardState } from "../wizard.types";
import { Slider, RunButton } from "../../ui";
import { calibrateComposite } from "../../../services/compose.service";
import { getPreviewUrl } from "../../../infrastructure/tauri/client";

const SpccPanel = lazy(() => import("../SpccPanel"));

interface CalibrateStepProps {
  state: WizardState;
  onWbChange: (mode: WizardState["wbMode"], r?: number, g?: number, b?: number) => void;
  onResult: (png: string | null) => void;
}

function resolveChannelPath(state: WizardState, binId: string): string | null {
  if (state.alignedPaths[binId]) return state.alignedPaths[binId];
  if (state.backgroundPaths[binId]) return state.backgroundPaths[binId];
  if (state.stackedPaths[binId]) return state.stackedPaths[binId];
  const bin = state.bins.find((b) => b.id === binId);
  if (bin && bin.files.length > 0) return bin.files[0];
  return null;
}

export default function CalibrateStep({ state, onWbChange, onResult }: CalibrateStepProps) {
  const [localR, setLocalR] = useState(state.wbR);
  const [localG, setLocalG] = useState(state.wbG);
  const [localB, setLocalB] = useState(state.wbB);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [elapsed, setElapsed] = useState<number | null>(null);

  const rgbBins = useMemo(() => {
    const r = state.bins.find((b) => b.id === "r" || b.id === "ha");
    const g = state.bins.find((b) => b.id === "g" || b.id === "oiii");
    const bBin = state.bins.find((b) => b.id === "b" || b.id === "sii");
    return { r, g, b: bBin };
  }, [state.bins]);

  const rPath = resolveChannelPath(state, rgbBins.r?.id ?? "");
  const gPath = resolveChannelPath(state, rgbBins.g?.id ?? "");
  const bPath = resolveChannelPath(state, rgbBins.b?.id ?? "");

  const handleModeChange = useCallback((mode: WizardState["wbMode"]) => {
    onWbChange(mode, localR, localG, localB);
  }, [onWbChange, localR, localG, localB]);

  const handleManualChange = useCallback((axis: "r" | "g" | "b", val: number) => {
    const nr = axis === "r" ? val : localR;
    const ng = axis === "g" ? val : localG;
    const nb = axis === "b" ? val : localB;
    if (axis === "r") setLocalR(val);
    if (axis === "g") setLocalG(val);
    if (axis === "b") setLocalB(val);
    onWbChange("manual", nr, ng, nb);
  }, [localR, localG, localB, onWbChange]);

  const handleSpccFactors = useCallback((r: number, g: number, b: number) => {
    setLocalR(r);
    setLocalG(g);
    setLocalB(b);
    onWbChange("manual", r, g, b);
  }, [onWbChange]);

  const handleRunCalibrate = useCallback(async () => {
    setLoading(true);
    setError("");
    setElapsed(null);
    try {
      const res = await calibrateComposite("./output", localR, localG, localB);
      if (res?.png_path) {
        const url = await getPreviewUrl(res.png_path);
        onResult(url);
      }
      if (res?.elapsed_ms) setElapsed(res.elapsed_ms);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [localR, localG, localB, onResult]);

  const isFactorsNeutral = localR === 1.0 && localG === 1.0 && localB === 1.0;

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">White Balance</label>
        <select value={state.wbMode} onChange={(e) => handleModeChange(e.target.value as WizardState["wbMode"])} className="ab-select">
          <option value="auto">Auto (Stability)</option>
          <option value="spcc">SPCC (Spectrophotometric)</option>
          <option value="manual">Manual</option>
          <option value="none">None</option>
        </select>
      </div>

      {state.compositeReady && (
        <div className="text-[10px] text-cyan-400/70 bg-cyan-500/5 border border-cyan-500/10 rounded-md px-2 py-1.5">
          Operating on blended composite (R/G/B cached). Adjust factors and apply.
        </div>
      )}

      {(state.wbMode === "manual" || state.wbMode === "auto") && (
        <div className="flex flex-col gap-2 pl-2">
          <Slider label="R" value={localR} min={0.5} max={2.0} step={0.01} accent="red"
            format={(v) => v.toFixed(2)} onChange={(v) => handleManualChange("r", v)} />
          <Slider label="G" value={localG} min={0.5} max={2.0} step={0.01} accent="green"
            format={(v) => v.toFixed(2)} onChange={(v) => handleManualChange("g", v)} />
          <Slider label="B" value={localB} min={0.5} max={2.0} step={0.01} accent="blue"
            format={(v) => v.toFixed(2)} onChange={(v) => handleManualChange("b", v)} />
        </div>
      )}

      {state.wbMode === "spcc" && rPath && gPath && bPath && (
        <Suspense fallback={<div className="flex items-center gap-2 py-4 text-zinc-600 text-xs"><Loader2 size={12} className="animate-spin" /> Loading SPCC...</div>}>
          <SpccPanel
            rPath={{ path: rPath } as any}
            gPath={{ path: gPath } as any}
            bPath={{ path: bPath } as any}
            onFactorsReady={handleSpccFactors}
          />
        </Suspense>
      )}

      {state.wbMode === "spcc" && (!rPath || !gPath || !bPath) && (
        <div className="text-[10px] text-amber-400/80 py-2">
          SPCC needs R/G/B (or Ha/OIII/SII) channels assigned and processed.
        </div>
      )}

      {state.wbMode === "none" && (
        <div className="text-[10px] text-zinc-500 py-1">
          White balance disabled. Factors will not be applied.
        </div>
      )}

      {state.compositeReady && state.wbMode !== "none" && (
        <RunButton
          label={isFactorsNeutral ? "Apply WB (neutral)" : `Apply WB (${localR.toFixed(2)} / ${localG.toFixed(2)} / ${localB.toFixed(2)})`}
          runningLabel="Calibrating..."
          running={loading}
          accent="cyan"
          onClick={handleRunCalibrate}
        />
      )}

      {elapsed !== null && (
        <div className="text-[9px] text-zinc-500">{elapsed}ms</div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}

      <div className="text-[9px] text-zinc-600 pt-1 border-t border-zinc-800/30">
        Current factors: R={state.wbR.toFixed(3)} G={state.wbG.toFixed(3)} B={state.wbB.toFixed(3)}
      </div>
    </div>
  );
}
