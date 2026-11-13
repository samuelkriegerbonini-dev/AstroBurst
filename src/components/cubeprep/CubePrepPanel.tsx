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
import { CUBE_PREP_STEPS, type CubePrepFilters, type CubePrepInput, type CubePrepReport, type CubePrepStepName } from "../../shared/types/cubeprep";
import { formatDecimalBytes } from "../../utils/format";
import { SHOW_IN_FOLDER } from "../../utils/outputNote";
import { revealInFolder } from "../../utils/revealInFolder";
import { fileBaseName } from "../../utils/values";
import { ErrorAlert, RunButton, WarningList } from "../ui";
import { applyProfile, changeRecipeInput, CUBE_PREP_LABELS, cubePrepError, cubePrepProgressDetail, cubePrepStageText, defaultStep, inputDirectory, joinProductPath, parseRecipe, recipeForSave, recipeProblems, setRecipeStep, type CubePrepDraft, type CubePrepProfile } from "./cubePrepModel";
import { cubePrepStore, useCubePrepState } from "./cubePrepStore";
import { saguiStore } from "../sagui/saguiStore";
import CubePrepStepEditor, { PREP_BUTTON, PREP_FIELD, PrepField } from "./CubePrepStepEditor";
import { commonPsfArcsec, FITS_INPUT, FITS_INPUT_EXTENSIONS, fwhmCell, inspectionKeyOf, layersText, psfFwhmRows, psfLine, redshiftNote, redshiftRequest, scaleLine, type RedshiftNote } from "./cubePrepInspect";

const JSON_FILTER = [{ name: "Cube Prep recipe", extensions: ["json"] }];

interface PsfMeasurement { line: string | null; warnings: string[]; error: string | null }
interface PsfRun { key: string; values: Record<string, PsfMeasurement>; done: number; total: number; running: boolean }

function droppedHandoff(current: CubePrepReport | null, sent: CubePrepReport): string {
  if (current === sent) return "SAGUI changed while the cube was being checked; send again.";
  return current ? "Recipe changed; send again." : "Recipe changed; run it again before sending to SAGUI.";
}

