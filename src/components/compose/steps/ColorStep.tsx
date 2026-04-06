import { useState, useCallback, useMemo } from "react";
import type { WizardState } from "../wizard";
import { applyToneComposite, type CurvePoint } from "../../../services/tone";
import { getOutputDir } from "../../../infrastructure/tauri";
import { getPreviewUrl } from "../../../infrastructure/tauri/client";
import { Slider, Toggle, RunButton } from "../../ui";
import CurveEditor from "../CurveEditor";

interface ColorStepProps {
  state: WizardState;
  onScnrChange: (enabled: boolean, amount?: number) => void;
  onResult: (png: string | null) => void;
}

type CurveChannel = "linked" | "r" | "g" | "b";

const DEFAULT_CURVE: CurvePoint[] = [
  { x: 0, y: 0 },
  { x: 1, y: 1 },
];

const CHANNEL_COLORS: Record<CurveChannel, string> = {
  linked: "#14b8a6",
  r: "#ef4444",
  g: "#22c55e",
  b: "#3b82f6",
};

function isIdentity(pts: CurvePoint[]): boolean {
  if (pts.length !== 2) return false;
  return (
    Math.abs(pts[0].x) < 1e-4 &&
    Math.abs(pts[0].y) < 1e-4 &&
    Math.abs(pts[1].x - 1) < 1e-4 &&
    Math.abs(pts[1].y - 1) < 1e-4
  );
}

export default function ColorStep({ state, onScnrChange, onResult }: ColorStepProps) {
  const [method, setMethod] = useState("average");
  const [preserveLum, setPreserveLum] = useState(false);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [error, setError] = useState("");

  const [curveMode, setCurveMode] = useState<CurveChannel>("linked");
  const [curveLinked, setCurveLinked] = useState<CurvePoint[]>(DEFAULT_CURVE.map((p) => ({ ...p })));
  const [curveR, setCurveR] = useState<CurvePoint[]>(DEFAULT_CURVE.map((p) => ({ ...p })));
  const [curveG, setCurveG] = useState<CurvePoint[]>(DEFAULT_CURVE.map((p) => ({ ...p })));
  const [curveB, setCurveB] = useState<CurvePoint[]>(DEFAULT_CURVE.map((p) => ({ ...p })));

  const isLinked = curveMode === "linked";

  const curvesActive = useMemo(() => {
    if (isLinked) return !isIdentity(curveLinked);
    return !isIdentity(curveR) || !isIdentity(curveG) || !isIdentity(curveB);
  }, [isLinked, curveLinked, curveR, curveG, curveB]);

  const handleToggleScnr = useCallback((val: boolean) => {
    onScnrChange(val, state.scnrAmount);
  }, [state.scnrAmount, onScnrChange]);

  const handleAmountChange = useCallback((val: number) => {
    onScnrChange(state.scnrEnabled, val);
  }, [state.scnrEnabled, onScnrChange]);

  const handleResetCurves = useCallback(() => {
    const fresh = () => DEFAULT_CURVE.map((p) => ({ ...p }));
    setCurveLinked(fresh());
    setCurveR(fresh());
    setCurveG(fresh());
    setCurveB(fresh());
  }, []);

  const handleApply = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const dir = await getOutputDir();

      const cR = isLinked ? curveLinked : curveR;
      const cG = isLinked ? curveLinked : curveG;
      const cB = isLinked ? curveLinked : curveB;

      const scnr = state.scnrEnabled
        ? { method, amount: state.scnrAmount, preserveLuminance: preserveLum }
        : null;

      const res = await applyToneComposite({
        outputDir: dir,
        curvesR: cR,
        curvesG: cG,
        curvesB: cB,
        scnr,
      });

      setResult(res);

      if (res?.png_path) {
        const url = await getPreviewUrl(res.png_path);
        onResult(url);
      } else if (res?.previewUrl) {
        onResult(res.previewUrl);
      }
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [isLinked, curveLinked, curveR, curveG, curveB, state.scnrEnabled, state.scnrAmount, method, preserveLum, onResult]);

  return (
    <div className="flex flex-col gap-3 p-3">
      {!state.compositeReady && (
        <div className="text-[10px] text-amber-400/70 bg-amber-500/5 border border-amber-500/10 rounded-md px-2 py-1.5">
          Run Blend first to create a composite before applying color adjustments.
        </div>
      )}

      <div className="bg-zinc-950/50 rounded-lg border border-zinc-800/50 overflow-hidden">
        <div className="flex items-center justify-between px-3 py-1.5 border-b border-zinc-800/30">
          <span className="text-[10px] font-semibold text-zinc-400 uppercase tracking-wider">Curves</span>
          <div className="flex items-center gap-1">
            {(["linked", "r", "g", "b"] as CurveChannel[]).map((ch) => (
              <button
                key={ch}
                onClick={() => setCurveMode(ch)}
                className={`px-1.5 py-0.5 rounded text-[9px] font-medium transition-all ${
                  curveMode === ch
                    ? "text-white"
                    : "text-zinc-600 hover:text-zinc-400"
                }`}
                style={curveMode === ch ? { background: `${CHANNEL_COLORS[ch]}30`, color: CHANNEL_COLORS[ch] } : undefined}
              >
                {ch === "linked" ? "RGB" : ch.toUpperCase()}
              </button>
            ))}
          </div>
        </div>

        <div className="px-3 py-2 flex flex-col items-center gap-2">
          {isLinked ? (
            <CurveEditor
              points={curveLinked}
              onChange={setCurveLinked}
              color={CHANNEL_COLORS.linked}
              width={220}
              height={180}
            />
          ) : curveMode === "r" ? (
            <CurveEditor
              points={curveR}
              onChange={setCurveR}
              color={CHANNEL_COLORS.r}
              label="Red"
              width={220}
              height={180}
            />
          ) : curveMode === "g" ? (
            <CurveEditor
              points={curveG}
              onChange={setCurveG}
              color={CHANNEL_COLORS.g}
              label="Green"
              width={220}
              height={180}
            />
          ) : (
            <CurveEditor
              points={curveB}
              onChange={setCurveB}
              color={CHANNEL_COLORS.b}
              label="Blue"
              width={220}
              height={180}
            />
          )}

          <div className="flex items-center gap-2 w-full">
            <span className="text-[9px] text-zinc-600 flex-1">
              {curvesActive ? "Modified" : "Identity"} | Dbl-click add, right-click remove
            </span>
            <button
              onClick={handleResetCurves}
              className="text-[9px] text-zinc-500 hover:text-zinc-300 transition-colors"
            >
              Reset
            </button>
          </div>
        </div>
      </div>

      <Toggle label="SCNR (Green Removal)" checked={state.scnrEnabled} accent="purple" onChange={handleToggleScnr} />

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

      <RunButton
        label={curvesActive || state.scnrEnabled ? "Apply Curves" + (state.scnrEnabled ? " + SCNR" : "") : "Apply"}
        runningLabel="Applying..."
        running={loading}
        disabled={!state.compositeReady || (!curvesActive && !state.scnrEnabled)}
        accent="purple"
        onClick={handleApply}
      />

      {result && (
        <div className="text-[9px] text-zinc-500">
          {result.elapsed_ms}ms
          {result.curves_applied && " | curves applied"}
          {result.scnr_applied && " | SCNR applied"}
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
