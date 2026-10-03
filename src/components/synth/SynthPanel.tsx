import { useState, useCallback, useMemo } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { generateSynth, generateSynthStack } from "../../services/synth";
import { synthOutputPaths, synthStackOutputPaths } from "../../utils/synthPaths";
import { formatCount } from "../../utils/formatCount";
import { formatByteSize, synthStackEstimate } from "../../utils/synthBudget";
import {
  SYNTH_PANEL_DEFAULTS as D,
  SYNTH_SEED_MAX,
  buildSynthConfig,
  nextRandomSeed,
  synthResultCard,
  synthSettingsSignature,
  type SynthFieldChoice,
  type SynthPanelState,
  type SynthPsfChoice,
  type SynthResultCard,
} from "../../utils/synthConfig";
import { ingestFiles } from "../../hooks/useFileIngest";
import { fileStore } from "../../hooks/useFileStore";
import { overwrittenFileIds } from "../../utils/overwrittenFiles";
import { astroFileFromPath } from "../../utils/validation";
import { Slider, RunButton, ErrorAlert, SectionHeader, Toggle } from "../ui";

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-rose-400">
    <path d="M9 3h6l1 7h-8l1-7z" />
    <path d="M5 10h14l-2 11H7L5 10z" />
    <circle cx="12" cy="16" r="2" opacity="0.4" />
  </svg>
);

const DICE_ICON = (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
    <rect x="3" y="3" width="18" height="18" rx="3" />
    <circle cx="8" cy="8" r="1.2" fill="currentColor" />
    <circle cx="16" cy="8" r="1.2" fill="currentColor" />
    <circle cx="12" cy="12" r="1.2" fill="currentColor" />
    <circle cx="8" cy="16" r="1.2" fill="currentColor" />
    <circle cx="16" cy="16" r="1.2" fill="currentColor" />
  </svg>
);

const STACK_PREFIX = "synth";

const FIELD_OPTS: { value: SynthFieldChoice; label: string }[] = [
  { value: "uniform", label: "Uniform" },
  { value: "king", label: "King Cluster" },
  { value: "disk", label: "Exp. Disk" },
];

const PSF_OPTS: { value: SynthPsfChoice; label: string }[] = [
  { value: "gaussian", label: "Gaussian" },
  { value: "moffat", label: "Moffat" },
  { value: "airy", label: "Airy" },
];

function formatFlux(value: number): string {
  return `${formatCount(value)} e⁻`;
}

function fileName(path: string): string {
  return path.split(/[/\\]/).pop() ?? path;
}

function OutputRow({ label, path }: { label: string; path: string }) {
  return (
    <>
      <div className="text-zinc-500">{label}</div>
      <div className="text-zinc-200 font-mono text-[10px] truncate" title={path}>{fileName(path)}</div>
    </>
  );
}