function skippedFiles(count: number): string {
  if (count === 0) return "";
  return count === 1 ? " 1 other opened FITS file was skipped." : ` ${count} other opened FITS files were skipped.`;
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
  const [tab, setTab] = useState<"inputs" | CubePrepStepName>("inputs");
  const [catalog, setCatalog] = useState<CubePrepFilters>({ datasets: [], filters: [] });
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [catalogAttempt, setCatalogAttempt] = useState(0);
  const [localError, setLocalError] = useState<string | null>(null);
  const [inputNotice, setInputNotice] = useState<string | null>(null);
  const [inspection, setInspection] = useState<{ key: string; values: Record<string, string[]> } | null>(null);
  const inspectionRequest = useRef(0);
  const handoffRequest = useRef(0);
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
  const [now, setNow] = useState(() => Date.now());
  const ticking = busy && state.startedAt !== null;
  const elapsedMs = state.startedAt === null ? null : Math.max(0, now - state.startedAt);
  const step = recipe.steps.find((entry) => entry.step === tab);
  const problems = recipeProblems(recipe);
  const fwhmRows = report ? psfFwhmRows(report.steps) : [];
  const redshiftAsk = redshiftRequest(step?.step === "edit" && step.action === "header" ? step.redshift : null, settings, report ? commonPsfArcsec(report.steps) : null);
  const redshiftKey = redshiftAsk ? JSON.stringify(redshiftAsk) : null;
  const redshiftAskRef = useRef(redshiftAsk);
  redshiftAskRef.current = redshiftAsk;

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
      return { path, dataset: match?.dataset ?? current.recipe.inputs[0]?.dataset ?? "", filter: match?.filter ?? "" };
    })];
    cubePrepStore.edit({ ...current, recipe: { ...current.recipe, inputs }, baseDir: current.baseDir || inputDirectory(paths[0] ?? ""), outputParent: current.outputParent || inputDirectory(paths[0] ?? "") });
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
  const changeInput = (index: number, change: Partial<CubePrepInput>) => edit({ recipe: changeRecipeInput(recipe, index, change) });
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
        values[input.path] = { line: psfLine(psf), warnings: psf.warnings, error: null };
      } catch (error) { values[input.path] = { line: null, warnings: [], error: cubePrepError(error) }; }
      if (request === psfRequest.current) setPsfRun({ key, values: { ...values }, done: index + 1, total, running: index + 1 < total });
    }
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
        {CUBE_PREP_STEPS.map((name) => <button key={name} className={`${PREP_BUTTON} ${tab === name ? "border-cyan-500 text-cyan-300" : ""}`} aria-pressed={tab === name} onClick={() => setTab(name)}>{CUBE_PREP_LABELS[name]}{recipe.steps.some((entry) => entry.step === name) && <><span aria-hidden="true"> ✓</span><span className="sr-only">, enabled</span></>}</button>)}
      </nav>
      {tab === "inputs" ? <div className="space-y-3">
        <div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={() => void perform(async () => { setInputNotice(null); const paths = await open({ multiple: true, filters: [{ name: "FITS images", extensions: FITS_INPUT_EXTENSIONS }] }); if (paths) addPaths(typeof paths === "string" ? [paths] : paths); })}>Add FITS images</button><button className={PREP_BUTTON} onClick={addSelectedImage}>Use selected image</button><button className={PREP_BUTTON} disabled={inspecting || recipe.inputs.length === 0} onClick={inspectInputs}>{inspecting ? "Inspecting…" : "Inspect inputs"}</button><button className={PREP_BUTTON} disabled={Boolean(psfRun?.running) || recipe.inputs.length === 0} title="Measures the stars of each input, one image at a time; large images take a while" onClick={measurePsfs}>{psfRun?.running ? `Measuring PSF ${psfRun.done + 1} of ${psfRun.total}…` : "Measure PSFs"}</button></div>
        {inputNotice && <p role="status" className="text-[10px] text-zinc-400">{inputNotice}</p>}
        {catalogError && <div className="flex items-start gap-2"><div className="min-w-0 flex-1"><ErrorAlert message={`Could not load the filter registry: ${catalogError}`} /></div><button className={PREP_BUTTON} onClick={() => { setCatalogError(null); setCatalogAttempt((attempt) => attempt + 1); }}>Retry</button></div>}
        {recipe.inputs.length === 0 && <p className="text-[11px] text-zinc-500">Add one image per filter. Assign its dataset and filter before running.</p>}
        {recipe.inputs.map((input, index) => { const measured = psfRun?.key === inspectionKey ? psfRun.values[input.path] : undefined; return <div key={`${index}-${input.path}`} className="rounded border border-zinc-700/60 p-2 space-y-2">
          <div className="flex items-center gap-2"><span className="min-w-0 flex-1 break-all text-[10px]" title={input.path}>{input.path}</span><button className={PREP_BUTTON} aria-label={`Remove input ${index + 1}`} onClick={() => edit({ recipe: { ...recipe, inputs: recipe.inputs.filter((_, i) => i !== index) } })}>Remove</button></div>
          <div className="grid grid-cols-2 gap-2"><PrepField label={`Dataset ${index + 1}`}><input list="cubeprep-datasets" className={PREP_FIELD} value={input.dataset} onChange={(e) => changeInput(index, { dataset: e.target.value })} /></PrepField><PrepField label={`Filter ${index + 1}`}><input list={`cubeprep-filters-${index}`} className={PREP_FIELD} value={input.filter} onChange={(e) => changeInput(index, { filter: e.target.value })} /></PrepField></div>
          <datalist id={`cubeprep-filters-${index}`}>{catalog.filters.filter((entry) => entry.dataset === input.dataset).map((entry) => <option key={entry.filter} value={entry.filter}>{entry.pivot_angstrom / 10000} µm</option>)}</datalist>
          {inspection?.key === inspectionKey && inspection.values[input.path]?.map((line) => <p key={line} className="text-[10px] text-zinc-400">{line}</p>)}
          {measured?.line && <><p className="text-[10px] text-zinc-400">{measured.line}</p><WarningList warnings={measured.warnings} /></>}
          {measured?.error && <p className="text-[10px] text-amber-300">{`PSF not measured: ${measured.error}`}</p>}
        </div>; })}
        <datalist id="cubeprep-datasets">{catalog.datasets.map((dataset) => <option key={dataset} value={dataset} />)}</datalist>
      </div> : <div className="space-y-3">
        <label className="flex items-center gap-2"><input type="checkbox" checked={Boolean(step)} onChange={(e) => edit({ recipe: e.target.checked ? setRecipeStep(recipe, defaultStep(tab, recipe, draft.profile)) : { ...recipe, steps: recipe.steps.filter((entry) => entry.step !== tab) } })} />Enable {CUBE_PREP_LABELS[tab]}</label>
        {step && <CubePrepStepEditor step={step} recipe={recipe} onChange={(next) => { let updated = setRecipeStep(recipe, next); if (next.step === "units" && draft.profile === "scientific") updated = applyProfile(updated, draft.profile); edit({ recipe: updated }); }} browse={browse} redshiftNote={redshiftAnswer?.key === redshiftKey ? redshiftAnswer.note : null} />}
      </div>}
      <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Input folder (relative recipe paths)"><input className={PREP_FIELD} value={draft.baseDir} onChange={(e) => edit({ baseDir: e.target.value })} /></PrepField></div><button className={PREP_BUTTON} aria-label="Browse input folder" onClick={() => browse(true, (baseDir) => edit({ baseDir }))}>Browse</button></div>
      <div className="flex items-end gap-1"><div className="flex-1 min-w-0"><PrepField label="Working folder"><input className={PREP_FIELD} value={draft.outputParent} onChange={(e) => edit({ outputParent: e.target.value })} /></PrepField></div><button className={PREP_BUTTON} aria-label="Browse working folder" onClick={() => browse(true, (outputParent) => edit({ outputParent }))}>Browse</button></div>
      <p className="text-[10px] text-zinc-500">Each run creates a separate cubeprep folder here, with intermediate FITS files, recipe.json and result.json. Closing this panel keeps the active job running.</p>
    </fieldset>
    <ErrorAlert message={localError || state.error} />
    {problems.length > 0 && <ul className="list-disc pl-4 space-y-1 text-[10px] text-zinc-500">{problems.map((problem) => <li key={problem}>{problem}</li>)}</ul>}
    <div className="flex items-center gap-2"><button className={PREP_BUTTON} disabled={busy || json !== null || problems.length > 0} onClick={() => void cubePrepStore.validate()}>Validate</button><div className="min-w-0 flex-1"><RunButton label="Validate & run" runningLabel={state.status === "validating" ? "Validating…" : "Running…"} running={cancellable} disabled={busy || json !== null || problems.length > 0} accent="cyan" small onClick={() => void cubePrepStore.run()} /></div>{cancellable && <button className={PREP_BUTTON} aria-describedby={cancelNote} disabled={state.status === "cancelling"} onClick={() => void cubePrepStore.cancel()}>{state.status === "cancelling" ? "Cancelling…" : "Cancel"}</button>}</div>
    {cancellable && <p id={cancelNote} className="text-[10px] text-zinc-500">Stops after the current step; partial products stay in the run folder.</p>}
    <div aria-live="polite" className="space-y-2">
      {state.status === "validating" && <p>Validating files and recipe…</p>}
      {state.progress && <p>{cubePrepStageText(state.progress)}</p>}
      {state.status === "cancelled" && <p className="text-amber-300">{state.outputDir ? "Cancelled. Any partial products remain in the run folder." : "Cancelled before the run started; no run folder was created."}</p>}
      {state.status === "complete" && <p className="text-cyan-300">Run completed. Inspect the diagnostics and warnings below.</p>}
    </div>
    {state.progress && <div className="space-y-1"><progress aria-label="Cube preparation progress" className="w-full" value={state.progress.percent} max={100} /><p className="text-[10px] text-zinc-500">{cubePrepProgressDetail(state.progress, elapsedMs)}</p></div>}
    {validation && <div className="space-y-2"><WarningList warnings={validation.warnings} /><details><summary className="cursor-pointer">{`Validated recipe · ${validation.dependencies.length} ${validation.dependencies.length === 1 ? "file" : "files"} checked`}</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify(validation, null, 2)}</pre></details></div>}
    {state.outputDir && <PathNote label="Run folder" path={state.outputDir} />}
    {report && <div className="space-y-3">
      <WarningList warnings={report.warnings} />
      {report.cube && <PathNote label="Final cube" path={joinProductPath(report.output_dir, report.cube)} />}
      {report.cube && <div className="flex flex-wrap gap-1"><button className={PREP_BUTTON} onClick={() => openFits(joinProductPath(report.output_dir, report.cube!))}>Open final cube</button><button className={PREP_BUTTON} disabled={sending || busy} onClick={sendToSagui}>{sending ? "Checking cube…" : "Send to SAGUI"}</button></div>}
      {fwhmRows.length > 0 && <table data-psf-fwhm className="w-full text-left text-[10px]"><caption className="text-left text-zinc-400">PSF FWHM by filter: source, target and after matching</caption><thead><tr><th scope="col">Filter</th><th scope="col">Source</th><th scope="col">Target</th><th scope="col">Matched</th></tr></thead><tbody>{fwhmRows.map((row, index) => <tr key={`${index}-${row.filter}`}><th scope="row" className="font-mono">{row.filter}</th><td>{fwhmCell(row.sourcePx, row.scaleArcsec)}</td><td>{row.reference ? "reference" : fwhmCell(row.targetPx, row.scaleArcsec)}</td><td>{row.reference ? "--" : fwhmCell(row.matchedPx, row.scaleArcsec)}</td></tr>)}</tbody></table>}
      {report.steps.map((result) => <details key={result.index} className="rounded border border-zinc-700/60 p-2"><summary className="cursor-pointer">{result.index}. {result.step} · {(result.elapsed_ms / 1000).toFixed(1)} s</summary><WarningList warnings={result.warnings} /><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify({ parameters: result.parameters, diagnostics: result.report }, null, 2)}</pre></details>)}
      <details><summary className="cursor-pointer">Products and provenance ({report.files.length})</summary><div className="mt-2 space-y-2">{report.files.map((file) => <div key={file.path} className="text-[10px]"><div className="flex items-start gap-1"><span className="min-w-0 flex-1 break-all">{`${file.path} · ${formatDecimalBytes(file.bytes)}`}</span>{FITS_INPUT.test(file.path) && <button className={PREP_BUTTON} onClick={() => openFits(joinProductPath(report.output_dir, file.path))}>Open FITS</button>}</div>{file.sha256 && <div className="break-all text-zinc-500">SHA-256: {file.sha256}</div>}</div>)}<pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all text-[10px]">{JSON.stringify(report.provenance ?? {}, null, 2)}</pre></div></details>
    </div>}
  </div>;
}
