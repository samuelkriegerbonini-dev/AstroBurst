import { useState, useCallback, useEffect, useRef } from "react";
import type { WizardState } from "../wizard";
import { resolveAnyChannelPath } from "../wizard";
import { Slider, RunButton, Toggle } from "../../ui";
import { restretchComposite } from "../../../services/compose";
import { maskedStretch, applyArcsinhStretch, maskedStretchComposite, arcsinhStretchComposite, applyGhsStretch, ghsStretchComposite, removeStars, removeStarsComposite } from "../../../services/processing";
import type { StarRemovalResult } from "../../../services/processing";
import { getPreviewUrl } from "../../../infrastructure/tauri";
import { getOutputDir } from "../../../infrastructure/tauri";
import { useCompositeContext } from "../../../context/CompositeContext";
import StfHistogram from "../StfHistogram";

const HIST_RGB = [
  { key: "__composite_r", color: "#ef4444" },
  { key: "__composite_g", color: "#22c55e" },
  { key: "__composite_b", color: "#3b82f6" },
];

interface StretchStepProps {
  state: WizardState;
  onStretchChange: (mode: WizardState["stretchMode"], factor?: number, target?: number) => void;
  onMaskParams: (growth: number, protection: number) => void;
  onMask: (path: string | null) => void;
  onResult: (png: string | null, stf?: { r: ChannelStf; g: ChannelStf; b: ChannelStf }) => void;
}

interface ChannelStf {
  shadow: number;
  midtone: number;
  highlight: number;
}

const DEFAULT_STF: ChannelStf = { shadow: 0, midtone: 0.5, highlight: 1 };

interface StretchRunResult {
  png_path?: string;
  previewUrl?: string;
  elapsed_ms?: number;
  iterations_run?: number;
  converged?: boolean;
  stretch_factor?: number;
}

