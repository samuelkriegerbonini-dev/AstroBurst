import { withSymmetryPoint, withShadowProtect, withHighlightProtect } from "./ghsSettings";
import { GHS_LIMITS, type GhsSettings } from "../../services/processing";
import { Slider } from "../ui";

export type GhsUpdate = (update: (settings: GhsSettings) => GhsSettings) => void;

interface GhsStretchControlsProps {
  settings: GhsSettings;
  disabled: boolean;
  onChange: GhsUpdate;
}

const ACCENT = "amber";




function threeDecimals(v: number): string {
  return v.toFixed(3);
}

export default function GhsStretchControls({ settings, disabled, onChange }: GhsStretchControlsProps) {
  const { stretchFactor, localIntensity, symmetryPoint, shadowProtect, highlightProtect } = GHS_LIMITS;
  return (
    <div className="flex flex-col gap-2">
      <Slider label="Stretch (D)" value={settings.stretchFactor} min={stretchFactor.min} max={stretchFactor.max} step={stretchFactor.step} disabled={disabled} accent={ACCENT} format={(v) => v.toFixed(2)} onChange={(v) => onChange((s) => ({ ...s, stretchFactor: v }))} />
      <Slider label="Local Intensity (b)" value={settings.localIntensity} min={localIntensity.min} max={localIntensity.max} step={localIntensity.step} disabled={disabled} accent={ACCENT} format={(v) => v.toFixed(1)} onChange={(v) => onChange((s) => ({ ...s, localIntensity: v }))} />
      <Slider label="Symmetry Point (SP)" value={settings.symmetryPoint} min={symmetryPoint.min} max={symmetryPoint.max} step={symmetryPoint.step} disabled={disabled} accent={ACCENT} format={threeDecimals} onChange={(v) => onChange((s) => withSymmetryPoint(s, v))} />
      <Slider label="Shadow Protect (LP)" value={settings.shadowProtect} min={shadowProtect.min} max={shadowProtect.max} step={shadowProtect.step} disabled={disabled} accent={ACCENT} format={threeDecimals} onChange={(v) => onChange((s) => withShadowProtect(s, v))} />
      <Slider label="Highlight Protect (HP)" value={settings.highlightProtect} min={highlightProtect.min} max={highlightProtect.max} step={highlightProtect.step} disabled={disabled} accent={ACCENT} format={threeDecimals} onChange={(v) => onChange((s) => withHighlightProtect(s, v))} />
      <div className="text-[9px] text-zinc-600 px-0.5">
        On linear data set SP near the background (often 0.002 to 0.02), then raise D; an SP above the faint signal leaves it black.
        b below 0 lifts faint signal hardest (b = -1 is logarithmic), b = 0 is exponential, b above 0 focuses contrast near SP.
        LP and HP keep shadows and highlights linear.
      </div>
    </div>
  );
}
