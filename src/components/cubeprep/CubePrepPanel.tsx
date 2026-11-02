import { useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { cubePrepFilters } from "../../services/cubeprep";
import { getFitsExtensions, getHeader } from "../../services/header";
import { fileStore } from "../../hooks/useFileStore";
import { dockStore } from "../../hooks/useDockLayout";
import { showWhenLoaded } from "../../hooks/useFileIngest";
import { CUBE_PREP_STEPS, type CubePrepFilters, type CubePrepInput, type CubePrepStepName } from "../../shared/types/cubeprep";
import { ErrorAlert, WarningList } from "../ui";
import { applyProfile, cubePrepError, defaultStep, inputDirectory, joinProductPath, parseRecipe, recipeForSave, recipeProblems, setRecipeStep, type CubePrepDraft, type CubePrepProfile } from "./cubePrepModel";
import { cubePrepStore, useCubePrepState } from "./cubePrepStore";
import { saguiStore } from "../sagui/saguiStore";
import CubePrepStepEditor, { PREP_BUTTON, PREP_FIELD, PrepField } from "./CubePrepStepEditor";

const LABELS: Record<CubePrepStepName, string> = { crop: "Crop", units: "Units", align: "Align", cube: "Cube", edit: "Edit", psfs: "PSFs", kernels: "Kernels", match: "Match", export: "Export" };
const FITS_FILTER = [{ name: "FITS images", extensions: ["fits", "fit", "fts", "gz"] }];
const JSON_FILTER = [{ name: "Cube Prep recipe", extensions: ["json"] }];

export default function CubePrepPanel() {
  const state = useCubePrepState();
  const { draft, validation, report } = state;
  const { recipe } = draft;
  const [tab, setTab] = useState<"inputs" | CubePrepStepName>("inputs");
  const [catalog, setCatalog] = useState<CubePrepFilters>({ datasets: [], filters: [] });
  const [localError, setLocalError] = useState<string | null>(null);
  const [inspection, setInspection] = useState<{ key: string; values: Record<string, string> } | null>(null);
  const inspectionRequest = useRef(0);
  const inspectionKey = JSON.stringify([draft.baseDir, recipe.inputs]);
  const [inspecting, setInspecting] = useState(false);
  const [sending, setSending] = useState(false);
  const [json, setJson] = useState<string | null>(null);
  const busy = ["running", "cancelling", "validating"].includes(state.status);
  const step = recipe.steps.find((entry) => entry.step === tab);
  const problems = recipeProblems(recipe);

  useEffect(() => {
    let current = true;
    cubePrepFilters().then((value) => { if (current) setCatalog(value); }).catch((error) => { if (current) setLocalError(cubePrepError(error)); });
    return () => { current = false; };
  }, []);

  useEffect(() => () => { ++inspectionRequest.current; }, []);

  const edit = (change: Partial<CubePrepDraft>) => { setLocalError(null); cubePrepStore.edit({ ...draft, ...change }); };
  const perform = async (operation: () => Promise<void>) => {
    setLocalError(null);
    try { await operation(); } catch (error) { setLocalError(cubePrepError(error)); }
  };
  const sendToSagui = () => void perform(async () => {
    if (!report) return;
    const request = ++inspectionRequest.current;
    setSending(true);
    try {
      const accepted = await saguiStore.prepareCubePrep(report, () => request === inspectionRequest.current && cubePrepStore.get().report === report);
      if (accepted) dockStore.dispatch({ type: "open", tool: "sagui" });
    } finally { setSending(false); }
  });
  const browse = (directory: boolean, selected: (path: string) => void) => {
    void perform(async () => {
      const path = await open({ directory, multiple: false });
      if (typeof path === "string") selected(path);
    });
  };
  const addPaths = (paths: string[]) => {
    const current = cubePrepStore.get().draft;
    const newPaths = paths.filter((path) => !current.recipe.inputs.some((input) => input.path === path));
    const inputs = [...current.recipe.inputs, ...newPaths.map((path): CubePrepInput => {
      const name = path.split(/[\\/]/).at(-1)?.toUpperCase() ?? "";
      const candidates = catalog.filters.filter((filter) => [filter.filter, ...filter.aliases].some((alias) => name.includes(alias.toUpperCase())));
      const match = candidates.length === 1 ? candidates[0] : null;
      return { path, dataset: match?.dataset ?? current.recipe.inputs[0]?.dataset ?? "", filter: match?.filter ?? "" };
    })];
    cubePrepStore.edit({ ...current, recipe: { ...current.recipe, inputs }, baseDir: current.baseDir || inputDirectory(paths[0] ?? ""), outputParent: current.outputParent || inputDirectory(paths[0] ?? "") });
  };
  const changeInput = (index: number, change: Partial<CubePrepInput>) => edit({ recipe: { ...recipe, inputs: recipe.inputs.map((input, i) => i === index ? { ...input, ...change } : input) } });
  const importRecipe = () => void perform(async () => {
    const path = await open({ multiple: false, filters: JSON_FILTER });
    if (typeof path !== "string") return;
    const imported = parseRecipe(await readTextFile(path));
    const current = cubePrepStore.get().draft;
    cubePrepStore.edit({ ...current, recipe: imported, baseDir: inputDirectory(path), outputParent: current.outputParent || inputDirectory(path) });
    setJson(null);
  });
  const saveRecipe = () => void perform(async () => {
    const path = await save({ defaultPath: joinProductPath(draft.baseDir || draft.outputParent || ".", `${recipe.name || "recipe"}.json`), filters: JSON_FILTER });
    if (path) await writeTextFile(path, JSON.stringify(validation?.recipe ?? recipeForSave(recipe, draft.baseDir), null, 2));
  });
  const inspectInputs = () => void perform(async () => {
    const request = ++inspectionRequest.current;
    setInspecting(true);
    try {
      const results = await Promise.all(recipe.inputs.map(async (input) => {
        const path = joinProductPath(draft.baseDir, input.path);
        try {
          const [header, extensions] = await Promise.all([getHeader(path), getFitsExtensions(path)]);
          const unit = header.BUNIT || "unit missing";
          const err = extensions.extensions.some((hdu) => hdu.is_err) ? "ERR present" : "ERR absent";
          const dq = extensions.extensions.some((hdu) => hdu.is_dq) ? "DQ present" : "DQ absent";
          return [input.path, `${unit} · ${err} · ${dq}`] as const;
        } catch (error) { return [input.path, cubePrepError(error)] as const; }
      }));
      if (request === inspectionRequest.current) setInspection({ key: inspectionKey, values: Object.fromEntries(results) });
    } finally { if (request === inspectionRequest.current) setInspecting(false); }
  });
  const openFits = (path: string) => {
    if (!showWhenLoaded(path)) setLocalError("The file queue is unavailable. Open this FITS file from Files.");
  };

  return <div className="h-full overflow-auto p-3 text-xs text-zinc-300 space-y-3">
    <div><h2 className="font-semibold text-cyan-300">Cube Prep</h2><p className="mt-1 text-[10px] text-zinc-500">Prepare registered, calibrated multiband images with the LIGHTSTACK recipe engine.</p></div>
    <fieldset disabled={busy} className="space-y-3 disabled:opacity-60">
      <div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={importRecipe}>Import recipe</button><button className={PREP_BUTTON} onClick={saveRecipe}>Save recipe</button><button className={PREP_BUTTON} onClick={() => setJson(json === null ? JSON.stringify(recipe, null, 2) : null)}>Recipe editor</button></div>
      <PrepField label="Recipe name"><input className={PREP_FIELD} value={recipe.name ?? ""} onChange={(e) => edit({ recipe: { ...recipe, name: e.target.value } })} /></PrepField>
      <PrepField label="Apply defaults"><select className={PREP_FIELD} value={draft.profile} onChange={(e) => { const profile = e.target.value as CubePrepProfile; edit({ profile, recipe: applyProfile(recipe, profile) }); }}><option value="lightstack">LIGHTSTACK 0.4.0 compatibility</option><option value="scientific">Scientific MEF for regional SEDs / SAGUI</option></select></PrepField>
      <p className="text-[10px] text-amber-300/80">{draft.profile === "lightstack" ? "LIGHTSTACK defaults are explicit. Its cube layout stores SCI only; choose MEF to retain available ERR and DQ." : "MEF retains available SCI, ERR and DQ. This does not create missing uncertainties or certify PSF matching; inspect validation and diagnostics before SAGUI."}</p>
      {json !== null && <div className="space-y-2"><PrepField label="Recipe JSON"><textarea rows={15} className={`${PREP_FIELD} font-mono`} value={json} onChange={(e) => setJson(e.target.value)} /></PrepField><button className={PREP_BUTTON} onClick={() => { try { edit({ recipe: parseRecipe(json) }); setJson(null); } catch (error) { setLocalError(cubePrepError(error)); } }}>Apply recipe</button><p className="text-[10px] text-zinc-500">Supports repeated steps and custom ordering. Apply edits before validation or running.</p></div>}
      <nav aria-label="Preparation steps" className="flex flex-wrap gap-1">
        <button className={`${PREP_BUTTON} ${tab === "inputs" ? "border-cyan-500 text-cyan-300" : ""}`} aria-pressed={tab === "inputs"} onClick={() => setTab("inputs")}>Inputs</button>
        {CUBE_PREP_STEPS.map((name) => <button key={name} className={`${PREP_BUTTON} ${tab === name ? "border-cyan-500 text-cyan-300" : ""}`} aria-pressed={tab === name} onClick={() => setTab(name)}>{LABELS[name]}{recipe.steps.some((entry) => entry.step === name) ? " ✓" : ""}</button>)}
      </nav>
      {tab === "inputs" ? <div className="space-y-3">
        <div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={() => void perform(async () => { const paths = await open({ multiple: true, filters: FITS_FILTER }); if (paths) addPaths(typeof paths === "string" ? [paths] : paths); })}>Add FITS images</button><button className={PREP_BUTTON} onClick={() => addPaths(fileStore.getFiles().filter((file) => /\.(fits|fit|fts)(\.gz)?$/i.test(file.path)).map((file) => file.path))}>Use opened images</button><button className={PREP_BUTTON} disabled={inspecting || recipe.inputs.length === 0} onClick={inspectInputs}>{inspecting ? "Inspecting…" : "Inspect inputs"}</button></div>
        {recipe.inputs.length === 0 && <p className="text-[11px] text-zinc-500">Add one image per filter. Assign its dataset and filter before running.</p>}
        {recipe.inputs.map((input, index) => <div key={`${index}-${input.path}`} className="rounded border border-zinc-700/60 p-2 space-y-2">
          <div className="flex items-center gap-2"><span className="min-w-0 flex-1 break-all text-[10px]" title={input.path}>{input.path}</span><button className={PREP_BUTTON} aria-label={`Remove input ${index + 1}`} onClick={() => edit({ recipe: { ...recipe, inputs: recipe.inputs.filter((_, i) => i !== index) } })}>Remove</button></div>
          <div className="grid grid-cols-2 gap-2"><PrepField label={`Dataset ${index + 1}`}><input list="cubeprep-datasets" className={PREP_FIELD} value={input.dataset} onChange={(e) => changeInput(index, { dataset: e.target.value })} /></PrepField><PrepField label={`Filter ${index + 1}`}><input list={`cubeprep-filters-${index}`} className={PREP_FIELD} value={input.filter} onChange={(e) => changeInput(index, { filter: e.target.value })} /></PrepField></div>
          <datalist id={`cubeprep-filters-${index}`}>{catalog.filters.filter((entry) => entry.dataset === input.dataset).map((entry) => <option key={entry.filter} value={entry.filter}>{entry.pivot_angstrom / 10000} µm</option>)}</datalist>
          {inspection?.key === inspectionKey && inspection.values[input.path] && <p className="text-[10px] text-zinc-400">{inspection.values[input.path]}</p>}
        </div>)}
        <datalist id="cubeprep-datasets">{catalog.datasets.map((dataset) => <option key={dataset} value={dataset} />)}</datalist>
      </div> : <div className="space-y-3">
        <label className="flex items-center gap-2"><input type="checkbox" checked={Boolean(step)} onChange={(e) => edit({ recipe: e.target.checked ? setRecipeStep(recipe, defaultStep(tab, recipe, draft.profile)) : { ...recipe, steps: recipe.steps.filter((entry) => entry.step !== tab) } })} />Enable {LABELS[tab]}</label>
        {step && <CubePrepStepEditor step={step} recipe={recipe} onChange={(next) => { let updated = setRecipeStep(recipe, next); if (next.step === "units" && draft.profile === "scientific") updated = applyProfile(updated, draft.profile); edit({ recipe: updated }); }} browse={browse} />}
      </div>}
      <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Input folder (relative recipe paths)"><input className={PREP_FIELD} value={draft.baseDir} onChange={(e) => edit({ baseDir: e.target.value })} /></PrepField></div><button className={PREP_BUTTON} onClick={() => browse(true, (baseDir) => edit({ baseDir }))}>Browse</button></div>
      <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Working folder"><input className={PREP_FIELD} value={draft.outputParent} onChange={(e) => edit({ outputParent: e.target.value })} /></PrepField></div><button className={PREP_BUTTON} onClick={() => browse(true, (outputParent) => edit({ outputParent }))}>Browse</button></div>
      <p className="text-[10px] text-zinc-500">Each run creates a separate cubeprep folder here, with intermediate FITS files, recipe.json and result.json. Closing this panel keeps the active job running.</p>
    </fieldset>
    <ErrorAlert message={localError || state.error} />
    {problems.length > 0 && <ul className="list-disc pl-4 space-y-1 text-[10px] text-zinc-500">{problems.map((problem) => <li key={problem}>{problem}</li>)}</ul>}
    <div className="flex gap-2"><button className={PREP_BUTTON} disabled={busy || json !== null} onClick={() => void cubePrepStore.validate()}>Validate</button><button className={`${PREP_BUTTON} border-cyan-700 text-cyan-200`} disabled={busy || json !== null || problems.length > 0} onClick={() => void cubePrepStore.run()}>Validate &amp; run</button>{busy && cubePrepStore.isJobActive() && <button className={PREP_BUTTON} disabled={state.status === "cancelling"} onClick={() => void cubePrepStore.cancel()}>{state.status === "cancelling" ? "Cancelling…" : "Cancel"}</button>}</div>
    <div aria-live="polite" className="space-y-2">
      {state.status === "validating" && <p>Validating files and recipe…</p>}
      {state.progress && <><progress className="w-full" value={state.progress.percent} max={100} /><p>{state.progress.stage} · {state.progress.current}/{state.progress.total} · {state.progress.percent.toFixed(0)}%</p></>}
      {state.status === "cancelled" && <p className="text-amber-300">Cancelled. Any partial products remain in the run folder.</p>}
      {state.status === "complete" && <p className="text-cyan-300">Run completed. Inspect the diagnostics and warnings below.</p>}
    </div>
    {validation && <div className="space-y-2"><WarningList warnings={validation.warnings} /><details><summary className="cursor-pointer">Validated recipe · {validation.dependencies.length} input files</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify(validation, null, 2)}</pre></details></div>}
    {state.outputDir && <div className="space-y-1"><p className="break-all text-[10px]">Run folder: {state.outputDir}</p><button className={PREP_BUTTON} onClick={() => void perform(async () => { await revealItemInDir(state.outputDir!); })}>Show run folder</button></div>}
    {report && <div className="space-y-3">
      <WarningList warnings={report.warnings} />
      {report.cube && <div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={() => openFits(joinProductPath(report.output_dir, report.cube!))}>Open final cube</button><button className={PREP_BUTTON} disabled={sending || busy} onClick={sendToSagui}>{sending ? "Checking cube…" : "Send to SAGUI"}</button></div>}
      {report.steps.map((result) => <details key={result.index} className="rounded border border-zinc-700/60 p-2"><summary className="cursor-pointer">{result.index}. {result.step} · {(result.elapsed_ms / 1000).toFixed(1)} s</summary><WarningList warnings={result.warnings} /><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify({ parameters: result.parameters, diagnostics: result.report }, null, 2)}</pre></details>)}
      <details><summary className="cursor-pointer">Products and provenance ({report.files.length})</summary><div className="mt-2 space-y-2">{report.files.map((file) => <div key={file.path} className="text-[10px]"><div className="flex items-start gap-1"><span className="min-w-0 flex-1 break-all">{file.path} · {file.bytes.toLocaleString()} bytes</span>{/\.(fits|fit|fts)(\.gz)?$/i.test(file.path) && <button className={PREP_BUTTON} onClick={() => openFits(joinProductPath(report.output_dir, file.path))}>Open FITS</button>}</div>{file.sha256 && <div className="break-all text-zinc-500">SHA-256: {file.sha256}</div>}</div>)}<pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify(report.provenance ?? {}, null, 2)}</pre></div></details>
    </div>}
  </div>;
}