export default function StretchStep({ state, onStretchChange, onMaskParams, onMask, onResult }: StretchStepProps) {
  const { compositeAutoStfR, compositeAutoStfG, compositeAutoStfB } = useCompositeContext();
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<StretchRunResult | null | undefined>(null);
  const [error, setError] = useState("");
  const [linked, setLinked] = useState(state.linkedStf);
  const [sharedMask, setSharedMask] = useState(true);
  const [detectionSigma, setDetectionSigma] = useState(8.0);
  const [maxEccentricity, setMaxEccentricity] = useState(0.85);
  const [srOpen, setSrOpen] = useState(false);
  const [srSigma, setSrSigma] = useState(4.0);
  const [srGrowth, setSrGrowth] = useState(3.0);
  const [srLoading, setSrLoading] = useState(false);
  const [srResult, setSrResult] = useState<StarRemovalResult | null>(null);
  const [srError, setSrError] = useState("");
  const [ghsD, setGhsD] = useState(2.0);
  const [ghsB, setGhsB] = useState(0.0);
  const [ghsSp, setGhsSp] = useState(0.01);
  const [ghsLp, setGhsLp] = useState(0.0);
  const [ghsHp, setGhsHp] = useState(1.0);
  const [stfR, setStfR] = useState<ChannelStf>({ ...DEFAULT_STF });
  const [stfG, setStfG] = useState<ChannelStf>({ ...DEFAULT_STF });
  const [stfB, setStfB] = useState<ChannelStf>({ ...DEFAULT_STF });
  const prevAutoStf = useRef<ChannelStf | null>(null);

  useEffect(() => {
    if (!compositeAutoStfR) return;
    if (prevAutoStf.current === compositeAutoStfR) return;
    prevAutoStf.current = compositeAutoStfR;
    const r = compositeAutoStfR as ChannelStf;
    const g = (compositeAutoStfG ?? compositeAutoStfR) as ChannelStf;
    const b = (compositeAutoStfB ?? compositeAutoStfR) as ChannelStf;
    setStfR({ ...r });
    setStfG({ ...g });
    setStfB({ ...b });
  }, [compositeAutoStfR, compositeAutoStfG, compositeAutoStfB]);

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
      let res: StretchRunResult | undefined;
      const dir = await getOutputDir();
      const stfBundle = { r: stfR, g: stfG, b: stfB };

      if (state.stretchMode === "masked") {
        if (state.compositeReady) {
          res = await maskedStretchComposite(dir, {
            iterations: 10,
            targetBackground: state.targetBackground,
            maskGrowth: state.maskGrowth,
            protectionAmount: state.maskProtection,
            sharedMask,
            detectionSigma,
            maxEccentricity,
          });
        } else {
          const path = resolveAnyChannelPath(state);
          if (!path) throw new Error("No channel path found");
          res = await maskedStretch(path, dir, {
            iterations: 10,
            targetBackground: state.targetBackground,
            maskGrowth: state.maskGrowth,
            protectionAmount: state.maskProtection,
            detectionSigma,
            maxEccentricity,
          });
        }
        if (res?.png_path) {
          const url = await getPreviewUrl(res.png_path);
          onResult(url, stfBundle);
        } else if (res?.previewUrl) {
          onResult(res.previewUrl, stfBundle);
        }
      } else if (state.stretchMode === "arcsinh") {
        if (state.compositeReady) {
          res = await arcsinhStretchComposite(state.stretchFactor, dir);
        } else {
          const path = resolveAnyChannelPath(state);
          if (!path) throw new Error("No channel path found");
          res = await applyArcsinhStretch(path, dir, state.stretchFactor);
        }
        if (res?.png_path) {
          const url = await getPreviewUrl(res.png_path);
          onResult(url, stfBundle);
        } else if (res?.previewUrl) {
          onResult(res.previewUrl, stfBundle);
        }
      } else if (state.stretchMode === "ghs") {
        const ghsOptions = {
          stretchFactor: ghsD,
          localIntensity: ghsB,
          symmetryPoint: ghsSp,
          shadowProtect: ghsLp,
          highlightProtect: ghsHp,
        };
        if (state.compositeReady) {
          res = await ghsStretchComposite(dir, ghsOptions);
        } else {
          const path = resolveAnyChannelPath(state);
          if (!path) throw new Error("No channel path found");
          res = await applyGhsStretch(path, dir, ghsOptions);
        }
        if (res?.png_path) {
          const url = await getPreviewUrl(res.png_path);
          onResult(url, stfBundle);
        } else if (res?.previewUrl) {
          onResult(res.previewUrl, stfBundle);
        }
      } else {
        if (state.compositeReady) {
          res = await restretchComposite(dir, stfR, stfG, stfB);
        }
        if (res?.png_path) {
          const url = await getPreviewUrl(res.png_path);
          onResult(url, stfBundle);
        }
      }

      setResult(res);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [state, stfR, stfG, stfB, ghsD, ghsB, ghsSp, ghsLp, ghsHp, sharedMask, detectionSigma, maxEccentricity, onResult]);

  const handleRemoveStars = useCallback(async () => {
    setSrLoading(true);
    setSrError("");
    try {
      const dir = await getOutputDir();
      const opts = { detectionSigma: srSigma, growthFactor: srGrowth };
      let res: StarRemovalResult;
      if (state.compositeReady) {
        res = await removeStarsComposite(dir, opts);
      } else {
        const path = resolveAnyChannelPath(state);
        if (!path) throw new Error("No channel path found");
        res = await removeStars(path, dir, opts);
      }
      setSrResult(res);
      if (res.previewUrl) {
        onResult(res.previewUrl, { r: stfR, g: stfG, b: stfB });
      }
    } catch (e) {
      setSrError(e instanceof Error ? e.message : String(e));
    } finally {
      setSrLoading(false);
    }
  }, [state, srSigma, srGrowth, stfR, stfG, stfB, onResult]);

  const handleResetStf = useCallback(() => {
    const autoR = (compositeAutoStfR ?? DEFAULT_STF) as ChannelStf;
    const autoG = (compositeAutoStfG ?? compositeAutoStfR ?? DEFAULT_STF) as ChannelStf;
    const autoB = (compositeAutoStfB ?? compositeAutoStfR ?? DEFAULT_STF) as ChannelStf;
    setStfR({ ...autoR });
    setStfG({ ...autoG });
    setStfB({ ...autoB });
    setResult(null);
  }, [compositeAutoStfR, compositeAutoStfG, compositeAutoStfB]);

  const isSaturated = state.compositeReady && (state.wbR > 1.3 || state.wbG > 1.3 || state.wbB > 1.3);

  const runLabel =
    state.stretchMode === "masked"
      ? "Apply Masked Stretch"
      : state.stretchMode === "arcsinh"
        ? "Apply Arcsinh Stretch"
        : state.stretchMode === "ghs"
          ? "Apply GHS Stretch"
          : state.compositeReady
            ? "Re-stretch Composite"
            : "Apply Auto STF";

  return (
    <div className="flex flex-col gap-3 p-3">
      {state.compositeReady && (
        <div className="text-[10px] text-emerald-400/70 bg-emerald-500/5 border border-emerald-500/10 rounded-md px-2 py-1.5">
          Operating on blended composite (R/G/B cached). {state.stretchMode === "auto_stf" ? "Adjust STF params to re-stretch." : "Stretch applies per-channel."}
        </div>
      )}

      {isSaturated && (
        <div className="text-[10px] text-amber-400/90 bg-amber-500/10 border border-amber-500/20 rounded-md px-2 py-1.5">
          WB factors &gt; 1.3 detected (R={state.wbR.toFixed(2)} G={state.wbG.toFixed(2)} B={state.wbB.toFixed(2)}). Consider reducing factors in Color Balance.
        </div>
      )}

      <div className="flex flex-col gap-2 p-2 rounded-lg border border-violet-500/15 bg-violet-500/5">
        <button
          onClick={() => setSrOpen((v) => !v)}
          className="flex items-center justify-between text-[10px] font-medium text-violet-300 hover:text-violet-200 transition-colors"
        >
          <span>Star Removal (experimental)</span>
          <span className="text-zinc-600">{srOpen ? "−" : "+"}</span>
        </button>
        {srOpen && (
          <>
            <Slider label="Detection Sigma" value={srSigma} min={2} max={10} step={0.5} accent="violet"
                    format={(v) => `${v.toFixed(1)}σ`} onChange={setSrSigma}
                    hint="lower = more stars removed" />
            <Slider label="Mask Growth" value={srGrowth} min={1} max={8} step={0.25} accent="violet"
                    format={(v) => `${v.toFixed(2)}x FWHM`} onChange={setSrGrowth}
                    hint="covers halos, may eat nebula if too high" />
            <RunButton
              label={state.compositeReady ? "Remove Stars from Composite" : "Remove Stars"}
              runningLabel="Removing stars..."
              running={srLoading}
              accent="violet"
              onClick={handleRemoveStars}
            />
            {srResult && (
              <div className="text-[9px] text-zinc-500">
                {srResult.stars_masked} stars removed, {(srResult.mask_coverage * 100).toFixed(1)}% masked, {srResult.elapsed_ms}ms.
                Stars layer saved separately{srResult.stars_fits_path ? " (FITS)" : ""}.
              </div>
            )}
            <div className="text-[9px] text-zinc-600">
              Classic detection + inpaint on linear data; the composite cache becomes starless, so stretches below apply to it. Re-run Blend to restore stars. Big saturated stars and diffraction spikes may leave residue.
            </div>
            {srError && <div className="text-[9px] text-red-400">{srError}</div>}
          </>
        )}
      </div>

      <div className="flex items-center justify-between">
        <label className="text-xs text-zinc-400">Stretch Mode</label>
        <select value={state.stretchMode} onChange={(e) => handleModeChange(e.target.value as WizardState["stretchMode"])} className="ab-select">
          <option value="masked">Masked Stretch (star-protected)</option>
          <option value="arcsinh">Arcsinh Stretch</option>
          <option value="ghs">GHS (Generalized Hyperbolic)</option>
          <option value="auto_stf">Auto STF</option>
        </select>
      </div>

      {state.stretchMode === "masked" && (
        <div className="flex flex-col gap-2">
          <Slider label="Target Background" value={state.targetBackground} min={0.05} max={0.5} step={0.01} accent="amber"
                  format={(v) => v.toFixed(2)} onChange={handleTargetChange} />
          <Slider label="Mask Growth" value={state.maskGrowth} min={0.5} max={10.0} step={0.1} accent="rose"
                  format={(v) => `${v.toFixed(1)}x FWHM`} onChange={(v) => onMaskParams(v, state.maskProtection)} />
          <Slider label="Star Protection" value={state.maskProtection} min={0.0} max={1.0} step={0.01} accent="rose"
                  format={(v) => `${(v * 100).toFixed(0)}%`} onChange={(v) => onMaskParams(state.maskGrowth, v)} />
          <Slider label="Detection Sigma" value={detectionSigma} min={3} max={15} step={0.5} accent="rose"
                  format={(v) => `${v.toFixed(1)}σ`} onChange={setDetectionSigma}
                  hint="higher = only strong stars masked" />
          <Slider label="Max Eccentricity" value={maxEccentricity} min={0.5} max={1.0} step={0.01} accent="rose"
                  format={(v) => v.toFixed(2)} onChange={setMaxEccentricity}
                  hint="lower = reject elongated blobs" />
          {state.compositeReady && (
            <Toggle label="Shared star mask" checked={sharedMask} accent="rose" onChange={setSharedMask} />
          )}
          {state.starMaskPath && (
            <div className="flex items-center gap-2 p-2 rounded-lg bg-rose-600/10 border border-rose-500/20">
              <span className="w-2 h-2 rounded-full bg-rose-400" />
              <span className="text-[10px] text-rose-300 flex-1 truncate">{state.starMaskPath.split(/[/\\]/).pop()}</span>
              <button onClick={() => onMask(null)} className="text-[9px] text-zinc-500 hover:text-red-400">Clear</button>
            </div>
          )}
          <div className="text-[9px] text-zinc-600">
            Star mask protects bright stars during the stretch. You can also import a .segm FITS mask from external software.
          </div>
        </div>
      )}

      {state.stretchMode === "arcsinh" && (
        <Slider label="Stretch Factor" value={state.stretchFactor} min={1} max={500} step={1} accent="amber"
                format={(v) => `${v}`} onChange={handleFactorChange} />
      )}

      {state.stretchMode === "ghs" && (
        <div className="flex flex-col gap-2">
          <Slider label="Stretch (D)" value={ghsD} min={0} max={50} step={0.05} accent="amber"
                  format={(v) => v.toFixed(2)} onChange={setGhsD} />
          <Slider label="Local Intensity (b)" value={ghsB} min={-5} max={15} step={0.1} accent="amber"
                  format={(v) => v.toFixed(1)} onChange={setGhsB} />
          <Slider label="Symmetry Point (SP)" value={ghsSp} min={0} max={1} step={0.001} accent="amber"
                  format={(v) => v.toFixed(3)}
                  onChange={(v) => {
                    setGhsSp(v);
                    if (ghsLp > v) setGhsLp(v);
                    if (ghsHp < v) setGhsHp(v);
                  }} />
          <Slider label="Shadow Protect (LP)" value={ghsLp} min={0} max={1} step={0.001} accent="amber"
                  format={(v) => v.toFixed(3)} onChange={(v) => setGhsLp(Math.min(v, ghsSp))} />
          <Slider label="Highlight Protect (HP)" value={ghsHp} min={0} max={1} step={0.001} accent="amber"
                  format={(v) => v.toFixed(3)} onChange={(v) => setGhsHp(Math.max(v, ghsSp))} />
          <div className="text-[9px] text-zinc-600">
            On linear data set SP near the background (~0.005&ndash;0.02) then raise D; an SP above the faint signal leaves it black.
            b&lt;0 super-stretches faint signal (b=-1 log), b=0 exponential, b&gt;0 focuses contrast near SP.
            LP/HP keep shadows/highlights linear.
          </div>
        </div>
      )}

      {state.stretchMode === "auto_stf" && state.compositeReady && (
        <>
          <Toggle label="Link channels" checked={linked} accent="amber" onChange={handleLinkedChange} />

          {linked ? (
            <div className="flex flex-col gap-2">
              <StfHistogram channels={HIST_RGB} shadow={stfR.shadow} midtone={stfR.midtone} highlight={stfR.highlight} />
              <Slider label="Shadow" value={stfR.shadow} min={0} max={0.5} step={0.0001} accent="amber"
                      format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "shadow", v)} />
              <Slider label="Midtone" value={stfR.midtone} min={0.0001} max={1} step={0.0001} scale="log" accent="amber"
                      format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "midtone", v)} />
              <Slider label="Highlight" value={stfR.highlight} min={0.5} max={1} step={0.001} accent="amber"
                      format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "highlight", v)} />
            </div>
          ) : (
            <div className="flex flex-col gap-3">
              <div className="flex flex-col gap-1.5">
                <span className="text-[10px] font-medium text-red-400">R Channel</span>
                <StfHistogram channels={[HIST_RGB[0]]} shadow={stfR.shadow} midtone={stfR.midtone} highlight={stfR.highlight} height={32} />
                <Slider label="Shadow" value={stfR.shadow} min={0} max={0.5} step={0.0001} accent="red"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "shadow", v)} />
                <Slider label="Midtone" value={stfR.midtone} min={0.0001} max={1} step={0.0001} scale="log" accent="red"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "midtone", v)} />
                <Slider label="Highlight" value={stfR.highlight} min={0.5} max={1} step={0.001} accent="red"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("r", "highlight", v)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <span className="text-[10px] font-medium text-green-400">G Channel</span>
                <StfHistogram channels={[HIST_RGB[1]]} shadow={stfG.shadow} midtone={stfG.midtone} highlight={stfG.highlight} height={32} />
                <Slider label="Shadow" value={stfG.shadow} min={0} max={0.5} step={0.0001} accent="green"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("g", "shadow", v)} />
                <Slider label="Midtone" value={stfG.midtone} min={0.0001} max={1} step={0.0001} scale="log" accent="green"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("g", "midtone", v)} />
                <Slider label="Highlight" value={stfG.highlight} min={0.5} max={1} step={0.001} accent="green"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("g", "highlight", v)} />
              </div>
              <div className="flex flex-col gap-1.5">
                <span className="text-[10px] font-medium text-blue-400">B Channel</span>
                <StfHistogram channels={[HIST_RGB[2]]} shadow={stfB.shadow} midtone={stfB.midtone} highlight={stfB.highlight} height={32} />
                <Slider label="Shadow" value={stfB.shadow} min={0} max={0.5} step={0.0001} accent="blue"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("b", "shadow", v)} />
                <Slider label="Midtone" value={stfB.midtone} min={0.0001} max={1} step={0.0001} scale="log" accent="blue"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("b", "midtone", v)} />
                <Slider label="Highlight" value={stfB.highlight} min={0.5} max={1} step={0.001} accent="blue"
                        format={(v) => v.toFixed(4)} onChange={(v) => updateChannel("b", "highlight", v)} />
              </div>
            </div>
          )}
        </>
      )}

      {state.stretchMode === "auto_stf" && !state.compositeReady && (
        <div className="text-[10px] text-zinc-500">
          Auto STF will compute optimal shadow/midtone/highlight per channel based on image statistics.
        </div>
      )}

      <div className="flex items-center gap-2">
        <div className="flex-1">
          <RunButton
            label={runLabel}
            runningLabel="Stretching..."
            running={loading}
            accent="amber"
            onClick={handleRun}
          />
        </div>
        {state.stretchMode === "auto_stf" && state.compositeReady && (
          <button
            onClick={handleResetStf}
            disabled={loading}
            className="px-2.5 py-1.5 rounded-md text-[10px] font-medium bg-zinc-800/60 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-700/60 transition-all disabled:opacity-40"
          >
            Reset to Auto STF
          </button>
        )}
      </div>

      {result && (
        <div className="text-[9px] text-zinc-500">
          {result.elapsed_ms}ms
          {result.iterations_run && `, ${result.iterations_run} iterations`}
          {result.converged !== undefined && `, ${result.converged ? "converged" : "not converged"}`}
          {result.stretch_factor && `, factor=${result.stretch_factor}`}
        </div>
      )}
      {error && <div className="text-[9px] text-red-400">{error}</div>}
    </div>
  );
}
