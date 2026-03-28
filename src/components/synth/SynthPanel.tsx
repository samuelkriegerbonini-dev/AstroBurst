import { useState, useCallback } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import {
  generateSynth,
  generateSynthStack,
  type SynthConfig,
  type FieldType,
  type PsfType,
} from "../../services/synth.service";
import { Slider, RunButton, ErrorAlert, SectionHeader, Toggle } from "../ui";

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-rose-400">
    <path d="M9 3h6l1 7h-8l1-7z" />
    <path d="M5 10h14l-2 11H7L5 10z" />
    <circle cx="12" cy="16" r="2" opacity="0.4" />
  </svg>
);

type FieldChoice = "uniform" | "king" | "disk";
type PsfChoice = "gaussian" | "moffat" | "airy";

function buildFieldType(choice: FieldChoice, coreR: number, tidalR: number, scaleLen: number, incl: number): FieldType {
  switch (choice) {
    case "king": return { KingCluster: { core_radius: coreR, tidal_radius: tidalR } };
    case "disk": return { ExponentialDisk: { scale_length: scaleLen, inclination_deg: incl } };
    default: return "Uniform";
  }
}

function buildPsfType(choice: PsfChoice, fwhm: number, beta: number, lambdaD: number): PsfType {
  switch (choice) {
    case "moffat": return { Moffat: { fwhm, beta } };
    case "airy": return { Airy: { lambda_over_d: lambdaD } };
    default: return { Gaussian: { fwhm } };
  }
}

