import { useCallback } from "react";
import { Slider, Toggle } from "../ui";
import { useCompositeStf, useCompositeActions } from "../../context/CompositeContext";
import type { StfParams } from "../../shared/types";
import StfHistogram from "../compose/StfHistogram";

const DEFAULT_STF: StfParams = { shadow: 0, midtone: 0.5, highlight: 1 };
const fmt4 = (v: number) => v.toFixed(4);

const HIST_RGB = [
  { key: "__composite_r", color: "#ef4444" },
  { key: "__composite_g", color: "#22c55e" },
  { key: "__composite_b", color: "#3b82f6" },
];

export default function RgbStfPanel() {
  const {
    compositeStfR, compositeStfG, compositeStfB,
    compositeStfLinked,
    compositeAutoStfR, compositeAutoStfG, compositeAutoStfB,
  } = useCompositeStf();
  const { setCompositeStf, setCompositeStfLinked } = useCompositeActions();

  const updateChannel = useCallback(
    (ch: "r" | "g" | "b", param: keyof StfParams, val: number) => {
      if (compositeStfLinked) {
        const next = { ...compositeStfR, [param]: val };
        setCompositeStf(next, next, next);
      } else {
        setCompositeStf(
          ch === "r" ? { ...compositeStfR, [param]: val } : compositeStfR,
          ch === "g" ? { ...compositeStfG, [param]: val } : compositeStfG,
          ch === "b" ? { ...compositeStfB, [param]: val } : compositeStfB,
        );
      }
    },
    [compositeStfLinked, compositeStfR, compositeStfG, compositeStfB, setCompositeStf],
  );

  const handleLinkedChange = useCallback(
    (v: boolean) => {
      setCompositeStfLinked(v);
      if (v) setCompositeStf(compositeStfR, compositeStfR, compositeStfR);
    },
    [setCompositeStfLinked, setCompositeStf, compositeStfR],
  );

  const handleReset = useCallback(() => {
    const r = compositeAutoStfR ?? DEFAULT_STF;
    const g = compositeAutoStfG ?? compositeAutoStfR ?? DEFAULT_STF;
    const b = compositeAutoStfB ?? compositeAutoStfR ?? DEFAULT_STF;
    setCompositeStf(r, g, b);
  }, [compositeAutoStfR, compositeAutoStfG, compositeAutoStfB, setCompositeStf]);

  const channel = (title: string, labelClass: string, accent: string, ch: "r" | "g" | "b", stf: StfParams) => (
    <div className="flex flex-col gap-1.5">
      <span className={`text-[10px] font-medium ${labelClass}`}>{title}</span>
      <Slider label="Shadow" value={stf.shadow} min={0} max={0.5} step={0.0001} accent={accent}
              format={fmt4} onChange={(v) => updateChannel(ch, "shadow", v)} />
      <Slider label="Midtone" value={stf.midtone} min={0.0001} max={1} step={0.0001} scale="log" accent={accent}
              format={fmt4} onChange={(v) => updateChannel(ch, "midtone", v)} />
      <Slider label="Highlight" value={stf.highlight} min={0.5} max={1} step={0.001} accent={accent}
              format={fmt4} onChange={(v) => updateChannel(ch, "highlight", v)} />
    </div>
  );

  return (
    <div className="flex flex-col gap-2 p-3 rounded-lg border border-violet-600/20 bg-violet-900/10">
      <div className="flex items-center justify-between">
        <span className="text-[10px] font-medium text-violet-300">RGB Channel STF · live on GPU</span>
        {compositeAutoStfR !== null && (
          <button
            onClick={handleReset}
            className="px-2 py-0.5 rounded text-[9px] font-medium bg-zinc-800/60 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-700/60 transition-all"
          >
            Reset to auto
          </button>
        )}
      </div>

      <Toggle label="Link channels" checked={compositeStfLinked} accent="amber" onChange={handleLinkedChange} />

      <StfHistogram
        channels={HIST_RGB}
        shadow={compositeStfR.shadow}
        midtone={compositeStfR.midtone}
        highlight={compositeStfR.highlight}
      />

      {compositeStfLinked ? (
        <div className="flex flex-col gap-2">
          <Slider label="Shadow" value={compositeStfR.shadow} min={0} max={0.5} step={0.0001} accent="amber"
                  format={fmt4} onChange={(v) => updateChannel("r", "shadow", v)} />
          <Slider label="Midtone" value={compositeStfR.midtone} min={0.0001} max={1} step={0.0001} scale="log" accent="amber"
                  format={fmt4} onChange={(v) => updateChannel("r", "midtone", v)} />
          <Slider label="Highlight" value={compositeStfR.highlight} min={0.5} max={1} step={0.001} accent="amber"
                  format={fmt4} onChange={(v) => updateChannel("r", "highlight", v)} />
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {channel("R Channel", "text-red-400", "red", "r", compositeStfR)}
          {channel("G Channel", "text-green-400", "green", "g", compositeStfG)}
          {channel("B Channel", "text-blue-400", "blue", "b", compositeStfB)}
        </div>
      )}
    </div>
  );
}
