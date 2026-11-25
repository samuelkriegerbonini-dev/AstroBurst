import { useEffect, useId, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { readTextFile, writeTextFile } from "@tauri-apps/plugin-fs";
import { getWcsInfo } from "../../services/astrometry";
import { suggestCompanions } from "../../services/companions";
import { cubePrepFilters } from "../../services/cubeprep";
import { getFitsExtensions, getHeader } from "../../services/header";
import { imagePsfFwhm, resolutionCheck } from "../../services/resolution";
import { fileStore } from "../../hooks/useFileStore";
import { dockStore } from "../../hooks/useDockLayout";
import { showWhenLoaded } from "../../hooks/useFileIngest";
import { useResolutionSettings } from "../../hooks/useResolutionSettings";
import { CUBE_PREP_STEPS, type CubePrepFilters, type CubePrepInput, type CubePrepRecipe, type CubePrepReport, type CubePrepStepName } from "../../shared/types/cubeprep";
import { formatDecimalBytes } from "../../utils/format";
import { SHOW_IN_FOLDER } from "../../utils/outputNote";
import { revealInFolder } from "../../utils/revealInFolder";
import { fileBaseName } from "../../utils/values";
import { ErrorAlert, RunButton, WarningList } from "../ui";
import { addRecipeStep, afterInputChange, applyGoal, applyProfile, broadestBand, changeRecipeInput, commonReference, CUBE_PREP_LABELS, CUBE_PREP_PROFILE_NAMES, cubePrepError, cubePrepProblems, cubePrepProgressDetail, cubePrepStageText, cubePrepSummary, defaultStep, importedDraft, inputDirectory, joinProductPath, kernelProfileNote, MATCHED_FWHM_TOLERANCE, parseRecipe, profileDifferences, recipeForSave, removeRecipeStepAt, replaceRecipeStepAt, runFolderLabel, setCommonReference, setRecipeStep, type CubePrepDraft, type CubePrepProfile, type CubePrepSummary, type CubePrepTab } from "./cubePrepModel";
import { cubePrepStore, useCubePrepState } from "./cubePrepStore";
import { saguiStore } from "../sagui/saguiStore";
import CubePrepStepEditor, { PREP_BUTTON, PREP_FIELD, PrepField } from "./CubePrepStepEditor";
import { commonPsfArcsec, FITS_INPUT, FITS_INPUT_EXTENSIONS, fwhmCell, fwhmChange, inspectionKeyOf, layersText, psfFwhmRows, psfLine, REDSHIFT_HELP, redshiftNote, redshiftRequest, scaleLine, signedPercent, type RedshiftNote } from "./cubePrepInspect";

const JSON_FILTER = [{ name: "Cube Prep recipe", extensions: ["json"] }];
const PRIMARY_BUTTON = "rounded border border-violet-500/70 bg-violet-500/15 px-2 py-1 text-[11px] text-violet-100 hover:bg-violet-500/25 disabled:opacity-40";
const REFERENCE_TABS = new Set<CubePrepStepName>(["align", "cube", "psfs", "kernels", "match"]);
const ABSOLUTE_PATH = /^(?:[A-Za-z]:[\\/]|[\\/])/;
const OTHER_FILTER = "__other";
const STEP_MARK = { done: "✓", current: "▸", pending: "○" } as const;
const STEP_WORD = { done: "done", current: "running", pending: "pending" } as const;

interface PsfMeasurement { line: string | null; warnings: string[]; error: string | null; fwhmArcsec: number | null }
interface PsfRun { key: string; values: Record<string, PsfMeasurement>; done: number; total: number; running: boolean }

function droppedHandoff(current: CubePrepReport | null, sent: CubePrepReport): string {
  if (current === sent) return "SAGUI changed while the cube was being checked; send again.";
  return current ? "Recipe changed; send again." : "Recipe changed; run it again before sending to SAGUI.";
}

function skippedFiles(count: number): string {
  if (count === 0) return "";
  return count === 1 ? " 1 other opened FITS file was skipped." : ` ${count} other opened FITS files were skipped.`;
}

function sameText(a: string, b: string): boolean {
  return a.trim().toLowerCase() === b.trim().toLowerCase();
}

function hasRelativePaths(recipe: CubePrepRecipe): boolean {
  const relative = (path: unknown) => typeof path === "string" && path.trim() !== "" && !ABSOLUTE_PATH.test(path.trim());
  return recipe.inputs.some((input) => relative(input.path)) || recipe.steps.some((step) => relative(step.region_file) || relative(step.data_dir) || (Array.isArray(step.files) && step.files.some((file: unknown) => file !== null && typeof file === "object" && relative((file as { path?: unknown }).path))));
}

function runStepNumber(stage: string, count: number): number {
  const folder = /^(\d+)_/.exec(stage);
  if (folder) return Number(folder[1]);
  return stage === "hashing_products" || stage === "publishing" ? count + 1 : 0;
}

function bandsText(summary: CubePrepSummary): string {
  const bands = summary.bands.map((band) => band.pivotUm === null ? band.filter : `${band.filter} (${band.pivotUm.toPrecision(4)} µm)`).join(", ");
  return `${summary.bands.length} ${summary.bands.length === 1 ? "band" : "bands"}: ${bands}`;
}

function sizeText(summary: CubePrepSummary): string {
  const layout = summary.layout === "mef" ? "MEF" : summary.layout === "lightstack" ? "LIGHTSTACK (SCI only)" : summary.layout;
  return [summary.cols !== null && summary.rows !== null ? `${summary.cols} × ${summary.rows} pixels` : null, layout, summary.hdus.length > 0 ? summary.hdus.join(", ") : null].filter(Boolean).join(" · ");
}

export function PathNote({ label, path }: { label: string; path: string }) {
  const [failure, setFailure] = useState<{ path: string; message: string | null } | null>(null);
  const message = failure?.path === path ? failure.message : null;
  const name = fileBaseName(path) || path;
  return <div className="space-y-0.5 text-[10px]">
    <div className="flex items-center gap-2"><span className="shrink-0 text-zinc-500">{label}</span><span className="min-w-0 flex-1 truncate font-mono text-zinc-300" title={path}>{name}</span><button type="button" className={`${PREP_BUTTON} shrink-0`} aria-label={`${SHOW_IN_FOLDER}: ${name}`} onClick={() => void revealInFolder(path).then((next) => setFailure({ path, message: next }))}>{SHOW_IN_FOLDER}</button></div>
    {message && <p role="alert" className="break-words text-red-400">{message}</p>}
  </div>;
}

export default function CubePrepPanel() {
  const state = useCubePrepState();
  const { draft, validation, report } = state;
  const { recipe } = draft;
  const [tab, setTab] = useState<CubePrepTab>("inputs");
  const [catalog, setCatalog] = useState<CubePrepFilters>({ datasets: [], filters: [] });
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogAttempt, setCatalogAttempt] = useState(0);
  const [localError, setLocalError] = useState<string | null>(null);
  const [inputNotice, setInputNotice] = useState<string | null>(null);
  const [inspection, setInspection] = useState<{ key: string; values: Record<string, string[]> } | null>(null);
  const [typedFilters, setTypedFilters] = useState<Record<number, boolean>>({});
  const inspectionRequest = useRef(0);
  const handoffRequest = useRef(0);
  const clearedFilters = useRef<Record<number, string>>({});
  const suggestedFilters = useRef<Record<string, string>>({});
  const inspectionKey = inspectionKeyOf(draft);
  const [inspecting, setInspecting] = useState(false);
  const [psfRun, setPsfRun] = useState<PsfRun | null>(null);
  const psfRequest = useRef(0);
  const settings = useResolutionSettings();
  const [redshiftAnswer, setRedshiftAnswer] = useState<{ key: string; note: RedshiftNote } | null>(null);
  const [sending, setSending] = useState(false);
  const [json, setJson] = useState<string | null>(null);
  const busy = ["running", "cancelling", "validating"].includes(state.status);
  const cancellable = busy && cubePrepStore.isJobActive();
  const cancelNote = useId();
  const problemsId = useId();
  const [now, setNow] = useState(() => Date.now());
  const ticking = busy && state.startedAt !== null;
  const elapsedMs = state.startedAt === null ? null : Math.max(0, now - state.startedAt);
  const problems = cubePrepProblems(draft);
  const validateBlocked = cubePrepProblems(draft, false).length > 0;
  const attention = new Set(problems.map((problem) => problem.tab));
  const differences = profileDifferences(recipe, draft.profile);
  const kernelNote = kernelProfileNote(recipe, draft.profile);
  const common = commonReference(recipe);
  const namedInputs = recipe.inputs.filter((input) => input.filter.trim());
  const fwhmRows = report ? psfFwhmRows(report.steps) : [];
  const summary = report ? cubePrepSummary(report) : null;
  const headerIndex = recipe.steps.findIndex((entry) => entry.step === "edit" && entry.action === "header");
  const headerRedshift = headerIndex >= 0 && typeof recipe.steps[headerIndex].redshift === "number" ? recipe.steps[headerIndex].redshift as number : null;
  const redshiftAsk = redshiftRequest(headerRedshift, settings, report ? commonPsfArcsec(report.steps) : null);
  const redshiftKey = redshiftAsk ? JSON.stringify(redshiftAsk) : null;
  const redshiftAskRef = useRef(redshiftAsk);
  redshiftAskRef.current = redshiftAsk;
  const measured = psfRun !== null && !psfRun.running && psfRun.key === inspectionKey ? Object.fromEntries(recipe.inputs.map((input) => [input.path, psfRun.values[input.path]?.fwhmArcsec ?? null])) : {};
  const measuredBand = broadestBand(recipe.inputs, [], measured);
  const instances = tab === "inputs" ? [] : recipe.steps.flatMap((entry, index) => entry.step === tab ? [index] : []);
  const absentSteps = CUBE_PREP_STEPS.filter((name) => !recipe.steps.some((entry) => entry.step === name));
  const runSteps = (validation?.recipe ?? recipe).steps;
  const runStep = state.progress ? runStepNumber(state.progress.stage, runSteps.length) : 0;

  useEffect(() => {
    let current = true;
    cubePrepFilters().then((value) => { if (current) setCatalog(value); }).catch((error) => { if (current) setCatalogError(cubePrepError(error)); });
    return () => { current = false; };
  }, [catalogAttempt]);

  useEffect(() => {
    if (!ticking) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [ticking]);

  useEffect(() => () => { ++inspectionRequest.current; ++handoffRequest.current; ++psfRequest.current; }, []);

  useEffect(() => {
    const asked = redshiftAskRef.current;
    if (asked === null || redshiftKey === null) return;
    let current = true;
    resolutionCheck(asked).then((check) => { if (current) setRedshiftAnswer({ key: redshiftKey, note: redshiftNote(check) }); }, () => undefined);
    return () => { current = false; };
  }, [redshiftKey]);

  const edit = (change: Partial<CubePrepDraft>) => { setLocalError(null); setInputNotice(null); cubePrepStore.edit({ ...draft, ...change }); };
  const editInputs = (next: CubePrepRecipe) => edit({ recipe: afterInputChange(next, catalog.filters, measured) });
  const perform = async (operation: () => Promise<void>) => {
    setLocalError(null);
    try { await operation(); } catch (error) { setLocalError(cubePrepError(error)); }
  };
  const sendToSagui = () => void perform(async () => {
    if (!report) return;
    const request = ++handoffRequest.current;
    setSending(true);
    try {
      const accepted = await saguiStore.prepareCubePrep(report, () => request === handoffRequest.current && cubePrepStore.get().report === report);
      if (accepted) dockStore.dispatch({ type: "open", tool: "sagui" });
      else if (request === handoffRequest.current) setLocalError(droppedHandoff(cubePrepStore.get().report, report));
    } finally { if (request === handoffRequest.current) setSending(false); }
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
      if (match) suggestedFilters.current[path] = match.filter;
      return { path, dataset: match?.dataset ?? current.recipe.inputs[0]?.dataset ?? "", filter: match?.filter ?? "" };
    })];
    cubePrepStore.edit({ ...current, recipe: afterInputChange({ ...current.recipe, inputs }, catalog.filters, measured), baseDir: current.baseDir || inputDirectory(paths[0] ?? ""), outputParent: current.outputParent || inputDirectory(paths[0] ?? "") });
  };
  const addSelectedImage = () => {
    setLocalError(null);
    const selected = fileStore.getSelectedFile();
    if (!selected || !FITS_INPUT.test(selected.path)) {
      setInputNotice(null);
      setLocalError("Select a FITS image in Files to add it.");
      return;
    }
    const present = (path: string) => cubePrepStore.get().draft.recipe.inputs.some((input) => input.path === path);
    const already = present(selected.path);
    const skipped = fileStore.getFiles().filter((file) => file.id !== selected.id && FITS_INPUT.test(file.path) && !present(file.path)).length;
    if (!already) addPaths([selected.path]);
    setInputNotice(`${already ? "The selected image is already an input." : "Added the selected image."}${skippedFiles(skipped)}`);
  };
  const changeInput = (index: number, change: Partial<CubePrepInput>) => {
    const current = recipe.inputs[index]?.filter ?? "";
    let renameFrom: string | undefined;
    if (change.filter !== undefined && !change.filter.trim() && current.trim()) clearedFilters.current[index] = current;
    else if (change.filter !== undefined && change.filter.trim() && !current.trim() && clearedFilters.current[index] !== undefined) {
      renameFrom = clearedFilters.current[index];
      delete clearedFilters.current[index];
    }
    editInputs(changeRecipeInput(recipe, index, change, renameFrom));
  };
  const removeInput = (index: number) => {
    clearedFilters.current = {};
    setTypedFilters({});
    editInputs({ ...recipe, inputs: recipe.inputs.filter((_, i) => i !== index) });
  };
  const chooseGoal = (profile: CubePrepProfile) => edit({ profile, recipe: applyGoal(recipe, profile, catalog.filters) });
  const setRedshift = (text: string) => {
    const redshift = text.trim() === "" ? null : Number(text);
    if (headerIndex >= 0) edit({ recipe: replaceRecipeStepAt(recipe, headerIndex, { ...recipe.steps[headerIndex], redshift }) });
    else if (redshift !== null) edit({ recipe: addRecipeStep(recipe, { step: "edit", action: "header", redshift, ra_center: null, dec_center: null, use_brightest_pixel: false }) });
  };
  const importRecipe = () => void perform(async () => {
    const path = await open({ multiple: false, filters: JSON_FILTER });
    if (typeof path !== "string") return;
    cubePrepStore.edit(importedDraft(cubePrepStore.get().draft, path, await readTextFile(path)));
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
      const results = await Promise.all(recipe.inputs.map(async (input): Promise<[string, string[]]> => {
        const path = joinProductPath(draft.baseDir, input.path);
        try {
          const [header, extensions, layers, wcs] = await Promise.all([getHeader(path), getFitsExtensions(path), suggestCompanions(path).catch(() => null), getWcsInfo(path).catch(() => null)]);
          return [input.path, [layersText(path, header.BUNIT, extensions.extensions, layers), scaleLine(wcs?.pixel_scale_arcsec)]];
        } catch (error) { return [input.path, [cubePrepError(error)]]; }
      }));
      if (request === inspectionRequest.current) setInspection({ key: inspectionKey, values: Object.fromEntries(results) });
    } finally { if (request === inspectionRequest.current) setInspecting(false); }
  });
  const measurePsfs = () => void perform(async () => {
    const request = ++psfRequest.current;
    const key = inspectionKey;
    const { baseDir, recipe: { inputs } } = draft;
    const total = inputs.length;
    const values: Record<string, PsfMeasurement> = {};
    setPsfRun({ key, values: {}, done: 0, total, running: true });
    for (const [index, input] of inputs.entries()) {
      if (request !== psfRequest.current || inspectionKeyOf(cubePrepStore.get().draft) !== key) {
        if (request === psfRequest.current) setPsfRun((run) => run && { ...run, running: false });
        return;
      }
      try {
        const psf = await imagePsfFwhm(joinProductPath(baseDir, input.path));
        values[input.path] = { line: psfLine(psf), warnings: psf.warnings, error: null, fwhmArcsec: psf.fwhm_arcsec ?? null };
      } catch (error) { values[input.path] = { line: null, warnings: [], error: cubePrepError(error), fwhmArcsec: null }; }
      if (request === psfRequest.current) setPsfRun({ key, values: { ...values }, done: index + 1, total, running: index + 1 < total });
    }
  });
  const openFits = (path: string) => {
    if (!showWhenLoaded(path)) setLocalError("The file queue is unavailable. Open this FITS file from Files.");
  };
  const tabButton = (name: CubePrepTab, caption: string, key: string) => <button key={key} className={`${PREP_BUTTON} ${tab === name ? "border-cyan-500 text-cyan-300" : ""}`} aria-pressed={tab === name} onClick={() => setTab(name)}>{caption}{attention.has(name) && <><span aria-hidden="true" className="ml-1 inline-block h-1.5 w-1.5 rounded-full bg-amber-400 align-middle" /><span className="sr-only">, needs attention</span></>}</button>;
  const filterControl = (input: CubePrepInput, index: number) => {
    const options = catalog.filters.filter((entry) => sameText(entry.dataset, input.dataset));
    if (options.length === 0 || typedFilters[index]) return <input list={`cubeprep-filters-${index}`} className={PREP_FIELD} value={input.filter} onChange={(e) => changeInput(index, { filter: e.target.value })} />;
    const listed = options.find((entry) => sameText(entry.filter, input.filter));
    return <select className={PREP_FIELD} value={listed?.filter ?? input.filter} onChange={(e) => { if (e.target.value === OTHER_FILTER) setTypedFilters((typed) => ({ ...typed, [index]: true })); else changeInput(index, { filter: e.target.value }); }}>
      {!input.filter.trim() && <option value="">Choose a filter</option>}
      {input.filter.trim() && !listed && <option value={input.filter}>{`${input.filter} (not in the registry)`}</option>}
      {options.map((entry) => <option key={entry.filter} value={entry.filter}>{`${entry.filter} (${(entry.pivot_angstrom / 10000).toPrecision(4)} µm)`}</option>)}
      <option value={OTHER_FILTER}>Other filter…</option>
    </select>;
  };

  return <div className="h-full overflow-auto p-3 text-xs text-zinc-300 space-y-3">
    <div><h2 className="font-semibold text-cyan-300">Cube Prep</h2><p className="mt-1 text-[10px] text-zinc-500">Prepare registered, calibrated multiband images with the LIGHTSTACK recipe engine.</p></div>
    <fieldset disabled={busy} className="space-y-3 disabled:opacity-60">
      <div className="flex items-end gap-2">
        <div className="min-w-0 flex-1"><PrepField label="Cube for"><select className={PREP_FIELD} value={draft.profile} onChange={(e) => chooseGoal(e.target.value as CubePrepProfile)}><option value="scientific">Regional SEDs with SAGUI (Scientific MEF: ERR and DQ kept, PSFs matched)</option><option value="lightstack">Comparison with LIGHTSTACK 0.4.0 (SCI only)</option></select></PrepField></div>
        <details className="relative shrink-0"><summary className={`${PREP_BUTTON} cursor-pointer list-none`}>Recipe</summary><div className="absolute right-0 z-10 mt-1 flex w-36 flex-col gap-1 rounded border border-zinc-700 bg-zinc-900 p-1 shadow-lg"><button className={PREP_BUTTON} onClick={importRecipe}>Import recipe</button><button className={PREP_BUTTON} onClick={saveRecipe}>Save recipe</button><button className={PREP_BUTTON} onClick={() => setJson(json === null ? JSON.stringify(recipe, null, 2) : null)}>Recipe editor</button></div></details>
      </div>
      <p className="text-[10px] text-amber-300/80">{draft.profile === "lightstack" ? "LIGHTSTACK defaults are explicit. Its cube layout stores SCI only; choose MEF to retain available ERR and DQ." : "MEF retains available SCI, ERR and DQ. This does not create missing uncertainties or certify PSF matching; inspect validation and diagnostics before SAGUI."}</p>
      {differences.length > 0 && <div data-profile-differences className="space-y-1 text-[10px] text-amber-300"><p>{`This recipe differs from the ${CUBE_PREP_PROFILE_NAMES[draft.profile]} defaults: ${differences.join("; ")}.`}</p><button className={PREP_BUTTON} onClick={() => edit({ recipe: applyProfile(recipe, draft.profile) })}>Apply these defaults</button></div>}
      {kernelNote && <p className="text-[10px] text-amber-300/80">{kernelNote}</p>}
      <PrepField label="Recipe name"><input className={PREP_FIELD} value={recipe.name ?? ""} onChange={(e) => edit({ recipe: { ...recipe, name: e.target.value } })} /></PrepField>
      {json !== null && <div className="space-y-2"><PrepField label="Recipe JSON"><textarea rows={15} className={`${PREP_FIELD} font-mono`} value={json} onChange={(e) => setJson(e.target.value)} /></PrepField><button className={PREP_BUTTON} onClick={() => { try { edit({ recipe: parseRecipe(json) }); setJson(null); } catch (error) { setLocalError(cubePrepError(error)); } }}>Apply recipe</button><p className="text-[10px] text-zinc-500">Supports repeated steps and custom ordering. Apply edits before validation or running.</p></div>}
      <nav aria-label="Preparation steps" className="flex flex-wrap items-center gap-1">
        {tabButton("inputs", "Inputs", "inputs")}
        {recipe.steps.map((entry, index) => tabButton(entry.step, `${index + 1}. ${CUBE_PREP_LABELS[entry.step]}`, `step-${index}`))}
        {absentSteps.length > 0 && <><span className="ml-1 text-[10px] text-zinc-500">Add a step:</span>{absentSteps.map((name) => <button key={name} className={`${PREP_BUTTON} border-dashed text-zinc-400`} aria-label={`Add the ${CUBE_PREP_LABELS[name]} step`} onClick={() => { edit({ recipe: setRecipeStep(recipe, defaultStep(name, recipe, draft.profile, catalog.filters)) }); setTab(name); }}>{`+ ${CUBE_PREP_LABELS[name]}`}</button>)}</>}
      </nav>
      {tab === "inputs" ? <div className="space-y-3">
        <div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={() => void perform(async () => { setInputNotice(null); const paths = await open({ multiple: true, filters: [{ name: "FITS images", extensions: FITS_INPUT_EXTENSIONS }] }); if (paths) addPaths(typeof paths === "string" ? [paths] : paths); })}>Add FITS images</button><button className={PREP_BUTTON} onClick={addSelectedImage}>Use selected image</button><button className={PREP_BUTTON} disabled={inspecting || recipe.inputs.length === 0} onClick={inspectInputs}>{inspecting ? "Inspecting…" : "Inspect inputs"}</button><button className={PREP_BUTTON} disabled={Boolean(psfRun?.running) || recipe.inputs.length === 0} title="Measures the stars of each input, one image at a time; large images take a while" onClick={measurePsfs}>{psfRun?.running ? `Measuring PSF ${psfRun.done + 1} of ${psfRun.total}…` : "Measure PSFs"}</button></div>
        {inputNotice && <p role="status" className="text-[10px] text-zinc-400">{inputNotice}</p>}
        {catalogError && <div className="flex items-start gap-2"><div className="min-w-0 flex-1"><ErrorAlert message={`Could not load the filter registry: ${catalogError}`} /></div><button className={PREP_BUTTON} onClick={() => { setCatalogError(null); setCatalogAttempt((attempt) => attempt + 1); }}>Retry</button></div>}
        {recipe.inputs.length === 0 && <div data-empty-inputs className="rounded border border-dashed border-zinc-700 p-3"><ol className="list-decimal space-y-1 pl-4 text-[11px] text-zinc-400"><li>{"Add one image per filter with Add FITS images, or Use selected image for the file selected in Files."}</li><li>{"Check the dataset and filter of each image. The reference band defaults to the broadest PSF."}</li><li>{"Choose a PSF file per filter in PSFs, then Validate & run."}</li></ol></div>}
        {namedInputs.length > 0 && <div className="space-y-1"><PrepField label="Reference band (all steps)"><select className={PREP_FIELD} value={common === null ? "" : namedInputs.find((input) => sameText(input.filter, common))?.filter ?? common} onChange={(e) => edit({ recipe: setCommonReference(recipe, e.target.value) })}>{common === null && <option value="">Differs between steps</option>}{common === "" && <option value="">Choose a reference</option>}{namedInputs.map((input) => <option key={input.filter} value={input.filter}>{input.filter}</option>)}</select></PrepField><p className="text-[10px] text-zinc-500">Align, Cube, PSFs, Kernels and Match use this band. Choose the broadest PSF: the default is the longest pivot wavelength, or the broadest measured PSF after Measure PSFs.</p></div>}
        {measuredBand && common !== null && !sameText(measuredBand, common) && <div className="flex flex-wrap items-center gap-2 text-[10px] text-amber-300"><p>{`The measured PSF is broadest in ${measuredBand}.`}</p><button className={PREP_BUTTON} onClick={() => edit({ recipe: setCommonReference(recipe, measuredBand) })}>{`Use ${measuredBand} as reference`}</button></div>}
        {recipe.inputs.map((input, index) => { const measuredPsf = psfRun?.key === inspectionKey ? psfRun.values[input.path] : undefined; const suggested = suggestedFilters.current[input.path]; return <div key={`${index}-${input.path}`} className="rounded border border-zinc-700/60 p-2 space-y-2">
          <div className="flex items-center gap-2"><span className="min-w-0 flex-1 break-all text-[10px]" title={input.path}>{input.path}</span><button className={PREP_BUTTON} aria-label={`Remove input ${index + 1}`} onClick={() => removeInput(index)}>Remove</button></div>
          <div className="grid grid-cols-2 gap-2"><PrepField label={`Dataset ${index + 1}`}><input list="cubeprep-datasets" className={PREP_FIELD} value={input.dataset} onChange={(e) => changeInput(index, { dataset: e.target.value })} /></PrepField><PrepField label={`Filter ${index + 1}`}>{filterControl(input, index)}</PrepField></div>
          {suggested !== undefined && sameText(suggested, input.filter) && <p className="text-[10px] text-zinc-500"><span aria-hidden="true">✓ </span>Dataset and filter read from the file name</p>}
          <datalist id={`cubeprep-filters-${index}`}>{catalog.filters.filter((entry) => entry.dataset === input.dataset).map((entry) => <option key={entry.filter} value={entry.filter}>{entry.pivot_angstrom / 10000} µm</option>)}</datalist>
          {inspection?.key === inspectionKey && inspection.values[input.path]?.map((line) => <p key={line} className="text-[10px] text-zinc-400">{line}</p>)}
          {measuredPsf?.line && <><p className="text-[10px] text-zinc-400">{measuredPsf.line}</p><WarningList warnings={measuredPsf.warnings} /></>}
          {measuredPsf?.error && <p className="text-[10px] text-amber-300">{`PSF not measured: ${measuredPsf.error}`}</p>}
        </div>; })}
        <datalist id="cubeprep-datasets">{catalog.datasets.map((dataset) => <option key={dataset} value={dataset} />)}</datalist>
      </div> : <div className="space-y-3">
        {instances.length <= 1 ? <label className="flex items-center gap-2"><input type="checkbox" checked={instances.length === 1} onChange={(e) => edit({ recipe: e.target.checked ? setRecipeStep(recipe, defaultStep(tab, recipe, draft.profile, catalog.filters)) : removeRecipeStepAt(recipe, instances[0]) })} />{`Enable ${CUBE_PREP_LABELS[tab]}`}</label>
          : <p className="text-[10px] text-zinc-400">{`${instances.length} ${CUBE_PREP_LABELS[tab]} steps run in recipe order. Remove the ones you do not need.`}</p>}
        {instances.length > 0 && REFERENCE_TABS.has(tab) && common && <p className="text-[10px] text-zinc-500">{`Reference band ${common}, set on Inputs. A different reference below applies to this step only.`}</p>}
        {instances.map((index, position) => <div key={index} className={instances.length > 1 ? "space-y-2 rounded border border-zinc-700/60 p-2" : ""}>
          {instances.length > 1 && <div className="flex items-center gap-2 text-[10px] text-zinc-400"><span className="min-w-0 flex-1">{`${CUBE_PREP_LABELS[tab]} ${position + 1} of ${instances.length} (step ${index + 1})`}</span><button className={PREP_BUTTON} aria-label={`Remove ${CUBE_PREP_LABELS[tab]} ${position + 1}`} onClick={() => edit({ recipe: removeRecipeStepAt(recipe, index) })}>Remove</button></div>}
          <CubePrepStepEditor step={recipe.steps[index]} recipe={recipe} profile={draft.profile} onChange={(next) => edit({ recipe: replaceRecipeStepAt(recipe, index, next, draft.profile) })} browse={browse} redshiftNote={index === headerIndex && redshiftAnswer?.key === redshiftKey ? redshiftAnswer.note : null} />
        </div>)}
        {tab === "edit" && instances.length > 0 && <button className={PREP_BUTTON} onClick={() => edit({ recipe: addRecipeStep(recipe, defaultStep("edit", recipe, draft.profile, catalog.filters)) })}>Add edit</button>}
        {tab === "cube" && instances.length > 0 && <div className="space-y-1 text-[10px] text-zinc-500"><PrepField label="Redshift (written to the cube header)"><input type="number" step="any" min={0} className={PREP_FIELD} value={headerRedshift ?? ""} onChange={(e) => setRedshift(e.target.value)} /></PrepField><p>{`${REDSHIFT_HELP} The value goes into an Edit step, Set centre / redshift, after the cube.`}</p></div>}
      </div>}
      {(!draft.baseDir.trim() || hasRelativePaths(recipe)) && <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Input folder (relative recipe paths)"><input className={PREP_FIELD} value={draft.baseDir} onChange={(e) => edit({ baseDir: e.target.value })} /></PrepField></div><button className={PREP_BUTTON} aria-label="Browse input folder" onClick={() => browse(true, (baseDir) => edit({ baseDir }))}>Browse</button></div>}
      <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Save runs in"><input className={PREP_FIELD} value={draft.outputParent} onChange={(e) => edit({ outputParent: e.target.value })} /></PrepField></div><button className={PREP_BUTTON} aria-label="Browse working folder" onClick={() => browse(true, (outputParent) => edit({ outputParent }))}>Browse</button></div>
      <p className="text-[10px] text-zinc-500">{`Each run creates a folder cubeprep-${runFolderLabel(recipe.name ?? "")}-<date and time> here, with intermediate FITS files, recipe.json and result.json. Closing this panel keeps the active job running.`}</p>
    </fieldset>
    <ErrorAlert message={localError || state.error} />
    <div className="flex items-center gap-2"><div className="min-w-0 flex-1"><RunButton label={json !== null ? "Apply or close the recipe editor first" : problems.length > 0 ? problems[0].text : "Validate & run"} runningLabel={state.status === "validating" ? "Validating…" : "Running…"} running={cancellable} disabled={busy || json !== null || problems.length > 0} accent="cyan" small describedBy={problems.length > 0 ? problemsId : undefined} onClick={() => void cubePrepStore.run()} /></div><button className={PREP_BUTTON} disabled={busy || json !== null || validateBlocked} onClick={() => void cubePrepStore.validate()}>Validate</button>{cancellable && <button className={PREP_BUTTON} aria-describedby={cancelNote} disabled={state.status === "cancelling"} onClick={() => void cubePrepStore.cancel()}>{state.status === "cancelling" ? "Cancelling…" : "Cancel"}</button>}</div>
    {problems.length > 0 && <ul id={problemsId} className="list-disc pl-4 space-y-1 text-[10px] text-zinc-400">{problems.map((problem) => <li key={problem.text}>{problem.tab === "folders" ? problem.text : <button type="button" className="text-left underline-offset-2 hover:text-zinc-200 hover:underline" onClick={() => setTab(problem.tab as CubePrepTab)}>{problem.text}</button>}</li>)}</ul>}
    {cancellable && <p id={cancelNote} className="text-[10px] text-zinc-500">Stops after the current step; partial products stay in the run folder.</p>}
    <div aria-live="polite" className="space-y-2">
      {state.status === "validating" && <p>Validating files and recipe…</p>}
      {state.progress && <p>{cubePrepStageText(state.progress)}</p>}
      {state.status === "cancelled" && <p className="text-amber-300">{state.outputDir ? "Cancelled. Any partial products remain in the run folder." : "Cancelled before the run started; no run folder was created."}</p>}
      {state.status === "complete" && <p className="text-cyan-300">Run completed. Inspect the diagnostics and warnings below.</p>}
    </div>
    {state.progress && <div className="space-y-1"><progress aria-label="Cube preparation progress" className="w-full" value={state.progress.percent} max={100} /><p className="text-[10px] text-zinc-500">{cubePrepProgressDetail(state.progress, elapsedMs)}</p>
      <ol data-run-steps className="space-y-0.5 text-[10px]">{runSteps.map((entry, index) => { const phase = index + 1 < runStep ? "done" : index + 1 === runStep ? "current" : "pending"; return <li key={index} data-step-state={phase} className={phase === "current" ? "text-cyan-300" : phase === "done" ? "text-zinc-400" : "text-zinc-500"}><span aria-hidden="true">{`${STEP_MARK[phase]} `}</span>{`${index + 1}. ${CUBE_PREP_LABELS[entry.step]}`}<span className="sr-only">{`, ${STEP_WORD[phase]}`}</span></li>; })}</ol>
    </div>}
    {validation && <div className="space-y-2"><WarningList warnings={validation.warnings} /><details><summary className="cursor-pointer">{`Validated recipe · ${validation.dependencies.length} ${validation.dependencies.length === 1 ? "file" : "files"} checked`}</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify(validation, null, 2)}</pre></details></div>}
    {state.outputDir && <PathNote label="Run folder" path={state.outputDir} />}
    {report && <div className="space-y-3">
      {summary && <section data-cube-summary className="space-y-1 rounded border border-zinc-700/60 p-2 text-[11px]"><h3 className="font-semibold text-zinc-200">Final cube</h3><p>{bandsText(summary)}</p>{sizeText(summary) && <p className="text-zinc-400">{sizeText(summary)}</p>}<p className={summary.psfTarget ? "text-cyan-300" : "text-amber-300"}>{summary.psfNote}</p></section>}
      {report.cube && <PathNote label="Final cube" path={joinProductPath(report.output_dir, report.cube)} />}
      {report.cube && <div className="flex flex-wrap gap-1"><button className={PRIMARY_BUTTON} disabled={sending || busy} onClick={sendToSagui}>{sending ? "Checking cube…" : "Send to SAGUI"}</button><button className={PREP_BUTTON} onClick={() => openFits(joinProductPath(report.output_dir, report.cube!))}>Open final cube</button></div>}
      <WarningList warnings={report.warnings} />
      {fwhmRows.length > 0 && <table data-psf-fwhm className="w-full text-left text-[10px]"><caption className="text-left text-zinc-400">PSF FWHM by filter: source, target and after matching</caption><thead><tr><th scope="col">Filter</th><th scope="col">Source</th><th scope="col">Target</th><th scope="col">Matched</th><th scope="col">vs target</th></tr></thead><tbody>{fwhmRows.map((row, index) => { const change = row.reference ? null : fwhmChange(row); const outside = change !== null && Math.abs(change) > MATCHED_FWHM_TOLERANCE; return <tr key={`${index}-${row.filter}`} data-outside-tolerance={outside} className={outside ? "text-amber-300" : undefined}><th scope="row" className="font-mono">{row.filter}</th><td>{fwhmCell(row.sourcePx, row.scaleArcsec)}</td><td>{row.reference ? "reference" : fwhmCell(row.targetPx, row.scaleArcsec)}</td><td>{row.reference ? "--" : fwhmCell(row.matchedPx, row.scaleArcsec)}</td><td>{change === null ? "--" : signedPercent(change)}</td></tr>; })}</tbody></table>}
      <details data-diagnostics className="rounded border border-zinc-700/60 p-2"><summary className="cursor-pointer">{`Diagnostics of ${report.steps.length} ${report.steps.length === 1 ? "step" : "steps"}`}</summary><div className="mt-2 space-y-2">{report.steps.map((result) => <details key={result.index} className="rounded border border-zinc-700/60 p-2"><summary className="cursor-pointer">{`${result.index}. ${result.step} · ${(result.elapsed_ms / 1000).toFixed(1)} s`}</summary><WarningList warnings={result.warnings} /><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify({ parameters: result.parameters, diagnostics: result.report }, null, 2)}</pre></details>)}</div></details>
      <details><summary className="cursor-pointer">Products and provenance ({report.files.length})</summary><div className="mt-2 space-y-2">{report.files.map((file) => <div key={file.path} className="text-[10px]"><div className="flex items-start gap-1"><span className="min-w-0 flex-1 break-all">{`${file.path} · ${formatDecimalBytes(file.bytes)}`}</span>{FITS_INPUT.test(file.path) && <button className={PREP_BUTTON} onClick={() => openFits(joinProductPath(report.output_dir, file.path))}>Open FITS</button>}</div>{file.sha256 && <div className="break-all text-zinc-500">SHA-256: {file.sha256}</div>}</div>)}<pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify(report.provenance ?? {}, null, 2)}</pre></div></details>
    </div>}
  </div>;
}
