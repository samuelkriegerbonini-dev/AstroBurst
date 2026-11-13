import { useState, useRef, useEffect, useCallback, useId } from "react";
import { buildEmpiricalPsf, estimatePsf } from "../../services/processing";
import { compositeChainState, compositeEstimatePsf } from "../../services/compositeChain";
import { getOutputDir } from "../../infrastructure/tauri";
import { useProcessingRun } from "../../hooks/useProcessingRun";
import { useCompositeChain } from "../../hooks/useCompositeChain";
import { useWcsPixelScale } from "../../hooks/useResolutionSettings";
import { SIGMA_MAD_TITLE } from "../../utils/analysisLabels";
import { robustWidth } from "../../utils/resolutionText";
import { COMPOSITE_RUN_KEY } from "../../utils/compositeChain";
import { paintKernel } from "../../utils/psfProfile";
import { Slider, Toggle, RunButton, ErrorAlert, SectionHeader } from "../ui";
import EmpiricalPsfResult from "./EmpiricalPsfResult";
import { compositeModeNotice } from "./compositeProps";
import type { CompositePanelProps } from "./compositeProps";
import type { EmpiricalPsfReport, PsfRadiusUnit, PsfStackMethod, PsfStarSource } from "../../shared/types/processing";

interface PsfResult {
  kernel: number[][];
  kernel_size: number;
  average_fwhm: number;
  average_ellipticity: number;
  stars_used: { x: number; y: number; fwhm: number; snr: number; ellipticity: number; peak: number; flux: number }[];
  stars_rejected: number;
  spread_pixels: number;
}

interface PsfPanelProps extends Omit<CompositePanelProps, "onCompositeDone"> {
  selectedFile: { path: string; result?: unknown } | null;
  onPsfReady?: (kernel: number[][]) => void;
  onCompositePsfReady?: (kernel: number[][], liveGeneration: number) => void;
  fileKey?: string | null;
  disabledReason?: string | null;
  disabledReasonId?: string;
}

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-violet-400">
    <circle cx="12" cy="12" r="3" />
    <circle cx="12" cy="12" r="8" opacity="0.3" />
    <circle cx="12" cy="12" r="11" opacity="0.1" />
  </svg>
);

