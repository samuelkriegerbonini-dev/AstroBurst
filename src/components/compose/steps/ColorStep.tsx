import { useState, useCallback } from "react";
import type { WizardState } from "../wizard.types";
import { applyScnr } from "../../../services/compose.service";
import { Slider, Toggle, RunButton } from "../../ui";

interface ColorStepProps {
  state: WizardState;
  onScnrChange: (enabled: boolean, amount?: number) => void;
  onResult: (png: string | null) => void;
}

export default function ColorStep({ state, onScnrChange, onResult }: ColorStepProps) {
  const [method, setMethod] = useState("average");
  const [preserveLum, setPreserveLum] = useState(false);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState("");

  const handleToggle = useCallback((val: boolean) => {
    onScnrChange(val, state.scnrAmount);
  }, [state.scnrAmount, onScnrChange]);

  const handleAmountChange = useCallback((val: number) => {
    onScnrChange(state.scnrEnabled, val);
  }, [state.scnrEnabled, onScnrChange]);

  const handleApply = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const res = await applyScnr("./output", {
        method,
        amount: state.scnrAmount,
        preserveLuminance: preserveLum,
      });
      setResult(res);
      if (res?.previewUrl || res?.png_path) {
        onResult(res.previewUrl ?? res.png_path);
      }
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [method, state.scnrAmount, preserveLum, onResult]);

  return (
    <div className="flex flex-col gap-3 p-3">
      {!state.compositeReady && (
        <div className="text-[10px] text-amber-400/70 bg-amber-500/5 border border-amber-500/10 rounded-md px-2 py-1.5">
          Run Blend first to create a composite before applying color adjustments.
        </div>
      )}

      <Toggle label="SCNR (Green Removal)" checked={state.scnrEnabled} accent="purple" onChange={handleToggle} />

      {state.scnrEnabled && (
        <div className="flex flex-col gap-2 pl-2">
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Method</label>
            <select value={method} onChange={(e) => setMethod(e.target.value)} className="ab-select">
              <option value="average">Average Neutral</option>
              <option value="maximum">Maximum Neutral</option>
            </select>
          </div>
          <Slider label="Amount" value={state.scnrAmount} min={0} max={1} step={0.05} accent="purple"
            format={(v) => `${(v * 100).toFixed(0)}%`} onChange={handleAmountChange} />
          <Toggle label="Preserve Luminance" checked={preserveLum} accent="purple" onChange={setPreserveLum} />
        </div>
      )}

      {state.scnrEnabled && (
        <RunButton
          label="Apply SCNR"
          runningLabel="Applying..."
          running={loading}
          disabled={!state.compositeReady}
          accent="purple"
          onClick={handleApply}
        />
      )}

      {result && (
        <div className="text-[9px] text-zinc-500">
          SCNR applied, {result.elapsed_ms}ms
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}

      <div className="text-[9px] text-zinc-600 pt-2 border-t border-zinc-800/30">
        SCNR operates on the cached composite R/G/B. Requires a blended composite.
      </div>
    </div>
  );
}
