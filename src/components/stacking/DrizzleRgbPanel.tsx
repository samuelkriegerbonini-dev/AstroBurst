import { useState, useCallback, useId, useMemo } from "react";
import { Grid3X3, CheckCircle2, Wand2, X } from "lucide-react";
import { Slider, Toggle, RunButton, ResultGrid, ErrorAlert, SectionHeader, WarningList, DecimalField } from "../ui";
import { useProgress } from "../../hooks/useProgress";
import { drizzleFrames, drizzleRgbStack } from "../../services/stacking";
import { cancelProgress } from "../../services/progress";
import { getOutputDir } from "../../infrastructure/tauri";
import type { ProcessedFile } from "../../shared/types";
import type { DrizzleAlignmentMethod, DrizzleRgbResult, StackResult } from "../../shared/types/stacking";
import { DRIZZLE_ALIGNMENT_METHODS, DRIZZLE_RGB_PROGRESS_EVENT, STACK_PROGRESS_EVENT } from "../../shared/types/stacking";
import { rejectionUsesSigma, stackProgressText, type RejectionSettings } from "../../utils/stackingRejection";
import { DRIZZLE_GAIN_FIELD, drizzleResultItems, isCancelledRun, manualGains, monoDrizzleResultItems } from "../../utils/drizzleRgbResult";
import { cullDrizzleChannels, subframeExclusionNotice } from "../../utils/subframeCull";
import { DRIZZLE_CHANNELS, monoDrizzleFilterLabel, unassignedFilterLine, withAutoChannels, type DrizzleChannel } from "../../utils/drizzleChannel";
import {
  DEFAULT_DRIZZLE_KERNEL,
  DRIZZLE_FIXED_REJECTION,
  DRIZZLE_KERNELS,
  DRIZZLE_PIXFRAC,
  DRIZZLE_REJECTION_DEFAULTS,
  DRIZZLE_SCALE,
  type DrizzleKernel,
} from "../../utils/drizzleParams";
import { imagetypLeftOutLine, sortByFrameKind } from "../../utils/frameKind";
import { drizzleOutputName } from "../../utils/stackingOutputs";
import { claimStackProgress, releaseStackProgress, stackProgressBusyLine, useStackProgressHolder } from "../../utils/stackProgressLock";
import { openInFiles } from "./resultHandoff";
import RejectionControls from "./RejectionControls";
import type { RunTarget } from "./StackingTab";

type Channel = DrizzleChannel;

type FinishedDrizzle =
  | { mode: "rgb"; result: DrizzleRgbResult; kernel: string }
  | { mode: "mono"; label: string; result: StackResult; kernel: string };

interface DrizzleRgbPanelProps {
  files: ProcessedFile[];
  runTarget?: RunTarget | null;
  onResult?: (result: DrizzleRgbResult, inputs: string[], target: RunTarget | null) => void;
  onMonoResult?: (result: StackResult, inputs: string[], target: RunTarget | null, filterLabel: string) => void;
  rejectedPaths?: readonly string[];
  onUseAllFrames?: () => void;
}

const NO_PATHS: readonly string[] = [];
const OPEN_IN_FILES_CLASS = "shrink-0 rounded-md bg-emerald-600/20 px-2 py-1 text-[11px] text-emerald-200 transition-colors hover:bg-emerald-600/30";

const ICON = <Grid3X3 size={14} className="text-rose-400" />;

const CHANNEL_STYLES: Record<Channel, { active: string; idle: string }> = {
  r: { active: "bg-red-500/25 text-red-300 ring-1 ring-red-500/50", idle: "text-zinc-600 hover:text-red-400" },
  g: { active: "bg-green-500/25 text-green-300 ring-1 ring-green-500/50", idle: "text-zinc-600 hover:text-green-400" },
  b: { active: "bg-blue-500/25 text-blue-300 ring-1 ring-blue-500/50", idle: "text-zinc-600 hover:text-blue-400" },
};