export default function PsfPanel({ selectedFile, onPsfReady, onCompositePsfReady, fileKey, compositeMode, compositeInput, fileName, disabledReason, disabledReasonId }: PsfPanelProps) {
  const { running: loading, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<PsfResult>("psf", compositeMode ? COMPOSITE_RUN_KEY : fileKey ?? null);
  const compositeChain = useCompositeChain();
  const result = runResult && (!compositeMode || compositeChain.psfKernel === runResult.kernel) ? runResult : null;
  const canRun = compositeMode ? compositeInput !== null : !!selectedFile;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const scale = useWcsPixelScale(compositeMode ? null : selectedFile?.path ?? null);
  const starScatter = result ? robustWidth(result.stars_used.map((s) => s.fwhm))?.scatter ?? null : null;

  const [numStars, setNumStars] = useState(30);
  const [cutoutRadius, setCutoutRadius] = useState(15);
  const [maxEllipticity, setMaxEllipticity] = useState(0.3);
  const [satThreshold, setSatThreshold] = useState(0.95);

  const { running: recipeRunning, blocked: recipeBlocked, busyTitle: recipeBusyTitle, result: recipeResult, error: recipeError, run: runRecipe } = useProcessingRun<EmpiricalPsfReport>("psfRecipe", compositeMode ? null : fileKey ?? null);
  const sourceId = useId();
  const unitId = useId();
  const stackId = useId();
  const [starSource, setStarSource] = useState<PsfStarSource["kind"]>("auto");
  const [regionPath, setRegionPath] = useState("");
  const [radecText, setRadecText] = useState("");
  const [cutoutSize, setCutoutSize] = useState(61);
  const [psfSize, setPsfSize] = useState(51);
  const [normalizationRadius, setNormalizationRadius] = useState(20);
  const [radiusUnit, setRadiusUnit] = useState<PsfRadiusUnit>("pixel");
  const [oversampling, setOversampling] = useState(1);
  const [stack, setStack] = useState<PsfStackMethod>("median");
  const [radialCleaning, setRadialCleaning] = useState(false);
  const [saveFits, setSaveFits] = useState(true);

  const handleFileEstimate = useCallback(() => {
    if (!selectedFile?.path) return;
    const path = selectedFile.path;
    void run(async () => {
      const res = await estimatePsf(path, {
        numStars,
        cutoutRadius,
        maxEllipticity,
        saturationThreshold: satThreshold,
      }) as PsfResult;
      if (res && onPsfReady) {
        onPsfReady(res.kernel);
      }
      return res ?? null;
    });
  }, [selectedFile, numStars, cutoutRadius, maxEllipticity, satThreshold, onPsfReady, run]);

  const handleCompositeEstimate = useCallback(() => {
    if (!compositeInput) return;
    const chainInput = compositeInput.input;
    void run(async () => {
      const { live_generation } = await compositeChainState();
      const res = await compositeEstimatePsf(chainInput, {
        numStars,
        cutoutRadius,
        maxEllipticity,
        saturationThreshold: satThreshold,
      });
      if (!res.kernel) throw new Error("PSF estimation returned no kernel");
      onCompositePsfReady?.(res.kernel, live_generation);
      return { ...res, kernel: res.kernel };
    });
  }, [compositeInput, numStars, cutoutRadius, maxEllipticity, satThreshold, onCompositePsfReady, run]);

  const handleEstimate = compositeMode ? handleCompositeEstimate : handleFileEstimate;

  useEffect(() => {
    if (!result || !canvasRef.current) return;
    paintKernel(canvasRef.current, result.kernel);
  }, [result]);

  const handleRecipe = useCallback(() => {
    if (!selectedFile?.path) return;
    const path = selectedFile.path;
    const source: PsfStarSource = starSource === "regions"
      ? { kind: "regions", path: regionPath.trim() }
      : starSource === "ra_dec"
        ? { kind: "ra_dec", text: radecText }
        : { kind: "auto", num_stars: numStars };
    void runRecipe(async () => {
      const outputDir = saveFits ? await getOutputDir() : null;
      const report = await buildEmpiricalPsf(path, source, {
        recipe: {
          cutout_size: cutoutSize,
          psf_size: psfSize,
          normalization_radius: normalizationRadius,
          normalization_radius_unit: radiusUnit,
          oversampling: oversampling > 1 ? oversampling : null,
          stack,
          radial_cleaning: radialCleaning,
        },
        outputDir,
      });
      onPsfReady?.(report.kernel);
      return report;
    });
  }, [selectedFile, starSource, regionPath, radecText, numStars, saveFits, cutoutSize, psfSize, normalizationRadius, radiusUnit, oversampling, stack, radialCleaning, onPsfReady, runRecipe]);

  const handlePickRegions = useCallback(async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const picked = await open({ multiple: false, filters: [{ name: "DS9 regions", extensions: ["reg"] }], title: "Stars for the empirical PSF" });
    const chosen = Array.isArray(picked) ? picked[0] : picked;
    if (chosen) setRegionPath(chosen);
  }, []);

  const recipeSourceMissing = (starSource === "regions" && regionPath.trim() === "") || (starSource === "ra_dec" && radecText.trim() === "");
  const recipeSizesInvalid = cutoutSize % 2 === 0 || psfSize % 2 === 0 || psfSize > cutoutSize;

  return (
    <div className="flex flex-col gap-3 p-4">
      <SectionHeader icon={ICON} title="PSF Estimation" subtitle="Empirical" />

      {!compositeMode && !selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to estimate PSF.</div>
      )}
      {compositeMode && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          {compositeModeNotice(fileName)}
        </div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Stars to sample" value={numStars} min={1} max={50} step={1} disabled={loading} accent="violet" onChange={setNumStars} />
        <Slider label="Cutout radius" value={cutoutRadius} min={5} max={50} step={1} disabled={loading} accent="violet" format={(v) => `${v}px`} onChange={setCutoutRadius} />
        <Slider label="Max ellipticity" value={maxEllipticity} min={0.05} max={1} step={0.05} disabled={loading} accent="violet" format={(v) => v.toFixed(2)} onChange={setMaxEllipticity} />
        <Slider label="Saturation threshold" value={satThreshold} min={0.5} max={1} step={0.05} disabled={loading} accent="violet" format={(v) => v.toFixed(2)} onChange={setSatThreshold} />
      </div>

      <div title={disabledReason ?? busyTitle}>
        <RunButton label="Estimate PSF" runningLabel="Estimating..." running={loading} disabled={!canRun || blocked || !!disabledReason} describedBy={disabledReason ? disabledReasonId : undefined} accent="violet" onClick={handleEstimate} />
      </div>
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          <div className="flex items-center gap-3 ab-metric-card p-3">
            <canvas
              ref={canvasRef}
              className="rounded border border-zinc-700 flex-shrink-0"
              style={{ width: 100, height: 100, imageRendering: "pixelated" }}
            />
            <div className="flex flex-col gap-1.5">
              <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
                <div className="text-zinc-500" title="Mean over the stars used of their width at half maximum. The Resolution section of the Photometry tool measures the image PSF with the photometry estimator.">FWHM</div>
                <div className="text-zinc-200 font-mono">{`${result.average_fwhm.toFixed(2)} px${scale !== null ? ` (${(result.average_fwhm * scale).toFixed(3)}")` : ""}`}</div>
                <div className="text-zinc-500" title={SIGMA_MAD_TITLE}>Star scatter</div>
                <div className="text-zinc-200 font-mono">{starScatter !== null ? `± ${starScatter.toFixed(2)} px${scale !== null ? ` (${(starScatter * scale).toFixed(3)}")` : ""}` : "--"}</div>
                <div className="text-zinc-500">Ellipticity</div>
                <div className="text-zinc-200 font-mono">{result.average_ellipticity.toFixed(3)}</div>
                <div className="text-zinc-500">Spread</div>
                <div className="text-zinc-200 font-mono">{result.spread_pixels.toFixed(2)} px</div>
                <div className="text-zinc-500">Stars</div>
                <div className="text-zinc-200 font-mono">{result.stars_used.length} / {result.stars_used.length + result.stars_rejected}</div>
              </div>
            </div>
          </div>

          <div className="flex flex-col gap-0.5">
            {result.stars_used.map((s, i) => (
              <div key={i} className="flex items-center gap-2 text-[10px] text-zinc-500 font-mono px-1">
                <span className="text-violet-400/90">#{i + 1}</span>
                <span>({s.x.toFixed(0)},{s.y.toFixed(0)})</span>
                <span>FWHM={s.fwhm.toFixed(1)} px</span>
                <span>SNR={s.snr.toFixed(0)}</span>
                <span>e={s.ellipticity.toFixed(2)}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {!compositeMode && (
        <div className="flex flex-col gap-3 border-t border-zinc-800 pt-3">
          <div className="flex items-baseline justify-between">
            <span className="text-xs font-semibold text-zinc-200">LIGHTSTACK recipe</span>
            <span className="text-[10px] text-zinc-500">median stack of chosen stars</span>
          </div>

          <div className="flex items-center justify-between">
            <label htmlFor={sourceId} className="text-xs text-zinc-400">Stars</label>
            <select id={sourceId} value={starSource} onChange={(e) => setStarSource(e.target.value as PsfStarSource["kind"])} disabled={recipeRunning} className="ab-select">
              <option value="auto">Automatic finder</option>
              <option value="regions">DS9 region file</option>
              <option value="ra_dec">RA/Dec list</option>
            </select>
          </div>
          {starSource === "auto" && (
            <div className="text-[10px] text-zinc-500 px-1">{`The finder above proposes up to ${numStars} stars.`}</div>
          )}
          {starSource === "regions" && (
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={regionPath}
                onChange={(e) => setRegionPath(e.target.value)}
                placeholder="stars_f444w.reg"
                spellCheck={false}
                disabled={recipeRunning}
                aria-label="DS9 region file (physical, image or sky frame)"
                className="flex-1 min-w-0 bg-zinc-900/80 border border-zinc-700/50 rounded px-2 py-1 text-[11px] font-mono text-zinc-200 placeholder:text-zinc-600 focus:border-violet-400"
              />
              <button
                type="button"
                onClick={() => void handlePickRegions().catch((e: unknown) => console.warn("[AstroBurst] The region file dialog failed:", e))}
                disabled={recipeRunning}
                className="px-3 py-1.5 rounded text-[10px] text-zinc-400 hover:text-zinc-200 border border-zinc-700/50 hover:border-zinc-600 transition-all disabled:opacity-40"
              >
                Choose
              </button>
            </div>
          )}
          {starSource === "ra_dec" && (
            <textarea
              value={radecText}
              onChange={(e) => setRadecText(e.target.value)}
              placeholder={"RA Dec [name] per line, degrees or sexagesimal\n53.16250 -27.79140 star1"}
              spellCheck={false}
              disabled={recipeRunning}
              aria-label="RA/Dec star list"
              className="w-full h-20 resize-y bg-zinc-900/80 border border-zinc-700/50 rounded px-2 py-1 text-[11px] font-mono text-zinc-200 placeholder:text-zinc-600 focus:border-violet-400"
            />
          )}

          <Slider label="Cutout size" value={cutoutSize} min={11} max={151} step={2} disabled={recipeRunning} accent="violet" format={(v) => `${v}px`} onChange={setCutoutSize} />
          <Slider label="PSF size" value={psfSize} min={5} max={151} step={2} disabled={recipeRunning} accent="violet" format={(v) => `${v}px`} onChange={setPsfSize} />
          <div className="flex items-center justify-between">
            <label htmlFor={unitId} className="text-xs text-zinc-400">Radius unit</label>
            <select
              id={unitId}
              value={radiusUnit}
              onChange={(e) => {
                const unit = e.target.value as PsfRadiusUnit;
                setRadiusUnit(unit);
                setNormalizationRadius(unit === "arcsec" ? 1 : 20);
              }}
              disabled={recipeRunning}
              className="ab-select"
            >
              <option value="pixel">Pixels</option>
              <option value="arcsec">Arcsec</option>
            </select>
          </div>
          {radiusUnit === "pixel" ? (
            <Slider label="Normalization radius" value={normalizationRadius} min={1} max={75} step={1} disabled={recipeRunning} accent="violet" format={(v) => `${v}px`} onChange={setNormalizationRadius} />
          ) : (
            <Slider label="Normalization radius" value={normalizationRadius} min={0.1} max={30} step={0.1} disabled={recipeRunning} accent="violet" format={(v) => `${v.toFixed(1)}"`} onChange={setNormalizationRadius} />
          )}
          <Slider label="Oversampling" value={oversampling} min={1} max={4} step={1} disabled={recipeRunning} accent="violet" format={(v) => (v === 1 ? "None" : `x${v}`)} onChange={setOversampling} />
          <div className="flex items-center justify-between">
            <label htmlFor={stackId} className="text-xs text-zinc-400">Stack</label>
            <select id={stackId} value={stack} onChange={(e) => setStack(e.target.value as PsfStackMethod)} disabled={recipeRunning} className="ab-select">
              <option value="median">Median</option>
              <option value="mean">Mean</option>
            </select>
          </div>
          <Toggle label="Radial cleaning" checked={radialCleaning} disabled={recipeRunning} accent="violet" onChange={setRadialCleaning} />
          <Toggle label="Save PSF FITS" checked={saveFits} disabled={recipeRunning} accent="violet" onChange={setSaveFits} />
          {recipeSizesInvalid && (
            <div className="text-[10px] text-amber-300 px-1">The cutout and PSF sizes must be odd, and the PSF cannot be larger than the cutout.</div>
          )}

          <div title={disabledReason ?? recipeBusyTitle}>
            <RunButton
              label="Build empirical PSF"
              runningLabel="Building..."
              running={recipeRunning}
              disabled={!selectedFile || recipeBlocked || recipeSourceMissing || recipeSizesInvalid || !!disabledReason}
              describedBy={disabledReason ? disabledReasonId : undefined}
              accent="violet"
              onClick={handleRecipe}
            />
          </div>
          <ErrorAlert message={recipeError} />
          {recipeResult && <EmpiricalPsfResult report={recipeResult} />}
        </div>
      )}
    </div>
  );
}
