import { useState, useCallback } from "react";
import type { WizardState } from "../wizard";
import { Slider, RunButton } from "../../ui";

interface MaskStepProps {
  state: WizardState;
  onMask: (path: string | null) => void;
  onMaskParams: (growth: number, protection: number) => void;
}

export default function MaskStep({ state, onMask, onMaskParams }: MaskStepProps) {
  const [growth, setGrowth] = useState(state.maskGrowth);
  const [protection, setProtection] = useState(state.maskProtection);

  const handleGrowthChange = useCallback((v: number) => {
    setGrowth(v);
    onMaskParams(v, protection);
  }, [protection, onMaskParams]);

  const handleProtectionChange = useCallback((v: number) => {
    setProtection(v);
    onMaskParams(growth, v);
  }, [growth, onMaskParams]);

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="text-xs text-zinc-400">
        Star mask protects bright stars during stretch. The mask is generated automatically
        by the Masked Stretch step. Adjust parameters here to control mask behavior.
      </div>

      <div className="flex flex-col gap-2">
        <Slider label="Mask Growth" value={growth} min={0.5} max={10.0} step={0.1} accent="rose"
          format={(v) => v.toFixed(1)} onChange={handleGrowthChange} />
        <Slider label="Protection Amount" value={protection} min={0.0} max={1.0} step={0.01} accent="rose"
          format={(v) => `${(v * 100).toFixed(0)}%`} onChange={handleProtectionChange} />
      </div>

      {state.starMaskPath && (
        <div className="flex items-center gap-2 p-2 rounded-lg bg-rose-600/10 border border-rose-500/20">
          <span className="w-2 h-2 rounded-full bg-rose-400" />
          <span className="text-[10px] text-rose-300 flex-1 truncate">{state.starMaskPath.split(/[/\\]/).pop()}</span>
          <button onClick={() => onMask(null)} className="text-[9px] text-zinc-500 hover:text-red-400">Clear</button>
        </div>
      )}

      <div className="text-[9px] text-zinc-600 pt-1 border-t border-zinc-800/30">
        The star mask will be applied during the Stretch step. You can also import
        a .segm FITS mask from external software.
      </div>
    </div>
  );
}