const NOTE_CLASS = "-mt-3 text-[10px] text-amber-400/90 leading-snug";

export default function DrizzleRgbPanel({ files = [], runTarget = null, onResult, onMonoResult, rejectedPaths = NO_PATHS, onUseAllFrames }: DrizzleRgbPanelProps) {
  const [assignments, setAssignments] = useState<Record<string, Channel>>({});
  const [scale, setScale] = useState(DRIZZLE_SCALE.initial);
  const [pixfrac, setPixfrac] = useState(DRIZZLE_PIXFRAC.initial);
  const [kernel, setKernel] = useState<DrizzleKernel>(DEFAULT_DRIZZLE_KERNEL);
  const [align, setAlign] = useState(true);
  const [alignmentMethod, setAlignmentMethod] = useState<DrizzleAlignmentMethod>("phase_correlation");
  const [rejectionSettings, setRejectionSettings] = useState<RejectionSettings>(DRIZZLE_REJECTION_DEFAULTS);
  const [wbMode, setWbMode] = useState<"auto" | "manual" | "none">("auto");
  const [wbRText, setWbRText] = useState("1");
  const [wbGText, setWbGText] = useState("1");
  const [wbBText, setWbBText] = useState("1");
  const [scnrEnabled, setScnrEnabled] = useState(false);
  const [scnrAmount, setScnrAmount] = useState(0.8);
  const [scnrMethod, setScnrMethod] = useState<"average" | "maximum">("average");
  const [saveFits, setSaveFits] = useState(true);
  const [isRunning, setIsRunning] = useState(false);
  const [finished, setFinished] = useState<FinishedDrizzle | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [runEvent, setRunEvent] = useState<string>(DRIZZLE_RGB_PROGRESS_EVENT);
  const rgbProgress = useProgress(DRIZZLE_RGB_PROGRESS_EVENT);
  const monoProgress = useProgress(STACK_PROGRESS_EVENT);
  const progress = runEvent === STACK_PROGRESS_EVENT ? monoProgress : rgbProgress;
  const resetRgbProgress = rgbProgress.reset;
  const resetMonoProgress = monoProgress.reset;
  const progressHolder = useStackProgressHolder();
  const kernelId = useId();
  const alignmentId = useId();
  const whiteBalanceId = useId();
  const gainId = useId();
  const scnrMethodId = useId();
  const leftOutId = useId();
  const unassignedId = useId();
  const busyId = useId();

  const channelPaths = useMemo(() => {
    const out: Record<Channel, string[]> = { r: [], g: [], b: [] };
    for (const f of files) {
      const ch = assignments[f.path];
      if (ch) out[ch].push(f.path);
    }
    return out;
  }, [files, assignments]);

  const culled = useMemo(() => cullDrizzleChannels(channelPaths, rejectedPaths), [channelPaths, rejectedPaths]);
  const culledPaths = culled.channels;
  const rejectedSet = useMemo(() => new Set(rejectedPaths), [rejectedPaths]);
  const exclusionNotice = subframeExclusionNotice(culled.excluded);
  const leftOutLine = useMemo(() => imagetypLeftOutLine(sortByFrameKind(files).calibration), [files]);
  const unassignedLine = useMemo(() => unassignedFilterLine(files, assignments), [files, assignments]);
  const autoNoteIds = [leftOutLine ? leftOutId : null, unassignedLine ? unassignedId : null].filter(Boolean).join(" ") || undefined;

  const readyChannels = DRIZZLE_CHANNELS.filter((ch) => culledPaths[ch].length >= 2);
  const shortChannels = DRIZZLE_CHANNELS.filter((ch) => culledPaths[ch].length === 1);
  const monoChannel = readyChannels.length === 1 ? readyChannels[0] : null;
  const monoLabel = useMemo(() => {
    if (!monoChannel) return null;
    const inChannel = new Set(culledPaths[monoChannel]);
    return monoDrizzleFilterLabel(files.filter((f) => inChannel.has(f.path)), monoChannel);
  }, [files, culledPaths, monoChannel]);
  const canRun = readyChannels.length >= 1;
  const busyLine = monoChannel ? stackProgressBusyLine(progressHolder, isRunning ? "drizzle" : null) : null;
  const manual = useMemo(() => manualGains({ r: wbRText, g: wbGText, b: wbBText }), [wbRText, wbGText, wbBText]);
  const gainError = wbMode === "manual" && !monoChannel ? manual.error : null;

  const toggleAssignment = useCallback((path: string, ch: Channel) => {
    setAssignments((prev) => {
      const next = { ...prev };
      if (next[path] === ch) {
        delete next[path];
      } else {
        next[path] = ch;
      }
      return next;
    });
  }, []);

  const autoAssign = useCallback(() => {
    setAssignments((prev) => withAutoChannels(prev, files));
  }, [files]);

  const clearAssignments = useCallback(() => setAssignments({}), []);

  const handleRun = useCallback(async () => {
    const gains = wbMode === "manual" ? manual.gains : null;
    if (!canRun || (!monoChannel && wbMode === "manual" && gains === null)) return;
    if (monoChannel && !claimStackProgress("drizzle")) return;
    const target = runTarget;
    const inputs = monoChannel ? [...culledPaths[monoChannel]] : [...culledPaths.r, ...culledPaths.g, ...culledPaths.b];
    const kernelAtRun = kernel;
    const resetRunProgress = monoChannel ? resetMonoProgress : resetRgbProgress;
    const { rejection, sigmaLow, sigmaHigh } = rejectionSettings;
    const sigmas = rejectionUsesSigma(rejection) ? { sigmaLow, sigmaHigh } : { sigmaLow: undefined, sigmaHigh: undefined };
    setRunEvent(monoChannel ? STACK_PROGRESS_EVENT : DRIZZLE_RGB_PROGRESS_EVENT);
    setIsRunning(true);
    setError(null);
    setCancelled(false);
    setFinished(null);
    resetRunProgress();
    try {
      if (monoChannel) {
        const res = await drizzleFrames(inputs, await getOutputDir(), {
          scale,
          pixfrac,
          kernel,
          align,
          name: drizzleOutputName(inputs[0], inputs.length, new Date()),
          rejection,
          ...sigmas,
        });
        const label = monoLabel ?? monoChannel.toUpperCase();
        setFinished({ mode: "mono", label, result: res, kernel: kernelAtRun });
        onMonoResult?.(res, inputs, target, label);
      } else {
        const res = await drizzleRgbStack(
          culledPaths.r,
          culledPaths.g,
          culledPaths.b,
          await getOutputDir(),
          {
            scale,
            pixfrac,
            kernel,
            align,
            alignmentMethod: align ? alignmentMethod : undefined,
            rejection,
            ...sigmas,
            wbMode,
            wbR: gains?.wbR,
            wbG: gains?.wbG,
            wbB: gains?.wbB,
            scnrEnabled,
            scnrAmount: scnrEnabled ? scnrAmount : undefined,
            scnrMethod: scnrEnabled ? scnrMethod : undefined,
            saveFits,
          },
        );
        setFinished({ mode: "rgb", result: res, kernel: kernelAtRun });
        onResult?.(res, inputs, target);
      }
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      if (isCancelledRun(message)) {
        setCancelled(true);
      } else {
        setError(message);
      }
    } finally {
      setIsRunning(false);
      resetRunProgress();
      if (monoChannel) releaseStackProgress("drizzle");
    }
  }, [canRun, monoChannel, monoLabel, culledPaths, scale, pixfrac, kernel, align, alignmentMethod, rejectionSettings, wbMode, manual, scnrEnabled, scnrAmount, scnrMethod, saveFits, resetRgbProgress, resetMonoProgress, runTarget, onResult, onMonoResult]);

  const totalAssigned = channelPaths.r.length + channelPaths.g.length + channelPaths.b.length;
  const runLabel = monoLabel ? `Drizzle ${monoLabel} (mono)` : `Drizzle ${readyChannels.map((c) => c.toUpperCase()).join("+") || "RGB"}`;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <div className="flex items-center justify-between">
        <SectionHeader
          icon={ICON}
          title="Drizzle RGB"
          subtitle={totalAssigned > 0 ? `${totalAssigned} frames assigned` : "Assign frames to channels"}
        />
        <div className="flex gap-2 items-center">
          <button onClick={autoAssign} aria-describedby={autoNoteIds} className="flex items-center gap-1 text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors" title="Assign lights to R, G and B from the FITS filter, else the file name">
            <Wand2 size={10} />
            Auto
          </button>
          <button onClick={clearAssignments} className="text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors">Clear</button>
        </div>
      </div>

      {leftOutLine && <p id={leftOutId} data-testid="drizzle-left-out" className={NOTE_CLASS}>{leftOutLine}</p>}
      {unassignedLine && <p id={unassignedId} data-testid="drizzle-unassigned" className={NOTE_CLASS}>{unassignedLine}</p>}

      {files.length === 0 && (
        <div className="text-[11px] text-zinc-500 italic">Load FITS frames first, then assign each one to R, G or B.</div>
      )}

      <div className="flex flex-col gap-1 max-h-[200px] overflow-y-auto">
        {files.map((f) => {
          const assigned = assignments[f.path];
          return (
            <div
              key={f.id}
              className={`flex items-center gap-2 px-2.5 py-1.5 rounded text-[11px] transition-all ${
                assigned ? "bg-zinc-800/60 text-zinc-200" : "text-zinc-500 hover:bg-zinc-800/40"
              }`}
            >
              <span className="truncate flex-1">{f.name}</span>
              {rejectedSet.has(f.path) && (
                <span className="shrink-0 text-[9px] text-amber-400/80" title="Rejected in the Subframe Selector; left out of the drizzle">culled</span>
              )}
              <div className="flex gap-1 shrink-0">
                {(["r", "g", "b"] as Channel[]).map((ch) => (
                  <button
                    key={ch}
                    onClick={() => toggleAssignment(f.path, ch)}
                    className={`w-6 h-5 rounded text-[10px] font-bold uppercase transition-all ${
                      assigned === ch ? CHANNEL_STYLES[ch].active : CHANNEL_STYLES[ch].idle
                    }`}
                  >
                    {ch}
                  </button>
                ))}
              </div>
            </div>
          );
        })}
      </div>

      <div className="grid grid-cols-3 gap-1.5">
        {(["r", "g", "b"] as Channel[]).map((ch) => (
          <div key={ch} className="ab-metric-card flex flex-col items-center p-2 rounded">
            <span className={`text-[9px] uppercase ${ch === "r" ? "text-red-400/90" : ch === "g" ? "text-green-400/90" : "text-blue-400/90"}`}>
              {ch} frames
            </span>
            <span className={`text-sm font-mono ${culledPaths[ch].length >= 2 ? "text-zinc-200" : "text-zinc-600"}`}>
              {culledPaths[ch].length}
            </span>
          </div>
        ))}
      </div>

      {shortChannels.length > 0 && (
        <div className="text-[10px] text-amber-400/90 bg-amber-900/20 border border-amber-800/30 rounded px-2.5 py-1.5">
          Channel{shortChannels.length > 1 ? "s" : ""} {shortChannels.map((c) => c.toUpperCase()).join(", ")} need at
          least 2 frames for drizzle and will be skipped.
        </div>
      )}

      <div className="flex flex-col gap-3 border-t border-zinc-800/50 pt-3">
        <span className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">Drizzle Parameters</span>
        <Slider label="Scale" value={scale} min={DRIZZLE_SCALE.min} max={DRIZZLE_SCALE.max} step={DRIZZLE_SCALE.step} accent="rose" format={(v) => `${v.toFixed(1)}x`} onChange={setScale} />
        <Slider label="Pixfrac" value={pixfrac} min={DRIZZLE_PIXFRAC.min} max={DRIZZLE_PIXFRAC.max} step={DRIZZLE_PIXFRAC.step} accent="rose" format={(v) => v.toFixed(2)} onChange={setPixfrac} />
        <div className="flex items-center justify-between">
          <label htmlFor={kernelId} className="text-xs text-zinc-400">Kernel</label>
          <select
            id={kernelId}
            value={kernel}
            onChange={(e) => setKernel(e.target.value as DrizzleKernel)}
            className="ab-select"
            disabled={isRunning}
          >
            {DRIZZLE_KERNELS.map((k) => (
              <option key={k.value} value={k.value}>{k.label}</option>
            ))}
          </select>
        </div>
        <Toggle label="Align frames and channels" checked={align} accent="rose" onChange={setAlign} />
        {align && monoChannel && (
          <div className="flex items-center justify-between">
            <span className="text-xs text-zinc-400">Alignment</span>
            <span data-testid="drizzle-mono-alignment" className="text-[11px] text-zinc-500">Phase Correlation, fixed in mono</span>
          </div>
        )}
        {align && !monoChannel && (
          <div className="flex items-center justify-between">
            <label htmlFor={alignmentId} className="text-xs text-zinc-400">Alignment</label>
            <select
              id={alignmentId}
              value={alignmentMethod}
              onChange={(e) => setAlignmentMethod(e.target.value as DrizzleAlignmentMethod)}
              className="ab-select"
              disabled={isRunning}
            >
              {DRIZZLE_ALIGNMENT_METHODS.map((m) => (
                <option key={m.value} value={m.value}>{m.label}</option>
              ))}
            </select>
          </div>
        )}
        <RejectionControls
          settings={rejectionSettings}
          onChange={(change) => setRejectionSettings((prev) => ({ ...prev, ...change }))}
          frameCount={readyChannels.length > 0 ? Math.min(...readyChannels.map((ch) => culledPaths[ch].length)) : null}
          accent="rose"
          label="Pixel rejection"
          fixed={DRIZZLE_FIXED_REJECTION}
          disabled={isRunning}
        />
        {!monoChannel && (
          <>
            <div className="flex items-center justify-between">
              <label htmlFor={whiteBalanceId} className="text-xs text-zinc-400">White balance</label>
              <select
                id={whiteBalanceId}
                value={wbMode}
                onChange={(e) => setWbMode(e.target.value as typeof wbMode)}
                className="ab-select"
                disabled={isRunning}
              >
                <option value="auto">Auto</option>
                <option value="manual">Manual</option>
                <option value="none">None</option>
              </select>
            </div>
            {wbMode === "manual" && (
              <>
                <div className="grid grid-cols-3 gap-2">
                  {[
                    { label: "R", text: wbRText, set: setWbRText },
                    { label: "G", text: wbGText, set: setWbGText },
                    { label: "B", text: wbBText, set: setWbBText },
                  ].map(({ label, text, set }) => (
                    <div key={label} className="flex flex-col gap-0.5">
                      <label htmlFor={`${gainId}-${label}`} className="text-[9px] text-zinc-500 uppercase">{label} gain</label>
                      <DecimalField
                        id={`${gainId}-${label}`}
                        {...DRIZZLE_GAIN_FIELD}
                        value={text}
                        onChange={set}
                        disabled={isRunning}
                        className="bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-rose-500/50 w-full"
                      />
                    </div>
                  ))}
                </div>
                {gainError && <div className="text-[9px] text-amber-400/90" data-field-error>{gainError}</div>}
              </>
            )}
            <Toggle label="SCNR green removal" checked={scnrEnabled} accent="rose" onChange={setScnrEnabled} />
            {scnrEnabled && (
              <>
                <Slider label="SCNR amount" value={scnrAmount} min={0} max={1} step={0.05} accent="rose"
                        format={(v) => `${(v * 100).toFixed(0)}%`} onChange={setScnrAmount} />
                <div className="flex items-center justify-between">
                  <label htmlFor={scnrMethodId} className="text-xs text-zinc-400">SCNR method</label>
                  <select
                    id={scnrMethodId}
                    value={scnrMethod}
                    onChange={(e) => setScnrMethod(e.target.value as "average" | "maximum")}
                    className="ab-select"
                    disabled={isRunning}
                  >
                    <option value="average">Average neutral</option>
                    <option value="maximum">Maximum neutral</option>
                  </select>
                </div>
              </>
            )}
            <Toggle label="Save FITS alongside PNG" checked={saveFits} accent="rose" onChange={setSaveFits} />
          </>
        )}
      </div>

      {exclusionNotice && (
        <div className="flex items-start justify-between gap-2 text-[10px] text-amber-400/90 bg-amber-900/20 border border-amber-800/30 rounded px-2.5 py-1.5">
          <span data-subframe-notice className="break-words">{exclusionNotice}</span>
          <button
            type="button"
            onClick={() => onUseAllFrames?.()}
            className="shrink-0 text-[10px] text-amber-300 hover:text-amber-200 bg-zinc-800 px-2 py-0.5 rounded"
          >Use all frames</button>
        </div>
      )}

      <RunButton
        label={runLabel}
        runningLabel="Drizzling..."
        running={isRunning}
        disabled={!canRun || gainError !== null || busyLine !== null}
        describedBy={busyLine ? busyId : undefined}
        accent="rose"
        onClick={handleRun}
      />
      {busyLine && <p id={busyId} data-testid="drizzle-progress-busy" className={NOTE_CLASS}>{busyLine}</p>}

      {isRunning && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-rose), #fda4af)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{stackProgressText(progress.stage, progress.current, progress.total)}</span>
            <span className="flex items-center gap-2">
              <span className="tabular-nums">{progress.percent}%</span>
              <button
                type="button"
                data-testid="drizzle-rgb-cancel"
                onClick={() => { cancelProgress(runEvent).catch(() => {}); }}
                title="Cancel drizzle"
                aria-label="Cancel drizzle"
                className="text-zinc-500 hover:text-red-400 transition-colors"
              >
                <X size={11} />
              </button>
            </span>
          </div>
        </div>
      )}

      {cancelled && <div className="text-[10px] text-zinc-500">Drizzle cancelled</div>}

      <ErrorAlert message={error} />

      {finished && (
        <div className="flex flex-col gap-2 animate-fade-in bg-emerald-500/10 border border-emerald-500/20 rounded-lg px-3 py-2.5">
          <div className="flex items-center gap-1.5 text-xs text-emerald-300 font-medium">
            <CheckCircle2 size={12} />
            {finished.mode === "mono" ? `Drizzle ${finished.label} (mono) Complete` : "Drizzle RGB Complete"}
          </div>
          <ResultGrid
            columns={3}
            items={finished.mode === "mono"
              ? monoDrizzleResultItems(finished.result, finished.label, finished.kernel)
              : drizzleResultItems(finished.result, finished.kernel)}
          />
          <WarningList warnings={finished.result.warnings} />
          {finished.result.fits_path && (
            <div className="flex items-center justify-between gap-2">
              <div className="min-w-0 text-[10px] text-zinc-500 truncate" title={finished.result.fits_path}>
                FITS: {finished.result.fits_path}
              </div>
              <button type="button" onClick={() => { if (finished.result.fits_path) openInFiles([finished.result.fits_path]); }} className={OPEN_IN_FILES_CLASS}>Open in Files</button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
