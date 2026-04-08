import { useState, useCallback, useMemo, useEffect, useRef, lazy, Suspense } from "react";
import { Loader2, AlertTriangle } from "lucide-react";
import type { WizardState } from "../wizard";
import { resolveChannelPath, isNarrowbandWorkflow, type FilterDetectionRef } from "../../../utils/wizard";
import { Slider, RunButton, Toggle } from "../../ui";
import { calibrateAndScnr, computeAutoWb, resetWb } from "../../../services/compose";
import { getPreviewUrl } from "../../../infrastructure/tauri/client";
import { getOutputDir } from "../../../infrastructure/tauri";

const SpccPanel = lazy(() => import("../SpccPanel"));

interface ColorBalanceStepProps {
  state: WizardState;
  filterDetections?: FilterDetectionRef[];
  onWbChange: (mode: WizardState["wbMode"], r?: number, g?: number, b?: number) => void;
  onScnrChange: (enabled: boolean, amount?: number, method?: string, preserveLuminance?: boolean) => void;
  onResult: (png: string | null, autoStf?: { shadow: number; midtone: number; highlight: number }) => void;
}

export default function ColorBalanceStep({ state, filterDetections, onWbChange, onScnrChange, onResult }: ColorBalanceStepProps) {
  const narrowband = useMemo(() => isNarrowbandWorkflow(state.bins, state.blendPreset, filterDetections), [state.bins, state.blendPreset, filterDetections]);

  const [localR, setLocalR] = useState(state.wbR);
  const [localG, setLocalG] = useState(state.wbG);
  const [localB, setLocalB] = useState(state.wbB);
  const [loading, setLoading] = useState(false);
  const [autoLoading, setAutoLoading] = useState(false);
  const [refChannel, setRefChannel] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [elapsed, setElapsed] = useState<number | null>(null);

  const [scnrMethod, setScnrMethod] = useState<"average" | "maximum">(state.scnrMethod);
  const [preserveLum, setPreserveLum] = useState(state.scnrPreserveLuminance);

  const defaultsSet = useRef(false);

  const rgbBins = useMemo(() => {
    const r = state.bins.find((b) => b.id === "r" || b.id === "ha");
    const g = state.bins.find((b) => b.id === "g" || b.id === "oiii");
    const bBin = state.bins.find((b) => b.id === "b" || b.id === "sii");
    return { r, g, b: bBin };
  }, [state.bins]);

  const rPath = resolveChannelPath(state, rgbBins.r?.id ?? "");
  const gPath = resolveChannelPath(state, rgbBins.g?.id ?? "");
  const bPath = resolveChannelPath(state, rgbBins.b?.id ?? "");

  useEffect(() => {
    if (defaultsSet.current || !state.compositeReady) return;
    defaultsSet.current = true;
    if (!narrowband && !state.scnrEnabled) {
      onScnrChange(true, 0.8, "average", false);
    }
  }, [state.compositeReady, narrowband, state.scnrEnabled, onScnrChange]);

  useEffect(() => {
    if (state.wbMode !== "auto" || !state.compositeReady) return;
    let cancelled = false;
    setAutoLoading(true);
    computeAutoWb()
      .then((res) => {
        if (cancelled) return;
        const r = Math.round(res.r_factor * 100) / 100;
        const g = Math.round(res.g_factor * 100) / 100;
        const b = Math.round(res.b_factor * 100) / 100;
        setLocalR(r);
        setLocalG(g);
        setLocalB(b);
        setRefChannel(res.ref_channel ?? null);
        onWbChange("auto", r, g, b);
      })
      .catch(() => {})
      .finally(() => { if (!cancelled) setAutoLoading(false); });
    return () => { cancelled = true; };
  }, [state.wbMode, state.compositeReady]);

  const sliderMax = useMemo(
    () => Math.max(3.0, Math.ceil(Math.max(localR, localG, localB) * 1.5 * 10) / 10),
    [localR, localG, localB],
  );
  const sliderMin = useMemo(
    () => Math.min(0.1, Math.floor(Math.min(localR, localG, localB) * 0.5 * 10) / 10),
    [localR, localG, localB],
  );

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

  const handleToggleScnr = useCallback((val: boolean) => {
    onScnrChange(val, state.scnrAmount, scnrMethod, preserveLum);
  }, [state.scnrAmount, scnrMethod, preserveLum, onScnrChange]);

  const handleAmountChange = useCallback((val: number) => {
    onScnrChange(state.scnrEnabled, val, scnrMethod, preserveLum);
  }, [state.scnrEnabled, scnrMethod, preserveLum, onScnrChange]);

  const handleMethodChange = useCallback((val: string) => {
    const m = val as "average" | "maximum";
    setScnrMethod(m);
    onScnrChange(state.scnrEnabled, state.scnrAmount, m, preserveLum);
  }, [state.scnrEnabled, state.scnrAmount, preserveLum, onScnrChange]);

  const handlePreserveLumChange = useCallback((val: boolean) => {
    setPreserveLum(val);
    onScnrChange(state.scnrEnabled, state.scnrAmount, scnrMethod, val);
  }, [state.scnrEnabled, state.scnrAmount, scnrMethod, onScnrChange]);

  const handleApply = useCallback(async () => {
    setLoading(true);
    setError("");
    setElapsed(null);
    try {
      const dir = await getOutputDir();
      const scnr = state.scnrEnabled
        ? { enabled: true, method: scnrMethod, amount: state.scnrAmount, preserveLuminance: preserveLum }
        : undefined;
      const res = await calibrateAndScnr(dir, localR, localG, localB, scnr);
      if (res?.png_path) {
        const url = await getPreviewUrl(res.png_path);
        onResult(url, res.auto_stf ?? undefined);
      }
      if (res?.elapsed_ms) setElapsed(res.elapsed_ms);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [localR, localG, localB, state.scnrEnabled, state.scnrAmount, scnrMethod, preserveLum, onResult]);

  const handleReset = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const res = await resetWb(await getOutputDir());
      setLocalR(1.0);
      setLocalG(1.0);
      setLocalB(1.0);
      onWbChange("manual", 1.0, 1.0, 1.0);
      if (res?.png_path) {
        const url = await getPreviewUrl(res.png_path);
        onResult(url, res.auto_stf ?? undefined);
      }
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setLoading(false);
    }
  }, [onResult, onWbChange]);

  const isFactorsNeutral = localR === 1.0 && localG === 1.0 && localB === 1.0;

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center gap-2">
        <span className={`px-1.5 py-0.5 rounded text-[9px] font-bold uppercase tracking-wider ${
          narrowband
            ? "bg-amber-500/15 text-amber-400 ring-1 ring-amber-500/30"
            : "bg-emerald-500/15 text-emerald-400 ring-1 ring-emerald-500/30"
        }`}>
          {narrowband ? "Narrowband" : "Broadband"}
        </span>
      </div>

      {!state.compositeReady && (
        <div className="text-[10px] text-amber-400/70 bg-amber-500/5 border border-amber-500/10 rounded-md px-2 py-1.5">
          Run Blend first to create a composite before adjusting color balance.
        </div>
      )}

      {state.compositeReady && (
        <div className="text-[10px] text-cyan-400/70 bg-cyan-500/5 border border-cyan-500/10 rounded-md px-2 py-1.5">
          Operating on blended composite (R/G/B cached). Adjust factors and apply.
        </div>
      )}

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">White Balance</label>
        <select value={state.wbMode} onChange={(e) => handleModeChange(e.target.value as WizardState["wbMode"])} className="ab-select">
          <option value="auto">Auto (Stability)</option>
          <option value="spcc">SPCC (Spectrophotometric)</option>
          <option value="manual">Manual</option>
          <option value="none">None</option>
        </select>
      </div>

      {(state.wbMode === "manual" || state.wbMode === "auto") && (
        <div className="flex flex-col gap-2 pl-2">
          {state.wbMode === "auto" && autoLoading && (
            <div className="flex items-center gap-2 text-[10px] text-cyan-400/60">
              <Loader2 size={10} className="animate-spin" />
              Computing stability-based WB...
            </div>
          )}
          {state.wbMode === "auto" && !autoLoading && refChannel && (
            <div className="text-[9px] text-zinc-500">
              Reference channel: {refChannel} (lowest MAD/median)
            </div>
          )}
          <Slider label="R" value={localR} min={sliderMin} max={sliderMax} step={0.01} accent="red"
                  format={(v) => v.toFixed(2)} onChange={(v) => handleManualChange("r", v)} />
          <Slider label="G" value={localG} min={sliderMin} max={sliderMax} step={0.01} accent="green"
                  format={(v) => v.toFixed(2)} onChange={(v) => handleManualChange("g", v)} />
          <Slider label="B" value={localB} min={sliderMin} max={sliderMax} step={0.01} accent="blue"
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

      <div className="border-t border-zinc-800/30 pt-3 mt-1">
        <Toggle
          label="SCNR (Green Excess Reduction)"
          checked={state.scnrEnabled}
          accent="cyan"
          onChange={handleToggleScnr}
        />
      </div>

      {narrowband && state.scnrEnabled && (
        <div className="flex items-start gap-2 text-[10px] text-amber-400/90 bg-amber-500/10 border border-amber-500/20 rounded-md px-2 py-1.5">
          <AlertTriangle size={12} className="shrink-0 mt-0.5" />
          <span>
            SCNR removes green channel excess. In narrowband data, green carries real
            emission signal (Ha/SII/OIII depending on mapping). Enable only for blend
            matrix artifacts, not data-inherent green.
          </span>
        </div>
      )}

      {state.scnrEnabled && (
        <div className="flex flex-col gap-2 pl-2">
          <div className="flex items-center justify-between">
            <label className="text-xs text-zinc-400">Method</label>
            <select value={scnrMethod} onChange={(e) => handleMethodChange(e.target.value)} className="ab-select">
              <option value="average">Average Neutral</option>
              <option value="maximum">Maximum Neutral</option>
            </select>
          </div>
          <Slider
            label="Amount"
            value={state.scnrAmount}
            min={0}
            max={1}
            step={0.05}
            accent="cyan"
            format={(v) => `${(v * 100).toFixed(0)}%`}
            onChange={handleAmountChange}
          />
          <Toggle
            label="Preserve Brightness (redistributes to R/B)"
            checked={preserveLum}
            accent="cyan"
            onChange={handlePreserveLumChange}
          />
          {narrowband && preserveLum && (
            <div className="flex items-start gap-2 text-[10px] text-amber-400/70 bg-amber-500/5 border border-amber-500/10 rounded-md px-2 py-1.5">
              <AlertTriangle size={10} className="shrink-0 mt-0.5" />
              <span>
                Preserve brightness redistributes removed green intensity to R and B.
                This may create color shifts that don't reflect actual emission spectra.
              </span>
            </div>
          )}
        </div>
      )}

      {state.compositeReady && state.wbMode !== "none" && (
        <div className="flex items-center gap-2">
          <div className="flex-1">
            <RunButton
              label={
                isFactorsNeutral && !state.scnrEnabled
                  ? "Apply Color Balance (neutral)"
                  : `Apply Color Balance (R=${localR.toFixed(2)} G=${localG.toFixed(2)} B=${localB.toFixed(2)}${state.scnrEnabled ? " + SCNR" : ""})`
              }
              runningLabel="Applying..."
              running={loading}
              accent="cyan"
              onClick={handleApply}
            />
          </div>
          {!isFactorsNeutral && (
            <button
              onClick={handleReset}
              disabled={loading}
              className="px-2.5 py-1.5 rounded-md text-[10px] font-medium bg-zinc-800/60 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-700/60 transition-all disabled:opacity-40"
            >
              Reset
            </button>
          )}
        </div>
      )}

      {elapsed !== null && (
        <div className="text-[9px] text-zinc-500">
          {elapsed}ms{state.scnrEnabled ? " | SCNR applied" : ""}
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}

      <div className="text-[9px] text-zinc-600 pt-1 border-t border-zinc-800/30">
        Current factors: R={state.wbR.toFixed(3)} G={state.wbG.toFixed(3)} B={state.wbB.toFixed(3)}
        {state.scnrEnabled && ` | SCNR ${(state.scnrAmount * 100).toFixed(0)}% ${scnrMethod}`}
      </div>
    </div>
  );
}
