import { Fragment, useId, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { openUrl } from "@tauri-apps/plugin-opener";
import { dockStore } from "../../hooks/useDockLayout";
import { showWhenLoaded } from "../../hooks/useFileIngest";
import { fileStore } from "../../hooks/useFileStore";
import type { SaguiConfig, SaguiProgress, SaguiRegion, SaguiSedScaling } from "../../shared/types/sagui";
import { fileBaseName } from "../../utils/values";
import { ErrorAlert, RunButton, WarningList } from "../ui";
import { PREP_BUTTON, PREP_FIELD, PrepField } from "../cubeprep/CubePrepStepEditor";
import { cubePrepError, cubePrepPsfTarget, importedDraft, inputDirectory, joinProductPath, MATCHED_FWHM_TOLERANCE } from "../cubeprep/cubePrepModel";
import { FITS_INPUT_EXTENSIONS } from "../cubeprep/cubePrepInspect";
import { PathNote } from "../cubeprep/CubePrepPanel";
import { cubePrepStore } from "../cubeprep/cubePrepStore";
import { isRecipePath, saguiProblemList, saguiStore, useSaguiState, type SaguiSection, type SaguiState } from "./saguiStore";
import { targetBand } from "./saguiInputPsfModel";
import { DQ_PRESETS, dqAdvice, dqPlaneRows, psfTargetWarning, regionSedCsv, resultSummary, runtimeIssueLine, wardVerdict } from "./saguiPanelModel";
import SaguiInputPsf from "./SaguiInputPsf";
import SaguiViewer from "./SaguiViewer";

const R_INSTALLERS = "https://cran.r-project.org/bin/windows/base/old/4.6.2/";
const SAGUI_COMMIT = "56fd3ea776d7c00e056368498da9412bd58b84bb";
const FITS_CUBE_FILTER = [{ name: "FITS cube", extensions: FITS_INPUT_EXTENSIONS }];
const PRIMARY_BUTTON = "rounded border border-violet-500/70 bg-violet-500/15 px-2 py-1 text-[11px] text-violet-100 hover:bg-violet-500/25 disabled:opacity-40";
const EXTERNAL_TARGET = "__external";
const CUSTOM_DQ = "custom";
const SECTIONS: readonly SaguiSection[] = ["cube", "runtime", "regions", "run"];
const SECTION_TITLES: Record<SaguiSection, string> = { cube: "Cube", runtime: "R runtime", regions: "Regions", run: "Run" };
type PathKey = "input_path" | "output_dir" | "rscript_path" | "library_path";
type CheckKind = "inspect" | "runtime" | "prepared" | "cube";
type Phase = "done" | "current" | "pending";
const CHECK_TEXT: Record<CheckKind, string> = { inspect: "Checking cube and official runtime…", runtime: "Checking the R installation…", prepared: "Checking the prepared cube…", cube: "Checking the cube…" };

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

function sameFile(a: string, b: string): boolean {
  return a.trim().replace(/\\/g, "/").toLowerCase() === b.trim().replace(/\\/g, "/").toLowerCase();
}

function StepHeading({ id, number, section, phase }: { id: string; number: number; section: SaguiSection; phase: Phase }) {
  return <h3 id={id} className="flex items-center gap-1 font-semibold text-zinc-200">{`${number}. ${SECTION_TITLES[section]}`}{phase === "done" && <><span aria-hidden="true" className="ml-1 text-cyan-300">✓</span><span className="sr-only">, done</span></>}{phase === "current" && <><span className="sr-only">, </span><span className="ml-1 rounded bg-violet-500/20 px-1 text-[10px] font-normal text-violet-200">current step</span></>}</h3>;
}

export default function SaguiPanel() {
  const state = useSaguiState();
  const { config, input, runtime, result, progress } = state;
  const [localError, setLocalError] = useState<string | null>(null);
  const [scaleText, setScaleText] = useState(config.starlet_scales.join(", "));
  const [checkKind, setCheckKind] = useState<CheckKind | null>(null);
  const [externalTarget, setExternalTarget] = useState(false);
  const [customDq, setCustomDq] = useState(false);
  const [confirmReplace, setConfirmReplace] = useState(false);
  const problemsId = useId();
  const headingIds = { cube: useId(), runtime: useId(), regions: useId(), run: useId() };
  const busy = ["checking", "running", "installing", "cancelling"].includes(state.status);
  const segmenting = busy && state.operation === "segment";
  const problemList = saguiProblemList(config);
  const problems = problemList.map((problem) => problem.text);
  const prepReport = cubePrepStore.get().report;
  const prepCube = prepReport?.cube ? joinProductPath(prepReport.output_dir, prepReport.cube) : null;
  const sectionDone: Record<SaguiSection, boolean> = {
    cube: input !== null && !problemList.some((problem) => problem.section === "cube"),
    runtime: runtime?.ready === true && !problemList.some((problem) => problem.section === "runtime"),
    regions: !problemList.some((problem) => problem.section === "regions"),
    run: false,
  };
  const current = SECTIONS.find((section) => !sectionDone[section]) ?? "run";
  const phase = (section: SaguiSection): Phase => sectionDone[section] ? "done" : section === current ? "current" : "pending";
  const bandNames = input?.bands.map((band) => band.filter) ?? [];
  const chosenBand = input ? targetBand(input, config.psf_target) : null;
  const external = input !== null && (externalTarget || (config.psf_target.trim() !== "" && chosenBand === null));
  const targetWarning = input ? psfTargetWarning(input, config.psf_target) : null;
  const verifiedByCubePrep = input !== null && prepReport !== null && prepCube !== null && sameFile(config.input_path, prepCube) && config.psf_target.trim() !== "" && cubePrepPsfTarget(prepReport, bandNames) === config.psf_target;
  const dqPreset = !customDq && DQ_PRESETS.some((preset) => preset.bits === config.dq_exclude_bits) ? String(config.dq_exclude_bits) : CUSTOM_DQ;
  const advice = input ? dqAdvice(input.dq_flags, config.dq_exclude_bits) : null;
  const dqRows = input ? dqPlaneRows(input.dq_flags, config.dq_exclude_bits) : [];
  const ward = input ? wardVerdict(input, config.mode) : null;
  const installBlock = !config.rscript_path.trim() ? "Choose the Rscript executable first." : !state.installParent.trim() ? "Choose a library installation folder first." : null;
  const edit = (change: Partial<SaguiConfig>) => { setLocalError(null); saguiStore.edit({ ...config, ...change }); };
  const perform = async (operation: () => Promise<void>) => { setLocalError(null); try { await operation(); } catch (error) { setLocalError(cubePrepError(error)); } };
  const check = async (kind: CheckKind, action: () => Promise<unknown>) => { setCheckKind(kind); try { await action(); } finally { setCheckKind(null); } };
  const useCubePrep = () => void perform(async () => {
    const report = cubePrepStore.get().report;
    if (!report) throw new Error("No prepared cube is available. Run Cube Prep or choose a multiband FITS file.");
    await check("prepared", () => saguiStore.prepareCubePrep(report, () => cubePrepStore.get().report === report));
  });
  const chooseCube = (path: string | undefined | null) => {
    if (!path) { setLocalError("No prepared cube is available. Choose a multiband FITS file."); return; }
    if (prepCube && sameFile(path, prepCube)) { useCubePrep(); return; }
    const currentConfig = saguiStore.get().config;
    setExternalTarget(false);
    setLocalError(null);
    saguiStore.edit({ ...currentConfig, input_path: path, output_dir: currentConfig.output_dir || inputDirectory(path) });
    void perform(() => check("cube", () => saguiStore.inspectCube()));
  };
  const browse = (key: PathKey, directory: boolean) => void perform(async () => {
    const path = await open({ directory, multiple: false, ...(key === "input_path" ? { filters: FITS_CUBE_FILTER } : {}) });
    if (typeof path !== "string") return;
    if (key === "input_path") { chooseCube(path); return; }
    saguiStore.edit({ ...saguiStore.get().config, [key]: path });
  });
  const inspectTypedCube = () => { if (saguiStore.get().input === null) void perform(() => check("cube", () => saguiStore.inspectCube())); };
  const fileField = (key: PathKey, label: string, directory: boolean, name: string) => <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label={label}><input className={PREP_FIELD} value={config[key]} onChange={(e) => edit({ [key]: e.target.value })} {...(key === "input_path" ? { onBlur: inspectTypedCube } : {})} /></PrepField></div><button className={PREP_BUTTON} aria-label={`Browse ${name}`} onClick={() => browse(key, directory)}>Browse</button></div>;
  const draftInputs = () => cubePrepStore.get().draft?.recipe.inputs.length ?? 0;
  const openRecipeInCubePrep = (confirmed: boolean) => {
    if (!confirmed && draftInputs() > 0) { setConfirmReplace(true); return; }
    setConfirmReplace(false);
    void perform(async () => {
      if (cubePrepStore.isJobActive()) throw new Error("Wait for the active Cube Prep run before opening a recipe there.");
      const path = config.input_path.trim();
      cubePrepStore.edit(importedDraft(cubePrepStore.get().draft, path, await readTextFile(path)));
      edit({ input_path: "" });
      dockStore.dispatch({ type: "open", tool: "cubeprep" });
    });
  };
  const openPrevious = () => void perform(async () => {
    const path = await open({ title: "Open previous SAGUI result", multiple: false, filters: [{ name: "SAGUI manifest", extensions: ["json"] }] });
    if (typeof path !== "string") return;
    await saguiStore.restore(path);
    const restored = saguiStore.get();
    if (restored.status === "restored") setScaleText(restored.config.starlet_scales.join(", "));
  });
  const openFits = (path: string) => { if (!showWhenLoaded(path)) setLocalError("The file queue is unavailable. Open the exported FITS file from Files."); };
  const saveRegion = (region: SaguiRegion) => void perform(async () => {
    if (!result) return;
    const path = await save({ defaultPath: `sagui_region_${region.region}_sed.csv`, filters: [{ name: "CSV", extensions: ["csv"] }] });
    if (path) await writeTextFile(path, regionSedCsv(region, result.unit));
  });
  const copyCommit = () => void perform(async () => { await navigator.clipboard.writeText(SAGUI_COMMIT); });
  const number = (key: "n_regions" | "starlet_threshold" | "dq_exclude_bits" | "knn_k", label: string, min: number, max?: number) => <PrepField label={label}><input className={PREP_FIELD} type="number" min={min} max={max} step={key === "starlet_threshold" ? "any" : 1} value={Number.isNaN(config[key]) ? "" : config[key]} onChange={(e) => edit({ [key]: e.target.value === "" ? NaN : Number(e.target.value) })} /></PrepField>;

  return <div className="h-full overflow-auto p-3 text-xs text-zinc-300 space-y-3">
    <div><h2 className="font-semibold text-violet-300">SAGUI</h2><p className="mt-1 text-[10px] text-zinc-500">Group pixels by their multiband SED using the official SAGUI R implementation.</p></div>
    <fieldset disabled={busy} className="space-y-4 disabled:opacity-60">
      <section aria-labelledby={headingIds.cube} className="space-y-2">
        <StepHeading id={headingIds.cube} number={1} section="cube" phase={phase("cube")} />
        {fileField("input_path", "Prepared multiband FITS cube", false, "prepared multiband FITS cube")}
        {isRecipePath(config.input_path) && (confirmReplace
          ? <div className="space-y-1 rounded border border-amber-500/30 p-2 text-[10px] text-amber-300"><p>{`Cube Prep holds a draft with ${draftInputs()} ${draftInputs() === 1 ? "input" : "inputs"}; opening this recipe replaces it. Save the draft from Cube Prep first if you need it.`}</p><div className="flex gap-1"><button className={PREP_BUTTON} onClick={() => openRecipeInCubePrep(true)}>Replace the draft</button><button className={PREP_BUTTON} onClick={() => setConfirmReplace(false)}>Keep the draft</button></div></div>
          : <button className={PREP_BUTTON} onClick={() => openRecipeInCubePrep(false)}>Open this recipe in Cube Prep</button>)}
        <div className="flex flex-wrap gap-1"><button className={prepCube ? PRIMARY_BUTTON : PREP_BUTTON} onClick={useCubePrep}>Use Cube Prep result</button><button className={PREP_BUTTON} onClick={() => chooseCube(fileStore.getSelectedFile()?.path)}>Use selected FITS</button></div>
        {prepCube && <p className="text-[10px] text-zinc-500" title={prepCube}>{`Cube Prep result: ${fileBaseName(prepCube) || prepCube}`}</p>}
        {!config.input_path.trim() && !prepCube && <div data-empty-cube className="space-y-1 rounded border border-dashed border-zinc-700 p-3 text-[11px] text-zinc-400"><p>{"No cube yet. Prepare one in Cube Prep with Cube for set to Regional SEDs with SAGUI, then use its result here, or choose a multiband FITS cube."}</p><button className={PREP_BUTTON} onClick={() => dockStore.dispatch({ type: "open", tool: "cubeprep" })}>Prepare in Cube Prep</button></div>}
        <p className="text-[10px] text-zinc-500">Requires registered photometric bands, per-pixel flux units, celestial WCS and a verified common PSF. Prepare a MEF cube in Cube Prep to retain available ERR and DQ.</p>
        {input && <div className="space-y-2"><p>{input.width} × {input.height} · {input.bands.length} bands · {input.unit}</p><p className="text-[10px] text-zinc-400">ERR {input.has_err ? "present" : "absent"} · DQ {input.has_dq ? "present" : "absent"} · NOISCORR {input.has_noise_correlation ? "present" : "absent"} · Exact Ward memory estimate: {(input.estimated_memory_bytes / 1024 ** 3).toFixed(2)} GiB</p><p className="text-[10px] text-zinc-500">The estimate uses the full footprint before masks. Before R starts, only the cube and the sparse neighbor graph are checked against free memory; exact Ward checks its own memory in R on the clustered support and refuses more than 65536 pixels.</p><SaguiInputPsf input={input} psfTarget={config.psf_target} nRegions={config.n_regions} /><WarningList warnings={input.warnings} /></div>}
      </section>
      <section aria-labelledby={headingIds.runtime} className="space-y-2">
        <StepHeading id={headingIds.runtime} number={2} section="runtime" phase={phase("runtime")} />
        <details open={!runtime?.ready}><summary className="cursor-pointer text-zinc-200">{runtime?.ready ? `R ${runtime.r_version ?? ""} · SAGUI ${runtime.sagui_version ?? ""}: ready` : "R and SAGUI installation"}</summary>
          <ol className="mt-2 list-decimal space-y-3 pl-4">
            <li className="space-y-2"><p>Choose Rscript 4.6.2.</p>{fileField("rscript_path", "Rscript executable", false, "Rscript executable")}
              <p className="text-[10px] text-zinc-500">Required: R 4.6.2 (installs alongside other R versions; Windows installer in the <button type="button" title={R_INSTALLERS} className="text-violet-300 underline underline-offset-2 hover:text-violet-200" onClick={() => void perform(async () => { await openUrl(R_INSTALLERS); })}>CRAN archive</button>) and SAGUI commit <span title={SAGUI_COMMIT} className="font-mono">{SAGUI_COMMIT.slice(0, 7)}</span> <button type="button" className="text-violet-300 underline underline-offset-2 hover:text-violet-200" aria-label="Copy the full SAGUI commit hash" onClick={copyCommit}>Copy</button>. On Windows, install Rtools45 before building SAGUI.</p></li>
            <li className="space-y-2"><p>Choose the private SAGUI library, or install the pinned SAGUI into a new one.</p>{fileField("library_path", "Private R package library", true, "private R package library")}
              <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Library installation folder"><input className={PREP_FIELD} value={state.installParent} onChange={(e) => saguiStore.setInstallParent(e.target.value)} /></PrepField></div><button className={PREP_BUTTON} aria-label="Browse library installation folder" onClick={() => void perform(async () => { const path = await open({ directory: true, multiple: false }); if (typeof path === "string") saguiStore.setInstallParent(path); })}>Browse</button></div>
              <p className="text-[10px] text-zinc-500">Install or repair downloads the pinned SAGUI package and dependencies into a new private library. The library is selected after verification; existing installations stay available.</p>
              <div className="flex flex-wrap items-center gap-2"><button className={`${PREP_BUTTON} border-violet-700 text-violet-200`} disabled={installBlock !== null} title={installBlock ?? undefined} onClick={() => void saguiStore.install()}>Install / repair SAGUI library</button>{installBlock && <span className="text-[10px] text-zinc-500">{installBlock}</span>}</div></li>
            <li className="space-y-2"><p>Check the installation.</p><button className={PREP_BUTTON} onClick={() => void check("runtime", () => saguiStore.inspectRuntime())}>Check R installation</button>
              {runtime && <div className="space-y-2"><p className={runtime.ready ? "text-cyan-300" : "text-amber-300"}>{runtime.ready ? "Official runtime ready" : "Runtime unavailable or incompatible"}{runtime.r_version ? ` · R ${runtime.r_version}` : ""}{runtime.sagui_version ? ` · SAGUI ${runtime.sagui_version}` : ""}</p><WarningList warnings={runtime.issues.map(runtimeIssueLine)} />{runtime.sagui_commit && <p className="break-all text-[10px] text-zinc-500" title={runtime.sagui_commit}>SAGUI commit: {runtime.sagui_commit.slice(0, 7)}</p>}</div>}</li>
          </ol>
        </details>
      </section>
      <section aria-labelledby={headingIds.regions} className="space-y-2">
        <StepHeading id={headingIds.regions} number={3} section="regions" phase={phase("regions")} />
        <div className="grid grid-cols-2 gap-2">{number("n_regions", "Number of regions", 2)}<PrepField label="Clustering"><select className={PREP_FIELD} value={config.mode} onChange={(e) => edit({ mode: e.target.value as SaguiConfig["mode"] })}><option value="ward">Exact Ward</option><option value="sparse">Sparse Ward (approximation)</option></select></PrepField></div>
        {ward && <p data-ward-verdict={ward.tone} className={`text-[10px] ${ward.tone === "ok" ? "text-zinc-400" : "text-amber-300"}`}>{ward.text}</p>}
        {config.mode === "sparse" && <>{number("knn_k", "Nearest neighbours", 2, 4096)}<p className="text-[10px] text-amber-300">Sparse Ward uses a neighbour graph; it is not numerically equivalent to exact Ward.</p></>}
        {input ? <div className="space-y-1">
          <PrepField label="Verified common PSF target"><select className={PREP_FIELD} value={external ? EXTERNAL_TARGET : chosenBand?.filter ?? ""} onChange={(e) => { if (e.target.value === EXTERNAL_TARGET) { setExternalTarget(true); edit({ psf_target: "" }); } else { setExternalTarget(false); edit({ psf_target: e.target.value }); } }}>{!external && chosenBand === null && <option value="">Choose the band every plane was matched to</option>}{bandNames.map((filter) => <option key={filter} value={filter}>{filter}</option>)}<option value={EXTERNAL_TARGET}>External PSF, not a band of this cube</option></select></PrepField>
          {external && <PrepField label="External PSF all bands were matched to"><input className={PREP_FIELD} value={config.psf_target} onChange={(e) => edit({ psf_target: e.target.value })} /></PrepField>}
          {verifiedByCubePrep && <p className="text-[10px] text-cyan-300">{`Verified by Cube Prep: every matched band within ${Math.round(100 * MATCHED_FWHM_TOLERANCE)} % of the ${config.psf_target} FWHM.`}</p>}
          {targetWarning && <p className="text-[10px] text-amber-300">{targetWarning}</p>}
        </div> : <PrepField label="Verified common PSF target"><input className={PREP_FIELD} value={config.psf_target} placeholder="Check the cube to choose one of its bands" onChange={(e) => edit({ psf_target: e.target.value })} /></PrepField>}
        <label className="flex items-center gap-2"><input type="checkbox" checked={config.starlet} onChange={(e) => edit({ starlet: e.target.checked })} />Use starlet morphology mask</label>
        {config.starlet && <details className="rounded border border-zinc-700/60 p-2"><summary className="cursor-pointer text-zinc-300">Starlet settings (the defaults suit most cubes)</summary><div className="mt-2 grid grid-cols-2 gap-2"><PrepField label="Starlet scales (1 = finest, 1 to 10)"><input className={PREP_FIELD} value={scaleText} onChange={(e) => { setScaleText(e.target.value); const values = e.target.value.split(/[\s,;]+/).filter(Boolean).map(Number); edit({ starlet_scales: values }); }} /></PrepField>{number("starlet_threshold", "Starlet threshold (k, in noise σ of each scale)", 0)}</div><p className="mt-1 text-[10px] text-zinc-500">Scale j of the starlet transform holds structure about 2^j pixels across; the mask keeps coefficients above k times the noise of their scale.</p></details>}
        <PrepField label="DQ bits to exclude"><select className={PREP_FIELD} value={dqPreset} onChange={(e) => { if (e.target.value === CUSTOM_DQ) setCustomDq(true); else { setCustomDq(false); edit({ dq_exclude_bits: Number(e.target.value) }); } }}>{DQ_PRESETS.map((preset) => <option key={preset.bits} value={String(preset.bits)}>{preset.label}</option>)}<option value={CUSTOM_DQ}>Custom mask</option></select></PrepField>
        {dqPreset === CUSTOM_DQ && number("dq_exclude_bits", "Custom DQ mask (decimal)", 0, 4294967295)}
        <p className="text-[10px] text-zinc-500">The default 1 excludes DO_NOT_USE on JWST and Roman planes (REED_SOLOMON on HST planes). 4294967295 excludes all flagged bits; 0 ignores DQ flags. Non-finite samples remain excluded.</p>
        {advice && <div className="space-y-1 text-[10px] text-amber-300"><p>{advice.text}</p>{advice.recommended !== config.dq_exclude_bits && <button className={PREP_BUTTON} onClick={() => { setCustomDq(false); edit({ dq_exclude_bits: advice.recommended }); }}>{`Use ${advice.recommended}`}</button>}</div>}
        {dqRows.length > 0 && <div className="space-y-1"><table data-dq-flags className="w-full text-left text-[10px]"><thead><tr className="text-zinc-500"><th scope="col">Band</th><th scope="col">DQ table</th><th scope="col">Flagged finite pixels</th></tr></thead><tbody>{dqRows.map((row) => <tr key={row.filter}><th scope="row" className="font-mono font-normal">{row.filter}</th><td>{row.table}</td><td>{row.flags.length === 0 ? "none" : row.flags.map((flag, index) => <Fragment key={flag.name}>{index > 0 && ", "}<span className={flag.excluded ? "text-zinc-500" : "text-amber-300"}>{`${flag.name} ${flag.count.toLocaleString("en-US")}${flag.excluded ? " (excluded)" : ""}`}</span></Fragment>)}</td></tr>)}</tbody></table><p className="text-[10px] text-zinc-500">Counts are per flag and can overlap. Flags in amber stay in the clustering support and in the regional sums with this mask.</p></div>}
        <details><summary className="cursor-pointer text-zinc-200">Advanced clustering</summary><div className="mt-2 space-y-2">
          <PrepField label="SED scaling"><select className={PREP_FIELD} value={config.sed_scaling ?? "median_centre"} onChange={(e) => edit({ sed_scaling: e.target.value as SaguiSedScaling })}><option value="median_centre">Median-centered (SAGUI default)</option><option value="median_mad">Median-centered, divided by MAD</option></select></PrepField>
          <p className="text-[10px] text-zinc-500">Clustering compares pixel spectra after this scaling. The default subtracts each spectrum&apos;s median; the MAD option also divides by the median absolute deviation, falling back to the standard deviation and then 1 when it is zero. Changing it changes the regions.</p>
        </div></details>
      </section>
      <section aria-labelledby={headingIds.run} className="space-y-2">
        <StepHeading id={headingIds.run} number={4} section="run" phase={phase("run")} />
        {fileField("output_dir", "Save runs in", true, "working folder")}
        <p className="text-[10px] text-zinc-500">Each run creates a separate sagui folder with segmentation FITS, mask FITS, regional photometry CSV, manifest and R logs.</p>
      </section>
    </fieldset>
    <ErrorAlert message={localError || state.error} />
    <div className="flex flex-wrap items-center gap-2"><div className="min-w-36 flex-1"><RunButton label={problems.length > 0 ? problems[0] : "Validate & segment"} runningLabel={state.status === "checking" ? "Checking…" : "Segmenting…"} running={segmenting} disabled={busy || problems.length > 0} accent="violet" small describedBy={problems.length > 0 ? problemsId : undefined} onClick={() => void saguiStore.run()} /></div><button className={PREP_BUTTON} disabled={busy || !config.input_path} onClick={() => void check("inspect", () => saguiStore.inspect())}>Check cube and runtime</button>{busy && saguiStore.canCancel() && <button className={PREP_BUTTON} disabled={state.status === "cancelling"} onClick={() => void saguiStore.cancel()}>{state.status === "cancelling" ? "Cancelling…" : "Cancel"}</button>}</div>
    {problems.length > 0 && <ul id={problemsId} className="list-disc pl-4 text-[10px] text-zinc-400 space-y-1">{problems.map((problem) => <li key={problem}>{problem}</li>)}</ul>}
    <div aria-live="polite" className="space-y-2">{state.status === "checking" && <p>{checkingText(state, checkKind)}</p>}{state.status === "installing" && <p>Installing SAGUI and its dependencies…</p>}{progress && <p>{stageParts(progress.stage)[0]}</p>}{state.status === "cancelled" && <p className="text-amber-300">{cancelledText(state)}</p>}{state.status === "complete" && <p className="text-cyan-300">Segmentation completed by the official R runtime.</p>}{state.status === "restored" && <p className="text-cyan-300">Previous result opened and verified. Its input and analysis settings are shown above; your Rscript, library and working folder were kept.</p>}{state.status === "installed" && <p className="text-cyan-300">Private SAGUI library installed and verified.</p>}</div>
    {progress && <div className="space-y-1"><progress aria-label={state.operation === "install" ? "SAGUI installation progress" : "SAGUI segmentation progress"} className="w-full" value={progress.percent} max={100} /><p className="text-[10px] text-zinc-500">{progressDetail(progress)}</p></div>}
    {state.outputDir && <PathNote label={state.operation === "install" ? "Installation folder" : "Run folder"} path={state.outputDir} />}
    {state.operation === "install" && state.installation && <PathNote label="Installation manifest" path={state.installation.manifest_path} />}
    {state.installation && <WarningList warnings={state.installation.warnings} />}
    {result && <div className="space-y-3"><p data-result-summary className="font-semibold text-cyan-200">{resultSummary(result)}</p><WarningList warnings={result.warnings} /><SaguiViewer key={result.run_id} result={result} onSaveRegion={saveRegion} />
      <h4 className="font-semibold">Next</h4><div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={() => openFits(result.labels_path)}>Open segmentation FITS</button><button className={PREP_BUTTON} onClick={() => openFits(result.mask_path)}>Open mask FITS</button><button className={PREP_BUTTON} onClick={() => void perform(async () => { const path = await save({ defaultPath: "sagui_photometry.csv", filters: [{ name: "CSV", extensions: ["csv"] }] }); if (path) await writeTextFile(path, await readTextFile(result.table_path)); })}>Save photometry as…</button></div>
      <div className="space-y-1"><PathNote label="Segmentation FITS" path={result.labels_path} /><PathNote label="Mask FITS" path={result.mask_path} /><PathNote label="Photometry CSV" path={result.table_path} /><PathNote label="Manifest" path={result.manifest_path} /></div>
    </div>}
    <div className="border-t border-zinc-700/60 pt-2"><button className={PREP_BUTTON} disabled={busy} onClick={openPrevious}>Open previous SAGUI result</button></div>
  </div>;
}
