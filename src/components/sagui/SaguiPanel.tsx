import { useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { openUrl } from "@tauri-apps/plugin-opener";
import { showWhenLoaded } from "../../hooks/useFileIngest";
import { fileStore } from "../../hooks/useFileStore";
import type { SaguiConfig, SaguiProgress, SaguiSedScaling } from "../../shared/types/sagui";
import { ErrorAlert, RunButton, WarningList } from "../ui";
import { PREP_BUTTON, PREP_FIELD, PrepField } from "../cubeprep/CubePrepStepEditor";
import { cubePrepError, inputDirectory } from "../cubeprep/cubePrepModel";
import { PathNote } from "../cubeprep/CubePrepPanel";
import { cubePrepStore } from "../cubeprep/cubePrepStore";
import { saguiConfigProblems, saguiStore, useSaguiState, type SaguiState } from "./saguiStore";
import SaguiInputPsf from "./SaguiInputPsf";
import SaguiViewer from "./SaguiViewer";

const R_INSTALLERS = "https://cran.r-project.org/bin/windows/base/old/4.6.2/";
type PathKey = "input_path" | "output_dir" | "rscript_path" | "library_path";
type CheckKind = "inspect" | "runtime" | "prepared";
const CHECK_TEXT: Record<CheckKind, string> = { inspect: "Checking cube and official runtime…", runtime: "Checking the R installation…", prepared: "Checking the prepared cube…" };

function checkingText(state: SaguiState, kind: CheckKind | null): string {
  if (state.operation === "install") return "Preparing or verifying the private SAGUI library…";
  if (state.operation === "restore") return "Verifying previous SAGUI result…";
  if (state.operation === "segment") return CHECK_TEXT.inspect;
  return kind ? CHECK_TEXT[kind] : "Checking…";
}

function cancelledText(state: SaguiState): string {
  if (state.operation === "install") return "Installation cancelled. Any partial files and logs are in the installation folder.";
  return state.outputDir ? "Cancelled. Any partial files and logs remain in the run folder." : "Cancelled before SAGUI started; no run folder was created.";
}

function stageParts(stage: string): [string, string] {
  const separator = /: | - /.exec(stage);
  return separator ? [stage.slice(0, separator.index), stage.slice(separator.index + separator[0].length)] : [stage, ""];
}

function progressDetail(progress: SaguiProgress): string {
  return [`${Math.round(progress.percent)}%`, stageParts(progress.stage)[1]].filter(Boolean).join(" · ");
}

export default function SaguiPanel() {
  const state = useSaguiState();
  const { config, input, runtime, result, progress } = state;
  const [localError, setLocalError] = useState<string | null>(null);
  const [scaleText, setScaleText] = useState(config.starlet_scales.join(", "));
  const [checkKind, setCheckKind] = useState<CheckKind | null>(null);
  const busy = ["checking", "running", "installing", "cancelling"].includes(state.status);
  const segmenting = busy && state.operation === "segment";
  const problems = saguiConfigProblems(config);
  const edit = (change: Partial<SaguiConfig>) => { setLocalError(null); saguiStore.edit({ ...config, ...change }); };
  const perform = async (operation: () => Promise<void>) => { setLocalError(null); try { await operation(); } catch (error) { setLocalError(cubePrepError(error)); } };
  const check = async (kind: CheckKind, action: () => Promise<unknown>) => { setCheckKind(kind); try { await action(); } finally { setCheckKind(null); } };
  const browse = (key: PathKey, directory: boolean) => void perform(async () => {
    const path = await open({ directory, multiple: false });
    if (typeof path !== "string") return;
    const current = saguiStore.get().config;
    saguiStore.edit({ ...current, [key]: path, ...(key === "input_path" && !current.output_dir ? { output_dir: inputDirectory(path) } : {}) });
  });
  const fileField = (key: PathKey, label: string, directory: boolean, name: string) => <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label={label}><input className={PREP_FIELD} value={config[key]} onChange={(e) => edit({ [key]: e.target.value })} /></PrepField></div><button className={PREP_BUTTON} aria-label={`Browse ${name}`} onClick={() => browse(key, directory)}>Browse</button></div>;
  const setInput = (path: string | undefined | null) => { if (path) edit({ input_path: path, output_dir: config.output_dir || inputDirectory(path) }); else setLocalError("No prepared cube is available. Choose a multiband FITS file."); };
  const useCubePrep = () => void perform(async () => {
    const report = cubePrepStore.get().report;
    if (!report) throw new Error("No prepared cube is available. Run Cube Prep or choose a multiband FITS file.");
    await check("prepared", () => saguiStore.prepareCubePrep(report, () => cubePrepStore.get().report === report));
  });
  const openPrevious = () => void perform(async () => {
    const path = await open({ title: "Open previous SAGUI result", multiple: false, filters: [{ name: "SAGUI manifest", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    await saguiStore.restore(path);
    const restored = saguiStore.get();
    if (restored.status === "restored") setScaleText(restored.config.starlet_scales.join(", "));
  });
  const openFits = (path: string) => { if (!showWhenLoaded(path)) setLocalError("The file queue is unavailable. Open the exported FITS file from Files."); };
  const number = (key: "n_regions" | "starlet_threshold" | "dq_exclude_bits" | "knn_k", label: string, min: number, max?: number) => <PrepField label={label}><input className={PREP_FIELD} type="number" min={min} max={max} step={key === "starlet_threshold" ? "any" : 1} value={Number.isNaN(config[key]) ? "" : config[key]} onChange={(e) => edit({ [key]: e.target.value === "" ? NaN : Number(e.target.value) })} /></PrepField>;

  return <div className="h-full overflow-auto p-3 text-xs text-zinc-300 space-y-3">
    <div><h2 className="font-semibold text-violet-300">SAGUI</h2><p className="mt-1 text-[10px] text-zinc-500">Group pixels by their multiband SED using the official SAGUI R implementation.</p></div>
    <button className={PREP_BUTTON} disabled={busy} onClick={openPrevious}>Open previous SAGUI result</button>
    <fieldset disabled={busy} className="space-y-3 disabled:opacity-60">
      {fileField("input_path", "Prepared multiband FITS cube", false, "prepared multiband FITS cube")}
      <div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={useCubePrep}>Use Cube Prep result</button><button className={PREP_BUTTON} onClick={() => setInput(fileStore.getSelectedFile()?.path)}>Use selected FITS</button></div>
      <p className="text-[10px] text-zinc-500">Requires registered photometric bands, per-pixel flux units, celestial WCS and a verified common PSF. Prepare a MEF cube in Cube Prep to retain available ERR and DQ.</p>
      <details open={!runtime?.ready}><summary className="cursor-pointer text-zinc-200">R and SAGUI installation</summary><div className="mt-2 space-y-3">
        {fileField("rscript_path", "Rscript executable", false, "Rscript executable")}{fileField("library_path", "Private R package library", true, "private R package library")}
        <p className="text-[10px] text-zinc-500">Required: R 4.6.2 (installs alongside other R versions; Windows installer in the <button type="button" title={R_INSTALLERS} className="text-violet-300 underline underline-offset-2 hover:text-violet-200" onClick={() => void perform(async () => { await openUrl(R_INSTALLERS); })}>CRAN archive</button>) and SAGUI commit 56fd3ea776d7c00e056368498da9412bd58b84bb. On Windows, install Rtools45 before building SAGUI.</p>
        <button className={PREP_BUTTON} onClick={() => void check("runtime", () => saguiStore.inspectRuntime())}>Check R installation</button>
        <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Library installation folder"><input className={PREP_FIELD} value={state.installParent} onChange={(e) => saguiStore.setInstallParent(e.target.value)} /></PrepField></div><button className={PREP_BUTTON} aria-label="Browse library installation folder" onClick={() => void perform(async () => { const path = await open({ directory: true, multiple: false }); if (typeof path === "string") saguiStore.setInstallParent(path); })}>Browse</button></div>
        <p className="text-[10px] text-zinc-500">Install or repair downloads the pinned SAGUI package and dependencies into a new private library. The library is selected after verification; existing installations stay available.</p>
        <button className={`${PREP_BUTTON} border-violet-700 text-violet-200`} disabled={!config.rscript_path || !state.installParent} onClick={() => void saguiStore.install()}>Install / repair SAGUI library</button>
      </div></details>
      <div className="grid grid-cols-2 gap-2">{number("n_regions", "Number of regions", 2)}<PrepField label="Clustering"><select className={PREP_FIELD} value={config.mode} onChange={(e) => edit({ mode: e.target.value as SaguiConfig["mode"] })}><option value="ward">Exact Ward</option><option value="sparse">Sparse Ward (approximation)</option></select></PrepField></div>
      {config.mode === "sparse" && <>{number("knn_k", "Nearest neighbours", 2, 4096)}<p className="text-[10px] text-amber-300">Sparse Ward uses a neighbour graph; it is not numerically equivalent to exact Ward.</p></>}
      <PrepField label="Verified common PSF target"><input className={PREP_FIELD} value={config.psf_target} placeholder="e.g. F444W" onChange={(e) => edit({ psf_target: e.target.value })} /></PrepField>
      <label className="flex items-center gap-2"><input type="checkbox" checked={config.starlet} onChange={(e) => edit({ starlet: e.target.checked })} />Use starlet morphology mask</label>
      {config.starlet && <div className="grid grid-cols-2 gap-2"><PrepField label="Starlet scales (1–10)"><input className={PREP_FIELD} value={scaleText} onChange={(e) => { setScaleText(e.target.value); const values = e.target.value.split(/[\s,;]+/).filter(Boolean).map(Number); edit({ starlet_scales: values }); }} /></PrepField>{number("starlet_threshold", "Starlet threshold", 0)}</div>}
      {number("dq_exclude_bits", "DQ bits to exclude (decimal mask)", 0, 4294967295)}
      <p className="text-[10px] text-zinc-500">The default 1 excludes DO_NOT_USE on JWST and Roman planes (REED_SOLOMON on HST planes). 4294967295 excludes all flagged bits; 0 ignores DQ flags. Non-finite samples remain excluded.</p>
      <details><summary className="cursor-pointer text-zinc-200">Advanced clustering</summary><div className="mt-2 space-y-2">
        <PrepField label="SED scaling"><select className={PREP_FIELD} value={config.sed_scaling ?? "median_centre"} onChange={(e) => edit({ sed_scaling: e.target.value as SaguiSedScaling })}><option value="median_centre">Median-centered (SAGUI default)</option><option value="median_mad">Median-centered, divided by MAD</option></select></PrepField>
        <p className="text-[10px] text-zinc-500">Clustering compares pixel spectra after this scaling. The default subtracts each spectrum&apos;s median; the MAD option also divides by the median absolute deviation, falling back to the standard deviation and then 1 when it is zero. Changing it changes the regions.</p>
      </div></details>
      {fileField("output_dir", "Working folder", true, "working folder")}
      <p className="text-[10px] text-zinc-500">Each run creates a separate sagui folder with segmentation FITS, mask FITS, regional photometry CSV, manifest and R logs.</p>
    </fieldset>
    <ErrorAlert message={localError || state.error} />
    {runtime && <div className="space-y-2"><p className={runtime.ready ? "text-cyan-300" : "text-amber-300"}>{runtime.ready ? "Official runtime ready" : "Runtime unavailable or incompatible"}{runtime.r_version ? ` · R ${runtime.r_version}` : ""}{runtime.sagui_version ? ` · SAGUI ${runtime.sagui_version}` : ""}</p><WarningList warnings={runtime.issues} />{runtime.sagui_commit && <p className="break-all text-[10px] text-zinc-500">SAGUI commit: {runtime.sagui_commit}</p>}</div>}
    {input && <div className="space-y-2"><p>{input.width} × {input.height} · {input.bands.length} bands · {input.unit}</p><p className="text-[10px] text-zinc-400">ERR {input.has_err ? "present" : "absent"} · DQ {input.has_dq ? "present" : "absent"} · NOISCORR {input.has_noise_correlation ? "present" : "absent"} · Exact Ward memory estimate: {(input.estimated_memory_bytes / 1024 ** 3).toFixed(2)} GiB</p><p className="text-[10px] text-zinc-500">The estimate uses the full footprint before masks. Before R starts, only the cube and the sparse neighbor graph are checked against free memory; exact Ward checks its own memory in R on the clustered support and refuses more than 65536 pixels.</p><SaguiInputPsf input={input} psfTarget={config.psf_target} nRegions={config.n_regions} /><WarningList warnings={input.warnings} /></div>}
    {problems.length > 0 && <ul className="list-disc pl-4 text-[10px] text-zinc-500 space-y-1">{problems.map((problem) => <li key={problem}>{problem}</li>)}</ul>}
    <div className="flex flex-wrap items-center gap-2"><button className={PREP_BUTTON} disabled={busy || !config.input_path} onClick={() => void check("inspect", () => saguiStore.inspect())}>Check cube and runtime</button><div className="min-w-36 flex-1"><RunButton label="Validate & segment" runningLabel={state.status === "checking" ? "Checking…" : "Segmenting…"} running={segmenting} disabled={busy || problems.length > 0} accent="violet" small onClick={() => void saguiStore.run()} /></div>{busy && saguiStore.isJobActive() && <button className={PREP_BUTTON} disabled={state.status === "cancelling"} onClick={() => void saguiStore.cancel()}>{state.status === "cancelling" ? "Cancelling…" : "Cancel"}</button>}</div>
    <div aria-live="polite" className="space-y-2">{state.status === "checking" && <p>{checkingText(state, checkKind)}</p>}{state.status === "installing" && <p>Installing SAGUI and its dependencies…</p>}{progress && <p>{stageParts(progress.stage)[0]}</p>}{state.status === "cancelled" && <p className="text-amber-300">{cancelledText(state)}</p>}{state.status === "complete" && <p className="text-cyan-300">Segmentation completed by the official R runtime.</p>}{state.status === "restored" && <p className="text-cyan-300">Previous result opened and verified. Its input and analysis settings are shown above; your Rscript, library and working folder were kept.</p>}{state.status === "installed" && <p className="text-cyan-300">Private SAGUI library installed and verified.</p>}</div>
    {progress && <div className="space-y-1"><progress aria-label={state.operation === "install" ? "SAGUI installation progress" : "SAGUI segmentation progress"} className="w-full" value={progress.percent} max={100} /><p className="text-[10px] text-zinc-500">{progressDetail(progress)}</p></div>}
    {state.outputDir && <PathNote label={state.operation === "install" ? "Installation folder" : "Run folder"} path={state.outputDir} />}
    {state.operation === "install" && state.installation && <PathNote label="Installation manifest" path={state.installation.manifest_path} />}
    {state.installation && <WarningList warnings={state.installation.warnings} />}
    {result && <div className="space-y-3"><WarningList warnings={result.warnings} /><SaguiViewer key={result.run_id} result={result} />
      <h3 className="font-semibold">Exported products</h3><div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={() => openFits(result.labels_path)}>Open segmentation FITS</button><button className={PREP_BUTTON} onClick={() => openFits(result.mask_path)}>Open mask FITS</button><button className={PREP_BUTTON} onClick={() => void perform(async () => { const path = await save({ defaultPath: "sagui_photometry.csv", filters: [{ name: "CSV", extensions: ["csv"] }] }); if (path) await writeTextFile(path, await readTextFile(result.table_path)); })}>Save photometry as…</button></div>
      <div className="space-y-1"><PathNote label="Segmentation FITS" path={result.labels_path} /><PathNote label="Mask FITS" path={result.mask_path} /><PathNote label="Photometry CSV" path={result.table_path} /><PathNote label="Manifest" path={result.manifest_path} /></div>
    </div>}
  </div>;
}
