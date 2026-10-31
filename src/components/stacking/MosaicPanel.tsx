import { useId, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { X } from "lucide-react";
import { ErrorAlert, ResultGrid, RunButton, SectionHeader, Slider, Toggle, WarningList } from "../ui";
import { estimateMosaic, runMosaic } from "../../services/stacking";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { useTimer } from "../../hooks/useTimer";
import { useArmedConfirm } from "../../hooks/useArmedConfirm";
import { MOSAIC_PROGRESS_EVENT, type MosaicLevelModel, type MosaicResult } from "../../shared/types/stacking";
import type { ProcessedFile } from "../../shared/types";
import { sortByFrameKind } from "../../utils/frameKind";
import { isCancelledRun } from "../../utils/drizzleRgbResult";
import { formatTime } from "../../utils/format";
import { formatCount } from "../../utils/formatCount";
import { DEFAULT_SOLVE_OPTIONS, loadSolveOptions, solveOptionsStorage } from "../../utils/plateSolveOptions";
import { PIPELINE_LANES, type ComposeMaster, type PipelineLane } from "../../utils/pipelineToCompose";
import { samePath } from "../../utils/processingChain";
import { toDims } from "../../utils/stackingOutputs";
import { recordMosaic, relabelMosaic, useSessionMosaics } from "../../utils/mosaicStore";
import {
  CHOOSE_CANVAS,
  DEFAULT_MOSAIC_SETTINGS,
  LIKE_CANVAS_NOTE,
  MOSAIC_INTRO,
  MOSAIC_LEVEL_OPTIONS,
  MOSAIC_REJECT_OPTIONS,
  MOSAIC_RESIDUAL_WARNING_PX,
  MOSAIC_SOLVE_NOTE,
  NEW_CANVAS,
  canvasChoices,
  canvasSummary,
  composePlan,
  estimateDisabledReason,
  estimateNotes,
  groupViews,
  mixedFilterLine,
  mosaicChannelOf,
  mosaicOutputName,
  mosaicProgressText,
  mosaicRequest,
  mosaicRequestKey,
  mosaicSolveRequest,
  panelFilterGroups,
  rejectionLine,
  residualViews,
  runDisabledReason,
  tileViews,
  withReference,
  type MosaicBusy,
  type MosaicEstimateRecord,
  type MosaicRejectMode,
  type MosaicSettings,
} from "../../utils/mosaicView";
import { openInFiles } from "./resultHandoff";
import type { RunTarget } from "./StackingTab";

export interface MosaicPanelProps {
  files?: readonly ProcessedFile[];
  runTarget?: RunTarget | null;
  onResult?: (result: MosaicResult, target: RunTarget | null) => void;
  composeHasProgress?: boolean;
  onComposeMosaics?: (masters: readonly ComposeMaster[]) => void;
}

interface ReferenceChoice {
  key: string;
  references: readonly (number | null)[];
}

const NO_FILES: readonly ProcessedFile[] = [];
const NO_REFERENCES: ReferenceChoice = { key: "", references: [] };
const DRAFT_SOLVE = mosaicSolveRequest(DEFAULT_SOLVE_OPTIONS);
const COMPOSE_CONFIRM_KEY = "compose";
const FEATHER_MAX_PX = 256;
const FITS_FILTER = [{ name: "FITS", extensions: ["fits", "fit", "fts"] }];
const SMALL_BUTTON_CLASS = "text-[10px] text-zinc-500 hover:text-zinc-300 bg-zinc-800 px-2 py-0.5 rounded";
const HANDOFF_BUTTON_CLASS = "rounded-md bg-sky-600/20 px-2 py-1 text-[11px] text-sky-200 transition-colors hover:bg-sky-600/30";
const ARMED_HANDOFF_BUTTON_CLASS = "rounded-md bg-amber-500/15 px-2 py-1 text-[11px] text-amber-300 ring-1 ring-amber-500/30";
const ROW_LABEL_CLASS = "flex items-center justify-between gap-2 text-xs text-zinc-400";

const ICON = (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-indigo-400">
    <rect x="3" y="3" width="11" height="11" />
    <rect x="10" y="10" width="11" height="11" opacity="0.6" />
  </svg>
);

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export default function MosaicPanel({ files = NO_FILES, runTarget = null, onResult, composeHasProgress = false, onComposeMosaics }: MosaicPanelProps) {
  const mosaics = useSessionMosaics();
  const candidates = useMemo(() => sortByFrameKind(files).lights, [files]);
  const [ticks, setTicks] = useState<Readonly<Record<string, boolean>>>({});
  const paths = useMemo(
    () => candidates.filter((file) => ticks[file.path] ?? !mosaics.some((mosaic) => samePath(mosaic.fitsPath, file.path))).map((file) => file.path),
    [candidates, ticks, mosaics],
  );
  const pathsKey = JSON.stringify(paths);
  const [settings, setSettings] = useState<MosaicSettings>(DEFAULT_MOSAIC_SETTINGS);
  const [referenceChoice, setReferenceChoice] = useState<ReferenceChoice>(NO_REFERENCES);
  const [busy, setBusy] = useState<MosaicBusy>(null);
  const [estimated, setEstimated] = useState<MosaicEstimateRecord | null>(null);
  const [result, setResult] = useState<MosaicResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cancelled, setCancelled] = useState<MosaicBusy>(null);
  const progress = useProgress(MOSAIC_PROGRESS_EVENT);
  const timer = useTimer();
  const composeConfirm = useArmedConfirm();
  const estimateReasonId = useId();
  const runReasonId = useId();

  const live: MosaicSettings = { ...settings, references: referenceChoice.key === pathsKey ? referenceChoice.references : [] };
  const draft = mosaicRequest(paths, live, DRAFT_SOLVE);
  const key = mosaicRequestKey(paths, draft);
  const gate = { panels: paths.length, busy, estimate: estimated, key };
  const estimateReason = estimateDisabledReason(gate);
  const runReason = runDisabledReason(gate);

  const update = (change: Partial<MosaicSettings>) => setSettings((prev) => ({ ...prev, ...change }));
  const pickAll = (picked: boolean) => setTicks(Object.fromEntries(candidates.map((file) => [file.path, picked])));
  const pickOnly = (only: readonly string[]) => setTicks(Object.fromEntries(candidates.map((file) => [file.path, only.includes(file.path)])));
  const togglePanel = (path: string, picked: boolean) => setTicks((prev) => ({ ...prev, [path]: !picked }));
  const chooseReference = (component: number, tile: number) =>
    setReferenceChoice({ key: pathsKey, references: withReference(live.references, component, tile) });

  const chooseCanvas = async (value: string) => {
    if (value === NEW_CANVAS) return update({ canvasOf: null });
    if (value !== CHOOSE_CANVAS) return update({ canvasOf: value });
    try {
      const picked = await open({ multiple: false, directory: false, title: "FITS whose canvas to use", filters: FITS_FILTER });
      if (typeof picked !== "string") return;
      const known = mosaics.find((mosaic) => samePath(mosaic.fitsPath, picked));
      update({ canvasOf: known ? known.fitsPath : picked });
    } catch (e) {
      setError(messageOf(e));
    }
  };

  const start = (phase: Exclude<MosaicBusy, null>) => {
    setBusy(phase);
    setError(null);
    setCancelled(null);
    progress.reset();
    timer.reset();
    timer.start();
  };

  const settle = (phase: Exclude<MosaicBusy, null>, e: unknown) => {
    const message = messageOf(e);
    if (isCancelledRun(message)) setCancelled(phase);
    else setError(message);
  };

  const finish = () => {
    setBusy(null);
    timer.stop();
    progress.reset();
  };

  const handleEstimate = async () => {
    if (estimateReason !== null || busy !== null) return;
    const request = { ...draft, solve: mosaicSolveRequest(loadSolveOptions(solveOptionsStorage()).options) };
    const panels = paths;
    setEstimated(null);
    start("estimate");
    try {
      setEstimated({ key, paths: panels, request, result: await estimateMosaic(panels, request) });
    } catch (e) {
      settle("estimate", e);
    } finally {
      finish();
    }
  };

  const handleRun = async () => {
    if (runReason !== null || busy !== null || !estimated) return;
    const target = runTarget;
    const { paths: panels, request } = estimated;
    const name = mosaicOutputName(panels[0], panels.length, new Date());
    const label = mosaicChannelOf(panels, files);
    setResult(null);
    start("run");
    try {
      const res = await runMosaic([...panels], undefined, name, request);
      setResult(res);
      recordMosaic({ fitsPath: res.fits_path, written: res.written, name, label, planes: res.planes, dimensions: toDims(res.dimensions), panels: panels.length });
      onResult?.(res, target);
    } catch (e) {
      settle("run", e);
    } finally {
      finish();
    }
  };

  const plan = composePlan(mosaics);
  const composeArmed = composeConfirm.armedKey === COMPOSE_CONFIRM_KEY;
  const composeTheseMosaics = () => {
    if (!onComposeMosaics) return;
    const handOff = () => onComposeMosaics(plan.masters);
    if (composeHasProgress) {
      composeConfirm.press(COMPOSE_CONFIRM_KEY, handOff);
      return;
    }
    composeConfirm.blur(COMPOSE_CONFIRM_KEY);
    handOff();
  };

  const filterGroups = panelFilterGroups(paths, files);
  const filterLine = mixedFilterLine(filterGroups);
  const choices = canvasChoices(mosaics, settings.canvasOf);
  const picked = candidates.filter((file) => paths.includes(file.path));
  const axesValue = picked.find((file) => settings.axesOf !== null && samePath(file.path, settings.axesOf))?.path ?? picked[0]?.path ?? "";
  const shown = estimated?.result ?? null;
  const summary = shown ? canvasSummary(shown) : null;
  const groups = shown ? groupViews(shown) : [];
  const notes = shown ? [...estimateNotes(shown), ...shown.warnings] : [];
  const tiles = shown ? tileViews(shown) : [];
  const residuals = result ? residualViews(result) : [];
  const resultFiles = result ? [result.fits_path, ...(result.coverage_path ? [result.coverage_path] : [])] : [];
  const cancelLabel = busy === "estimate" ? "Cancel estimate" : "Cancel mosaic";

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Mosaic" />
      <p data-testid="mosaic-intro" className="text-[10px] leading-snug text-zinc-500">{MOSAIC_INTRO}</p>

      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <span className="text-xs text-zinc-400">{`Panels (${paths.length} of ${candidates.length})`}</span>
          <div className="flex gap-1">
            <button type="button" onClick={() => pickAll(true)} className={SMALL_BUTTON_CLASS}>All</button>
            <button type="button" onClick={() => pickAll(false)} className={SMALL_BUTTON_CLASS}>None</button>
          </div>
        </div>
        {candidates.length === 0 && <p className="text-[10px] text-zinc-500">No panels loaded yet: open the stacked panels in Files.</p>}
        {candidates.map((file) => {
          const isPicked = paths.includes(file.path);
          return (
            <label key={file.path} className="flex items-center gap-2 text-[11px] text-zinc-300">
              <input type="checkbox" checked={isPicked} aria-label={`Use ${file.name} as a panel`} onChange={() => togglePanel(file.path, isPicked)} />
              <span className="truncate" title={file.path}>{file.name}</span>
            </label>
          );
        })}
        {filterLine && (
          <div data-testid="mosaic-filters" className="flex flex-wrap items-center gap-1.5 text-[10px] text-amber-300">
            <span>{filterLine}</span>
            {filterGroups.flatMap((group) =>
              group.lane === null ? [] : [<button key={group.lane} type="button" onClick={() => pickOnly(group.paths)} className={SMALL_BUTTON_CLASS}>{`Only ${group.lane}`}</button>],
            )}
          </div>
        )}
        <p className="text-[10px] leading-snug text-zinc-500">{MOSAIC_SOLVE_NOTE}</p>
      </div>

      <div className="flex flex-col gap-2 border-t border-zinc-800/50 pt-3">
        <label className={ROW_LABEL_CLASS}>
          Canvas
          <select aria-label="Canvas" value={settings.canvasOf ?? NEW_CANVAS} onChange={(e) => void chooseCanvas(e.target.value)} className="ab-select max-w-[65%]">
            {choices.map((choice) => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
          </select>
        </label>
        {settings.canvasOf !== null ? (
          <p className="text-[10px] leading-snug text-zinc-500">{LIKE_CANVAS_NOTE}</p>
        ) : (
          <>
            <Toggle label="North up" checked={settings.northUp} accent="indigo" onChange={(northUp) => update({ northUp })} />
            {!settings.northUp && picked.length > 0 && (
              <label className={ROW_LABEL_CLASS}>
                Canvas axes follow
                <select aria-label="Canvas axes follow" value={axesValue} onChange={(e) => update({ axesOf: e.target.value })} className="ab-select max-w-[65%]">
                  {picked.map((file) => <option key={file.path} value={file.path}>{file.name}</option>)}
                </select>
              </label>
            )}
          </>
        )}
        <label className={ROW_LABEL_CLASS}>
          Level matching
          <select aria-label="Level matching" value={settings.levels} onChange={(e) => update({ levels: e.target.value as MosaicLevelModel })} className="ab-select">
            {MOSAIC_LEVEL_OPTIONS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </label>
        <Slider label="Seam feather" value={settings.featherPx} min={0} max={FEATHER_MAX_PX} step={8} accent="indigo" format={(v) => `${v} px`} onChange={(featherPx) => update({ featherPx })} />
        <Toggle label="Leave out DQ-flagged pixels" checked={settings.dqMask} accent="indigo" onChange={(dqMask) => update({ dqMask })} />
        <label className={ROW_LABEL_CLASS}>
          Outlier rejection
          <select aria-label="Outlier rejection" value={settings.reject} onChange={(e) => update({ reject: e.target.value as MosaicRejectMode })} className="ab-select">
            {MOSAIC_REJECT_OPTIONS.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </label>
        {settings.reject === "on" && (
          <>
            <Slider label="Rejection SNR" value={settings.snr} min={1} max={10} step={0.1} accent="indigo" format={(v) => v.toFixed(1)} onChange={(snr) => update({ snr })} />
            <Slider label="Gradient scale" value={settings.scale} min={0} max={3} step={0.1} accent="indigo" format={(v) => v.toFixed(1)} onChange={(scale) => update({ scale })} />
          </>
        )}
      </div>

      <div className="flex flex-col gap-1.5">
        <RunButton
          label="Estimate"
          runningLabel="Estimating..."
          running={busy === "estimate"}
          disabled={estimateReason !== null}
          accent="indigo"
          small
          describedBy={estimateReason ? estimateReasonId : undefined}
          onClick={handleEstimate}
        />
        {estimateReason && <p id={estimateReasonId} className="text-[10px] text-zinc-500">{estimateReason}</p>}
      </div>

      {estimated && shown && (
        <div data-testid="mosaic-estimate" className="flex flex-col gap-2 rounded border border-zinc-800/60 p-2">
          {summary && <p data-testid="mosaic-canvas" className="text-[11px] text-zinc-300">{summary}</p>}
          {shown.refusal && <p data-testid="mosaic-refusal" className="text-[11px] text-red-400">{shown.refusal}</p>}
          {groups.map((group) => (
            <div key={group.component} data-testid="mosaic-group" className="flex flex-wrap items-center justify-between gap-2 text-[10px] text-zinc-400">
              <span>{`Group ${group.number}: ${group.panels.map((panel) => panel.name).join(", ")}`}</span>
              <label className="flex items-center gap-1">
                Level reference
                <select
                  aria-label={`Level reference of group ${group.number}`}
                  value={String(live.references[group.component] ?? group.reference)}
                  onChange={(e) => chooseReference(group.component, Number(e.target.value))}
                  className="ab-select"
                >
                  {group.panels.map((panel) => <option key={panel.index} value={String(panel.index)}>{panel.name}</option>)}
                </select>
              </label>
            </div>
          ))}
          <div data-testid="mosaic-estimate-warnings" className="flex flex-col gap-1 empty:hidden">
            <WarningList warnings={notes} />
          </div>
          <p data-testid="mosaic-rejection-line" className="text-[10px] leading-snug text-zinc-500">{rejectionLine(estimated.request.reject.mode, shown)}</p>
          <div className="flex flex-col gap-1">
            {tiles.map((tile) => (
              <div key={tile.index} data-testid="mosaic-tile-row" className="flex flex-col gap-0.5 rounded bg-zinc-900/50 px-2 py-1 text-[10px] text-zinc-400">
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate text-zinc-300">{tile.name}</span>
                  {tile.group && <span className="shrink-0">{tile.reference ? `${tile.group} · reference` : tile.group}</span>}
                </div>
                <div>{`${tile.wcs} · scale ${tile.scale} · rotation ${tile.rotation} · ${tile.coverage}${tile.singleExposure ? " · single exposure" : ""}`}</div>
                {tile.solve && <div className="text-zinc-500">{tile.solve}</div>}
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="flex flex-col gap-1.5">
        <RunButton
          label={`Run mosaic (${paths.length} panels)`}
          runningLabel="Building mosaic..."
          running={busy === "run"}
          disabled={runReason !== null}
          accent="indigo"
          describedBy={runReason ? runReasonId : undefined}
          onClick={handleRun}
        />
        {runReason && <p id={runReasonId} className="text-[10px] text-zinc-500">{runReason}</p>}
      </div>

      {busy && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div data-testid="mosaic-progress-bar" className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-indigo), #a5b4fc)" }} />
          </div>
          <div className="flex justify-between items-center gap-2 text-[10px] text-zinc-500">
            <span data-testid="mosaic-progress-label" className="truncate">{mosaicProgressText(progress.stage, busy)}</span>
            <span className="flex shrink-0 items-center gap-2">
              <span data-testid="mosaic-elapsed" className="font-mono">{timer.formatted}</span>
              <button
                type="button"
                data-testid="mosaic-cancel"
                onClick={() => { cancelProgress(MOSAIC_PROGRESS_EVENT).catch(() => {}); }}
                title={cancelLabel}
                aria-label={cancelLabel}
                className="text-zinc-500 hover:text-red-400 transition-colors"
              >
                <X size={11} />
              </button>
            </span>
          </div>
        </div>
      )}
      {cancelled && <div className="text-[10px] text-zinc-500">{cancelled === "estimate" ? "Estimate cancelled" : "Mosaic cancelled"}</div>}
      <ErrorAlert message={error} />

      {result && (
        <div data-testid="mosaic-result" className="flex flex-col gap-2 animate-fade-in border-t border-zinc-800/50 pt-3">
          <span className="text-xs font-semibold text-zinc-400">Result</span>
          <ResultGrid
            items={[
              { label: "Canvas", value: `${formatCount(result.canvas.cols)} × ${formatCount(result.canvas.rows)}` },
              { label: "Panels", value: result.tiles.length },
              { label: "Rejected px", value: result.reject ? formatCount(result.rejected_px) : "off" },
            ]}
          />
          {residuals.length > 0 && (
            <div className="flex flex-col gap-0.5">
              <span className="text-[10px] text-zinc-500">Star offsets in the overlaps</span>
              {residuals.map((residual) => (
                <div key={residual.pair} data-testid="mosaic-residual-row" data-warn={residual.warn} className={`text-[10px] ${residual.warn ? "text-amber-300" : "text-zinc-400"}`}>
                  {`${residual.pair}: ${residual.shift}, ${residual.stars}${residual.warn ? ` · above ${MOSAIC_RESIDUAL_WARNING_PX} px` : ""}`}
                </div>
              ))}
            </div>
          )}
          <div data-testid="mosaic-result-warnings" className="flex flex-col gap-1 empty:hidden">
            <WarningList warnings={result.warnings} />
          </div>
          <div className="text-[10px] text-zinc-500">{`Finished in ${formatTime(result.elapsed_ms)}`}</div>
          <div className="flex flex-wrap gap-2">
            <button type="button" onClick={() => openInFiles(resultFiles)} className={HANDOFF_BUTTON_CLASS}>Open in Files</button>
          </div>
        </div>
      )}

      {mosaics.length > 0 && (
        <div data-testid="mosaic-session" className="flex flex-col gap-1.5 border-t border-zinc-800/50 pt-3">
          <span className="text-xs text-zinc-400">Mosaics of this session</span>
          {mosaics.map((mosaic) => (
            <div key={mosaic.fitsPath} className="flex items-center justify-between gap-2 text-[10px] text-zinc-400">
              <span className="truncate" title={mosaic.fitsPath}>{mosaic.name}</span>
              {mosaic.planes === 1 ? (
                <select aria-label={`Channel of ${mosaic.name}`} value={mosaic.label} onChange={(e) => relabelMosaic(mosaic.fitsPath, e.target.value as PipelineLane)} className="ab-select">
                  {PIPELINE_LANES.map((lane) => <option key={lane} value={lane}>{lane}</option>)}
                </select>
              ) : (
                <span className="shrink-0">RGB</span>
              )}
            </div>
          ))}
          {plan.notes.map((note) => <p key={note} className="text-[10px] text-zinc-500">{note}</p>)}
          {onComposeMosaics && plan.masters.length > 0 && (
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                data-testid="mosaic-compose"
                aria-live="polite"
                aria-label={composeArmed ? "Confirm replacing the Compose channels with these mosaics" : undefined}
                title={composeArmed ? "Click again to replace the Compose channels with these mosaics; the Compose step results are discarded" : "Open the newest mosaic of each channel and put it in that Compose channel"}
                onClick={composeTheseMosaics}
                onBlur={() => composeConfirm.blur(COMPOSE_CONFIRM_KEY)}
                className={composeArmed ? ARMED_HANDOFF_BUTTON_CLASS : HANDOFF_BUTTON_CLASS}
              >
                {composeArmed ? "Replace Compose channels?" : "Compose these mosaics"}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