export default function SynthPanel() {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ stars: number; path: string } | null>(null);

  const [width, setWidth] = useState(2048);
  const [height, setHeight] = useState(2048);
  const [nStars, setNStars] = useState(500);
  const [fluxMin, setFluxMin] = useState(100);
  const [fluxMax, setFluxMax] = useState(50000);
  const [seed, setSeed] = useState(42);

  const [fieldChoice, setFieldChoice] = useState<FieldChoice>("uniform");
  const [coreRadius, setCoreRadius] = useState(50.0);
  const [tidalRadius, setTidalRadius] = useState(400.0);
  const [scaleLength, setScaleLength] = useState(200.0);
  const [inclination, setInclination] = useState(30.0);

  const [psfChoice, setPsfChoice] = useState<PsfChoice>("gaussian");
  const [fwhm, setFwhm] = useState(3.0);
  const [beta, setBeta] = useState(4.0);
  const [lambdaD, setLambdaD] = useState(2.5);

  const [gain, setGain] = useState(1.5);
  const [readNoise, setReadNoise] = useState(8.0);
  const [skyBg, setSkyBg] = useState(200.0);
  const [darkCurrent, setDarkCurrent] = useState(0.05);
  const [expTime, setExpTime] = useState(300.0);
  const [biasLevel, setBiasLevel] = useState(1000.0);

  const [vignette, setVignette] = useState(false);
  const [vigStrength, setVigStrength] = useState(0.3);

  const [saveCatalog, setSaveCatalog] = useState(true);
  const [saveGt, setSaveGt] = useState(false);

  const [stackMode, setStackMode] = useState(false);
  const [nFrames, setNFrames] = useState(8);

  const buildConfig = useCallback((): SynthConfig => ({
    field: { width, height, n_stars: nStars, flux_min: fluxMin, flux_max: fluxMax, seed },
    field_type: buildFieldType(fieldChoice, coreRadius, tidalRadius, scaleLength, inclination),
    psf_type: buildPsfType(psfChoice, fwhm, beta, lambdaD),
    noise: { gain, readout_noise: readNoise, sky_background: skyBg, dark_current: darkCurrent, exposure_time: expTime, bias_level: biasLevel, seed: seed + 1000 },
    apply_vignette: vignette,
    vignette_strength: vigStrength,
    n_frames: stackMode ? nFrames : 1,
  }), [width, height, nStars, fluxMin, fluxMax, seed, fieldChoice, coreRadius, tidalRadius, scaleLength, inclination, psfChoice, fwhm, beta, lambdaD, gain, readNoise, skyBg, darkCurrent, expTime, biasLevel, vignette, vigStrength, stackMode, nFrames]);

  const handleGenerate = useCallback(async () => {
    setLoading(true);
    setError(null);
    setResult(null);
    try {
      const config = buildConfig();

      if (stackMode) {
        const dir = await save({ title: "Choose output directory", defaultPath: "synth_stack" });
        if (!dir) { setLoading(false); return; }
        const res = await generateSynthStack(config, dir, "synth");
        setResult({ stars: res.star_count, path: res.output_path ?? dir });
      } else {
        const path = await save({ title: "Save synthetic FITS", defaultPath: "synthetic.fits", filters: [{ name: "FITS", extensions: ["fits", "fit"] }] });
        if (!path) { setLoading(false); return; }
        const catPath = saveCatalog ? path.replace(/\.fits?$/i, "_catalog.csv") : undefined;
        const gtPath = saveGt ? path.replace(/\.fits?$/i, "_groundtruth.fits") : undefined;
        const res = await generateSynth(config, path, saveCatalog, catPath, saveGt, gtPath);
        setResult({ stars: res.star_count, path: res.output_path ?? path });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [buildConfig, stackMode, saveCatalog, saveGt]);

  const FIELD_OPTS: { value: FieldChoice; label: string }[] = [
    { value: "uniform", label: "Uniform" },
    { value: "king", label: "King Cluster" },
    { value: "disk", label: "Exp. Disk" },
  ];

  const PSF_OPTS: { value: PsfChoice; label: string }[] = [
    { value: "gaussian", label: "Gaussian" },
    { value: "moffat", label: "Moffat" },
    { value: "airy", label: "Airy" },
  ];

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
        <Slider label="Seed" value={seed} min={0} max={9999} step={1} disabled={loading} accent="rose" onChange={setSeed} />
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">Distribution</div>
        <div className="flex gap-1 px-1">
          {FIELD_OPTS.map((o) => (
            <button key={o.value} onClick={() => setFieldChoice(o.value)}
              className={`px-2.5 py-1 rounded text-[10px] font-medium transition-all ${fieldChoice === o.value ? "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30" : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"}`}>
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
            <Slider label="Inclination" value={inclination} min={0} max={85} step={1} disabled={loading} accent="rose" format={(v) => `${v}\u00B0`} onChange={setInclination} />
          </>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">PSF Model</div>
        <div className="flex gap-1 px-1">
          {PSF_OPTS.map((o) => (
            <button key={o.value} onClick={() => setPsfChoice(o.value)}
              className={`px-2.5 py-1 rounded text-[10px] font-medium transition-all ${psfChoice === o.value ? "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30" : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"}`}>
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
          <Slider label="\u03BB/D" value={lambdaD} min={0.5} max={10} step={0.1} disabled={loading} accent="rose" format={(v) => `${v.toFixed(1)}px`} onChange={setLambdaD} />
        )}
      </div>

      <div className="flex flex-col gap-2">
        <div className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider px-1">CCD Noise</div>
        <Slider label="Gain" value={gain} min={0.1} max={10} step={0.1} disabled={loading} accent="rose" format={(v) => `${v.toFixed(1)} e\u207B/ADU`} onChange={setGain} />
        <Slider label="Read noise" value={readNoise} min={0} max={50} step={0.5} disabled={loading} accent="rose" format={(v) => `${v.toFixed(1)} e\u207B`} onChange={setReadNoise} />
        <Slider label="Sky background" value={skyBg} min={0} max={2000} step={10} disabled={loading} accent="rose" format={(v) => `${v.toFixed(0)} ADU`} onChange={setSkyBg} />
        <Slider label="Exposure" value={expTime} min={1} max={3600} step={1} disabled={loading} accent="rose" format={(v) => `${v.toFixed(0)}s`} onChange={setExpTime} />
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
          <Slider label="Frames" value={nFrames} min={2} max={64} step={1} disabled={loading} accent="rose" onChange={setNFrames} />
        )}
      </div>

      <RunButton label={stackMode ? `Generate ${nFrames} Frames` : "Generate FITS"} runningLabel="Generating..." running={loading} accent="rose" onClick={handleGenerate} />
      <ErrorAlert message={error} />

      {result && (
        <div className="animate-fade-in ab-metric-card p-3">
          <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
            <div className="text-zinc-500">Stars</div>
            <div className="text-zinc-200 font-mono">{result.stars}</div>
            <div className="text-zinc-500">Size</div>
            <div className="text-zinc-200 font-mono">{width} x {height}</div>
            <div className="text-zinc-500">Output</div>
            <div className="text-zinc-200 font-mono text-[10px] truncate" title={result.path}>{result.path.split(/[/\\]/).pop()}</div>
          </div>
        </div>
      )}
    </div>
  );
}