export default function SynthPanel() {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SynthResultCard | null>(null);

  const [width, setWidth] = useState(D.width);
  const [height, setHeight] = useState(D.height);
  const [nStars, setNStars] = useState(D.nStars);
  const [fluxMin, setFluxMin] = useState(D.fluxMin);
  const [fluxMax, setFluxMax] = useState(D.fluxMax);
  const [seed, setSeed] = useState(D.seed);

  const [fieldChoice, setFieldChoice] = useState<SynthFieldChoice>(D.fieldChoice);
  const [coreRadius, setCoreRadius] = useState(D.coreRadius);
  const [tidalRadius, setTidalRadius] = useState(D.tidalRadius);
  const [scaleLength, setScaleLength] = useState(D.scaleLength);
  const [inclination, setInclination] = useState(D.inclination);

  const [psfChoice, setPsfChoice] = useState<SynthPsfChoice>(D.psfChoice);
  const [fwhm, setFwhm] = useState(D.fwhm);
  const [beta, setBeta] = useState(D.beta);
  const [lambdaD, setLambdaD] = useState(D.lambdaD);

  const [gain, setGain] = useState(D.gain);
  const [readNoise, setReadNoise] = useState(D.readNoise);
  const [skyBg, setSkyBg] = useState(D.skyBg);
  const [darkCurrent, setDarkCurrent] = useState(D.darkCurrent);
  const [expTime, setExpTime] = useState(D.expTime);
  const [biasLevel, setBiasLevel] = useState(D.biasLevel);

  const [vignette, setVignette] = useState(D.vignette);
  const [vigStrength, setVigStrength] = useState(D.vigStrength);

  const [saveCatalog, setSaveCatalog] = useState(true);
  const [saveGt, setSaveGt] = useState(false);

  const [stackMode, setStackMode] = useState(D.stackMode);
  const [nFrames, setNFrames] = useState(D.nFrames);
  const [varyFrames, setVaryFrames] = useState(D.varyFrames);
  const [ditherPx, setDitherPx] = useState(D.ditherPx);
  const [seeingJitterPct, setSeeingJitterPct] = useState(D.seeingJitterPct);
  const [cosmicRays, setCosmicRays] = useState(D.cosmicRays);

  const handleFluxMin = useCallback((value: number) => {
    const rounded = Math.round(value);
    setFluxMin(rounded);
    setFluxMax((prev) => (prev <= rounded ? rounded + 1 : prev));
  }, []);

  const handleFluxMax = useCallback((value: number) => {
    const rounded = Math.round(value);
    setFluxMax(rounded);
    setFluxMin((prev) => (prev >= rounded ? Math.max(1, rounded - 1) : prev));
  }, []);

  const handleRandomSeed = useCallback(() => {
    setSeed((prev) => nextRandomSeed(prev));
  }, []);

  const panelState = useMemo((): SynthPanelState => ({
    width, height, nStars, fluxMin, fluxMax, seed,
    fieldChoice, coreRadius, tidalRadius, scaleLength, inclination,
    psfChoice, fwhm, beta, lambdaD,
    gain, readNoise, skyBg, darkCurrent, expTime, biasLevel,
    vignette, vigStrength,
    stackMode, nFrames, varyFrames, ditherPx, seeingJitterPct, cosmicRays,
  }), [width, height, nStars, fluxMin, fluxMax, seed, fieldChoice, coreRadius, tidalRadius, scaleLength, inclination, psfChoice, fwhm, beta, lambdaD, gain, readNoise, skyBg, darkCurrent, expTime, biasLevel, vignette, vigStrength, stackMode, nFrames, varyFrames, ditherPx, seeingJitterPct, cosmicRays]);

  const config = useMemo(() => buildSynthConfig(panelState), [panelState]);
  const signature = useMemo(
    () => synthSettingsSignature(config, { stackMode, saveCatalog, saveGroundTruth: saveGt }),
    [config, stackMode, saveCatalog, saveGt],
  );
  const stale = result !== null && result.signature !== signature;

  const stackEstimate = synthStackEstimate(width, height, nFrames);
  const stackOverBudget = stackMode ? stackEstimate.overBudgetReason : null;

  const handleGenerate = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      if (stackMode) {
        const picked = await open({ directory: true, multiple: false, title: "Choose output folder" });
        const dir = typeof picked === "string" ? picked : null;
        if (!dir) return;
        setResult(null);
        const res = await generateSynthStack(config, dir, STACK_PREFIX, saveCatalog, saveGt);
        const sidecars = synthStackOutputPaths(res.output_path || dir, STACK_PREFIX);
        setResult(synthResultCard(res, {
          kind: "stack",
          fallbackPath: dir,
          signature,
          catalogPath: saveCatalog ? sidecars.catalog : null,
          groundTruthPath: saveGt ? sidecars.groundTruth : null,
        }));
      } else {
        const path = await save({ title: "Save synthetic FITS", defaultPath: "synthetic.fits", filters: [{ name: "FITS", extensions: ["fits", "fit"] }] });
        if (!path) return;
        setResult(null);
        const out = synthOutputPaths(path);
        const catalogPath = saveCatalog ? out.catalog : undefined;
        const groundTruthPath = saveGt ? out.groundTruth : undefined;
        const res = await generateSynth(config, out.fits, saveCatalog, catalogPath, saveGt, groundTruthPath);
        setResult(synthResultCard(res, { kind: "single", fallbackPath: out.fits, signature, catalogPath, groundTruthPath }));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [config, signature, stackMode, saveCatalog, saveGt]);

  const handleOpenInViewer = useCallback(() => {
    if (!result?.openPath) return;
    const [openId] = overwrittenFileIds(fileStore.getFiles(), [result.openPath]);
    if (openId) {
      fileStore.selectFile(openId);
      return;
    }
    if (!ingestFiles([astroFileFromPath(result.openPath)])) setError("The viewer is not ready to open files yet.");
  }, [result]);

  return (
    <div className="flex flex-col gap-3 p-4">
      <SectionHeader icon={ICON} title="Synthetic Generator" subtitle="Star Fields & CCD Noise" />

      <div className="text-[10px] text-zinc-600 px-1">
        Generate synthetic astronomical images with configurable star distributions, PSF models, and realistic CCD noise for testing processing pipelines.
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">Image</div>
        <Slider label="Width" value={width} min={256} max={8192} step={256} disabled={loading} accent="rose" format={(v) => `${v}px`} onChange={setWidth} />
        <Slider label="Height" value={height} min={256} max={8192} step={256} disabled={loading} accent="rose" format={(v) => `${v}px`} onChange={setHeight} />
        <Slider label="Stars" value={nStars} min={10} max={5000} step={10} disabled={loading} accent="rose" onChange={setNStars} />
        <Slider label="Flux min" value={fluxMin} min={1} max={100000} step={1} scale="log" disabled={loading} accent="rose" format={formatFlux} onChange={handleFluxMin} />
        <Slider label="Flux max" value={fluxMax} min={1} max={1000000} step={1} scale="log" disabled={loading} accent="rose" format={formatFlux} onChange={handleFluxMax} />
        <div className="flex items-end gap-1.5">
          <div className="flex-1 min-w-0">
            <Slider label="Seed" value={seed} min={0} max={SYNTH_SEED_MAX} step={1} disabled={loading} accent="rose" onChange={setSeed} />
          </div>
          <button
            type="button"
            aria-label="New random seed"
            title="New random seed"
            disabled={loading}
            onClick={handleRandomSeed}
            className="mb-0.5 p-1 rounded text-zinc-400 hover:text-rose-400 hover:bg-zinc-800/50 disabled:opacity-40 disabled:pointer-events-none"
          >
            {DICE_ICON}
          </button>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">Distribution</div>
        <div className="flex gap-1 px-1">
          {FIELD_OPTS.map((o) => (
            <button key={o.value} type="button" disabled={loading} aria-pressed={fieldChoice === o.value} onClick={() => setFieldChoice(o.value)}
              className={`px-2.5 py-1 rounded text-[10px] font-medium transition-all disabled:opacity-40 disabled:pointer-events-none ${fieldChoice === o.value ? "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30" : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"}`}>
              {o.label}
            </button>
          ))}
        </div>
        {fieldChoice === "king" && (
          <>
            <Slider label="Core radius" value={coreRadius} min={5} max={200} step={5} disabled={loading} accent="rose" format={(v) => `${v}px`} onChange={setCoreRadius} />
            <Slider label="Tidal radius" value={tidalRadius} min={50} max={1500} step={50} disabled={loading} accent="rose" format={(v) => `${v}px`} onChange={setTidalRadius} />
          </>
        )}
        {fieldChoice === "disk" && (
          <>
            <Slider label="Scale length" value={scaleLength} min={20} max={800} step={10} disabled={loading} accent="rose" format={(v) => `${v}px`} onChange={setScaleLength} />
            <Slider label="Inclination" value={inclination} min={0} max={85} step={1} disabled={loading} accent="rose" format={(v) => `${v}°`} onChange={setInclination} />
          </>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">PSF Model</div>
        <div className="flex gap-1 px-1">
          {PSF_OPTS.map((o) => (
            <button key={o.value} type="button" disabled={loading} aria-pressed={psfChoice === o.value} onClick={() => setPsfChoice(o.value)}
              className={`px-2.5 py-1 rounded text-[10px] font-medium transition-all disabled:opacity-40 disabled:pointer-events-none ${psfChoice === o.value ? "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30" : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"}`}>
              {o.label}
            </button>
          ))}
        </div>
        {(psfChoice === "gaussian" || psfChoice === "moffat") && (
          <Slider label="FWHM" value={fwhm} min={0.5} max={15} step={0.1} disabled={loading} accent="rose" format={(v) => `${v.toFixed(1)}px`} onChange={setFwhm} />
        )}
        {psfChoice === "moffat" && (
          <Slider label="Beta" value={beta} min={1} max={10} step={0.1} disabled={loading} accent="rose" format={(v) => v.toFixed(1)} onChange={setBeta} />
        )}
        {psfChoice === "airy" && (
          <Slider label="λ/D" value={lambdaD} min={0.5} max={10} step={0.1} disabled={loading} accent="rose" format={(v) => `${v.toFixed(1)}px`} onChange={setLambdaD} />
        )}
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">CCD Noise</div>
        <Slider label="Gain" value={gain} min={0.1} max={10} step={0.1} disabled={loading} accent="rose" format={(v) => `${v.toFixed(1)} e⁻/ADU`} onChange={setGain} />
        <Slider label="Read noise" value={readNoise} min={0} max={50} step={0.5} disabled={loading} accent="rose" format={(v) => `${v.toFixed(1)} e⁻`} onChange={setReadNoise} />
        <Slider label="Sky background" value={skyBg} min={0} max={2000} step={10} disabled={loading} accent="rose" format={(v) => `${v.toFixed(0)} ADU`} onChange={setSkyBg} />
        <Slider label="Dark current" value={darkCurrent} min={0} max={5} step={0.01} disabled={loading} accent="rose" format={(v) => `${v.toFixed(2)} e⁻/s`} onChange={setDarkCurrent} />
        <Slider label="Exposure (s)" value={expTime} min={1} max={3600} step={1} disabled={loading} accent="rose" hint="sets EXPTIME and dark current; star flux and sky are per-frame totals" format={(v) => v.toFixed(0)} onChange={setExpTime} />
        <Slider label="Bias level" value={biasLevel} min={0} max={5000} step={10} disabled={loading} accent="rose" format={(v) => `${v.toFixed(0)} ADU`} onChange={setBiasLevel} />
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">Options</div>
        <Toggle label="Vignetting" checked={vignette} disabled={loading} onChange={setVignette} />
        {vignette && (
          <Slider label="Vignette strength" value={vigStrength} min={0} max={1} step={0.05} disabled={loading} accent="rose" format={(v) => v.toFixed(2)} onChange={setVigStrength} />
        )}
        <Toggle label="Save star catalog (.csv)" checked={saveCatalog} disabled={loading} onChange={setSaveCatalog} />
        <Toggle label="Save ground truth" checked={saveGt} disabled={loading} onChange={setSaveGt} />
        <Toggle label="Stack mode (multi-frame)" checked={stackMode} disabled={loading} onChange={setStackMode} />
        {stackMode && (
          <>
            <Slider label="Frames" value={nFrames} min={2} max={64} step={1} disabled={loading} accent="rose" hint={` ≈ ${formatByteSize(stackEstimate.bytes)} on disk`} onChange={setNFrames} />
            <Toggle label="Vary frames" checked={varyFrames} disabled={loading} onChange={setVaryFrames} />
            {varyFrames && (
              <>
                <Slider label="Dither (px)" value={ditherPx} min={0} max={20} step={0.5} disabled={loading} accent="rose" format={(v) => v.toFixed(1)} onChange={setDitherPx} />
                <Slider label="Seeing jitter %" value={seeingJitterPct} min={0} max={50} step={1} disabled={loading} accent="rose" format={(v) => `${v.toFixed(0)}%`} onChange={setSeeingJitterPct} />
                <Toggle label="Cosmic rays" checked={cosmicRays} disabled={loading} onChange={setCosmicRays} />
              </>
            )}
          </>
        )}
      </div>

      <RunButton label={stackMode ? `Generate ${nFrames} Frames` : "Generate FITS"} runningLabel="Generating..." running={loading} disabled={!!stackOverBudget} accent="rose" onClick={handleGenerate} />
      {stackOverBudget && <div className="text-[10px] text-amber-400/90 px-1">{stackOverBudget}</div>}
      <ErrorAlert message={error} />

      {result && (
        <div className="animate-fade-in ab-metric-card p-3 flex flex-col gap-2">
          <div className={stale ? "text-[10px] text-amber-400/90" : "sr-only"} role="status">{stale ? "Settings changed since generation" : ""}</div>
          <div className={`grid grid-cols-2 gap-x-4 gap-y-1 text-xs ${stale ? "opacity-60" : ""}`}>
            <div className="text-zinc-500">Stars</div>
            <div className="text-zinc-200 font-mono">{result.stars}</div>
            <div className="text-zinc-500">Size</div>
            <div className="text-zinc-200 font-mono">{result.width} x {result.height}</div>
            <OutputRow label={result.kind === "stack" ? "Folder" : "Output"} path={result.path} />
            {result.manifestPath && <OutputRow label="Frames manifest" path={result.manifestPath} />}
            {result.catalogPath && <OutputRow label="Catalog" path={result.catalogPath} />}
            {result.groundTruthPath && <OutputRow label="Ground truth" path={result.groundTruthPath} />}
          </div>
          {result.openPath && (
            <button
              type="button"
              onClick={handleOpenInViewer}
              className="self-start px-2.5 py-1 rounded text-[10px] font-medium text-rose-400 ring-1 ring-rose-500/30 hover:bg-rose-600/20"
            >
              Open in viewer
            </button>
          )}
        </div>
      )}
    </div>
  );
}
