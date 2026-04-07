import { useState, useCallback, useMemo } from "react";
import type { WizardState } from "../wizard";
import { applyToneComposite, type CurvePoint } from "../../../services/tone";
import { getOutputDir } from "../../../infrastructure/tauri";
import { getPreviewUrl } from "../../../infrastructure/tauri/client";
import { RunButton } from "../../ui";
import CurveEditor from "../CurveEditor";
import { useCompositeContext } from "../../../context/CompositeContext";

interface AdjustStepProps {
  state: WizardState;
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

export default function AdjustStep({ state, onResult }: AdjustStepProps) {
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeContext();
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

      const res = await applyToneComposite({
        outputDir: dir,
        stfR: compositeStfR,
        stfG: compositeStfG,
        stfB: compositeStfB,
        linkedStf: compositeStfLinked,
        curvesR: cR,
        curvesG: cG,
        curvesB: cB,
        scnr: null,
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
  }, [isLinked, curveLinked, curveR, curveG, curveB, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, onResult]);

  return (
    <div className="flex flex-col gap-3 p-3">
      {!state.compositeReady && (
        <div className="text-[10px] text-amber-400/70 bg-amber-500/5 border border-amber-500/10 rounded-md px-2 py-1.5">
          Run Blend first to create a composite before applying adjustments.
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

      <RunButton
        label={curvesActive ? "Apply Curves" : "Apply"}
        runningLabel="Applying..."
        running={loading}
        disabled={!state.compositeReady || !curvesActive}
        accent="purple"
        onClick={handleApply}
      />

      {result && (
        <div className="text-[9px] text-zinc-500">
          {result.elapsed_ms}ms
          {result.curves_applied && " | curves applied"}
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
