import { useState, useCallback, useEffect, useId, useMemo } from "react";
import { Loader2, BarChart3, Check, X } from "lucide-react";
import type { WizardState } from "../wizard";
import { effectiveBinFiles, wizardStackName } from "../../../utils/wizard";
import { stackAllNext, stackRunFinish, type StackBinOutcome, type StackRunStart } from "../../../utils/stackRun";
import { stackFrames, drizzleFrames } from "../../../services/stacking";
import { analyzeSubframes } from "../../../services/analysis";
import { getOutputDir } from "../../../infrastructure/tauri";
import { RunButton, Slider, Toggle, WarningList } from "../../ui";
import { useComposeWizardContext, type WizardAction, type WizardStackRun } from "../../../context/ComposeWizardContext";
import { resolveEffectivePath } from "../../../hooks/useFileStore";
import { STACK_PROGRESS_EVENT, type CombineMethod, type NormalizationMethod } from "../../../shared/types/stacking";
import {
  boundRejectionSettings,
  COMBINE_OPTIONS,
  DEFAULT_STACK_SETTINGS,
  NORMALIZATION_OPTIONS,
  rejectionUsesSigma,
  stackProgressText,
  type RejectionSettings,
} from "../../../utils/stackingRejection";
import { formatCount } from "../../../utils/formatCount";
import RejectionControls from "../../stacking/RejectionControls";
import { cancelProgress } from "../../../services/progress";
import { useProgress } from "../../../hooks/useProgress";
import { formatTime } from "../../../utils/format";
import {
  DEFAULT_SUBFRAME_THRESHOLDS,
  SUBFRAME_STALE_NOTE,
  analysedThresholdsOf,
  subframeResultStale,
  type SubframeThresholds,
} from "../../../utils/subframeSettings";
import {
  DEFAULT_DRIZZLE_KERNEL,
  DRIZZLE_FIXED_REJECTION,
  DRIZZLE_KERNELS,
  DRIZZLE_PIXFRAC,
  DRIZZLE_SCALE,
  type DrizzleKernel,
} from "../../../utils/drizzleParams";
import {
  claimStackProgress,
  releaseStackProgress,
  stackProgressBusyLine,
  stackProgressHolder,
  useStackProgressHolder,
} from "../../../utils/stackProgressLock";

const ELAPSED_TICK_MS = 100;
const COMPOSE_REJECTION_DEFAULTS: RejectionSettings = { ...boundRejectionSettings(DEFAULT_STACK_SETTINGS), sigmaLow: 3.0, sigmaHigh: 3.0 };

let lastStackToken = 0;

interface StackStepProps {
  state: WizardState;
  dispatch: React.Dispatch<WizardAction>;
  onStacked: (channelId: string, path: string) => void;
}

interface StackDisplayResult {
  fits_path?: string;
  frame_count?: number;
  rejected_pixels?: number;
  elapsed_ms?: number;
  warnings?: string[];
}

export default function StackStep({ state, dispatch, onStacked }: StackStepProps) {
  const {
    stackRun,
    setStackRun,
    finishStackRun,
    getStackGeneration,
    getState,
    stackDiscarded,
    recordStackOutcome,
  } = useComposeWizardContext();
  const [now, setNow] = useState(() => Date.now());
  const [results, setResults] = useState<Record<string, StackDisplayResult>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [cancelled, setCancelled] = useState<Record<string, boolean>>({});
  const progress = useProgress(STACK_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const [analyzing, setAnalyzing] = useState<Record<string, boolean>>({});
  const [analyzeErrors, setAnalyzeErrors] = useState<Record<string, string>>({});
  const [overrides, setOverrides] = useState<Record<string, Record<string, boolean>>>({});
  const [initialThresholds] = useState<SubframeThresholds>(
    () => analysedThresholdsOf(state.bins, state.subframeResults) ?? DEFAULT_SUBFRAME_THRESHOLDS,
  );
  const [maxFwhm, setMaxFwhm] = useState(initialThresholds.maxFwhm);
  const [maxEcc, setMaxEcc] = useState(initialThresholds.maxEccentricity);
  const [minSnr, setMinSnr] = useState(initialThresholds.minSnr);
  const [minStars, setMinStars] = useState(initialThresholds.minStars);
  const thresholds = useMemo<SubframeThresholds>(
    () => ({ maxFwhm, maxEccentricity: maxEcc, minSnr, minStars }),
    [maxFwhm, maxEcc, minSnr, minStars],
  );
  const [useDrizzle, setUseDrizzle] = useState(false);
  const [drizzleScale, setDrizzleScale] = useState(DRIZZLE_SCALE.initial);
  const [drizzlePixfrac, setDrizzlePixfrac] = useState(DRIZZLE_PIXFRAC.initial);
  const [drizzleKernel, setDrizzleKernel] = useState<DrizzleKernel>(DEFAULT_DRIZZLE_KERNEL);
  const combineId = useId();
  const normalizationId = useId();
  const subframeStaleId = useId();
  const kernelId = useId();
  const busyId = useId();
  const busyLine = stackProgressBusyLine(useStackProgressHolder(), stackRun ? "compose" : null);
  const [rejectionSettings, setRejectionSettings] = useState<RejectionSettings>(COMPOSE_REJECTION_DEFAULTS);
  const [combine, setCombine] = useState<CombineMethod>("mean");
  const [normalization, setNormalization] = useState<NormalizationMethod>(DEFAULT_STACK_SETTINGS.normalization);

  const stackableBins = useMemo(
    () => state.bins.filter((b) => b.files.length > 1),
    [state.bins],
  );

  const singleBins = useMemo(
    () => state.bins.filter((b) => b.files.length === 1),
    [state.bins],
  );

  const handleAnalyze = useCallback(async (binId: string, files: string[]) => {
    if (files.length === 0) return;
    setAnalyzing((prev) => ({ ...prev, [binId]: true }));
    setAnalyzeErrors((prev) => ({ ...prev, [binId]: "" }));
    setOverrides((prev) => ({ ...prev, [binId]: {} }));
    try {
      const res = await analyzeSubframes(files, thresholds);
      dispatch({ type: "SET_SUBFRAME_RESULT", binId, result: { ...res, analysedWith: thresholds } });
    } catch (e) {
      setAnalyzeErrors((prev) => ({ ...prev, [binId]: e instanceof Error ? e.message : String(e) }));
    } finally {
      setAnalyzing((prev) => ({ ...prev, [binId]: false }));
    }
  }, [thresholds, dispatch]);

  const toggleOverride = useCallback((binId: string, filePath: string) => {
    setOverrides((prev) => {
      const binOverrides = { ...(prev[binId] ?? {}) };
      const sub = state.subframeResults[binId]?.subframes.find((s) => s.file_path === filePath);
      const original = sub?.accepted ?? true;
      if (binOverrides[filePath] === undefined) {
        binOverrides[filePath] = !original;
      } else {
        delete binOverrides[filePath];
      }
      return { ...prev, [binId]: binOverrides };
    });
  }, [state.subframeResults]);

  const applySelection = useCallback((binId: string) => {
    const result = state.subframeResults[binId];
    if (!result || subframeResultStale(result, thresholds)) return;
    const binOv = overrides[binId] ?? {};
    const rejected = result.subframes
      .filter((s) => {
        const ov = binOv[s.file_path];
        return ov !== undefined ? !ov : !s.accepted;
      })
      .map((s) => s.file_path);
    dispatch({ type: "SET_EXCLUDED_FILES", binId, files: rejected });
  }, [state.subframeResults, overrides, dispatch, thresholds]);

  const handleStack = useCallback(async (
    binId: string,
    batch: WizardStackRun["batch"] = null,
  ): Promise<StackBinOutcome> => {
    const latest = getState();
    const latestBin = latest.bins.find((b) => b.id === binId);
    const files = latestBin ? effectiveBinFiles(latest, latestBin).map(resolveEffectivePath) : [];
    if (files.length < 2) {
      setErrors((prev) => ({ ...prev, [binId]: `Need at least 2 files after exclusions, got ${files.length}` }));
      return "failed";
    }
    if (!claimStackProgress("compose")) {
      const line = stackProgressBusyLine(stackProgressHolder(), null);
      if (line) setErrors((prev) => ({ ...prev, [binId]: line }));
      return "failed";
    }
    lastStackToken += 1;
    const started: StackRunStart = { binId, token: lastStackToken, generation: getStackGeneration(), files };
    const discarded = (): boolean => {
      const current = getState();
      const bin = current.bins.find((b) => b.id === binId);
      const currentFiles = bin ? effectiveBinFiles(current, bin).map(resolveEffectivePath) : [];
      const outcome = stackRunFinish(started, { generation: getStackGeneration(), files: currentFiles });
      if (outcome.store) return false;
      recordStackOutcome(binId, outcome, started.generation);
      return true;
    };
    setStackRun({ binId, startedAt: Date.now(), batch, token: started.token, files });
    setErrors((prev) => ({ ...prev, [binId]: "" }));
    setCancelled((prev) => ({ ...prev, [binId]: false }));
    resetProgress();
    try {
      const subResult = latest.subframeResults[binId];
      let weights: number[] | undefined;
      if (subResult) {
        const byPath = new Map(subResult.subframes.map((s) => [resolveEffectivePath(s.file_path), s.weight]));
        weights = files.map((f) => byPath.get(f) ?? 1.0);
      }
      const name = wizardStackName(binId, useDrizzle, Date.now());
      const { rejection, sigmaLow, sigmaHigh } = rejectionSettings;
      const result = useDrizzle
        ? await drizzleFrames(files, await getOutputDir(), {
            scale: drizzleScale,
            pixfrac: drizzlePixfrac,
            kernel: drizzleKernel,
            align: true,
            name,
            rejection,
            ...(rejectionUsesSigma(rejection) ? { sigmaLow, sigmaHigh } : { sigmaLow: undefined, sigmaHigh: undefined }),
          })
        : await stackFrames(files, await getOutputDir(), {
            ...rejectionSettings,
            align: true,
            name,
            weights,
            combine,
            normalization,
          });
      if (discarded()) return "discarded";
      setResults((prev) => ({ ...prev, [binId]: result }));
      if (result.fits_path) {
        onStacked(binId, result.fits_path);
      }
      recordStackOutcome(binId, { store: true, notice: null }, started.generation);
      return "done";
    } catch (e) {
      if (discarded()) return "discarded";
      const msg = e instanceof Error ? e.message : String(e);
      if (/cancel/i.test(msg)) {
        setCancelled((prev) => ({ ...prev, [binId]: true }));
        return "cancelled";
      }
      console.error(`[AstroBurst] Stack failed for ${binId}:`, msg);
      setErrors((prev) => ({ ...prev, [binId]: msg }));
      return "failed";
    } finally {
      finishStackRun(started.token);
      resetProgress();
      releaseStackProgress("compose");
    }
  }, [onStacked, useDrizzle, drizzleScale, drizzlePixfrac, drizzleKernel, rejectionSettings, combine, normalization, resetProgress, setStackRun, finishStackRun, getStackGeneration, getState, recordStackOutcome]);

  const handleStackAll = useCallback(async () => {
    const bins = stackableBins.slice();
    const generation = getStackGeneration();
    for (let i = 0; i < bins.length; i++) {
      const batch = { current: i + 1, total: bins.length, label: bins[i].shortLabel };
      const outcome = await handleStack(bins[i].id, batch);
      if (stackAllNext(outcome, { generation, live: getStackGeneration() }) === "stop") break;
    }
  }, [stackableBins, handleStack, getStackGeneration]);

  useEffect(() => {
    if (!stackRun) return;
    const id = setInterval(() => setNow(Date.now()), ELAPSED_TICK_MS);
    return () => clearInterval(id);
  }, [stackRun]);

  const anyLoading = stackRun !== null;
  const elapsed = formatTime(stackRun ? Math.max(0, now - stackRun.startedAt) : 0);

  if (stackableBins.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-12 text-zinc-600 text-xs gap-2">
        <span>No channels have multiple files to stack.</span>
        {singleBins.length > 0 && (
          <span className="text-[10px] text-[var(--ab-text-4)]">
            {singleBins.length} channel(s) with single files will be used directly.
          </span>
        )}
      </div>
    );
  }

  const smallestBinCount = Math.min(...stackableBins.map((bin) => effectiveBinFiles(state, bin).length));

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center justify-between">
        <span className="text-xs text-zinc-400">
          {stackableBins.length} channel(s) to stack
        </span>
        <RunButton
          label="Stack All"
          runningLabel={stackRun?.batch ? `Stacking ${stackRun.batch.label} — ${stackRun.batch.current}/${stackRun.batch.total}` : "Stacking..."}
          running={anyLoading}
          disabled={busyLine !== null}
          describedBy={busyLine ? busyId : undefined}
          accent="blue"
          onClick={handleStackAll}
          small
        />
      </div>

      {busyLine && <p id={busyId} data-testid="compose-stack-progress-busy" className="text-[9px] text-amber-400/70">{busyLine}</p>}

      {anyLoading && (
        <div className="flex flex-col gap-1.5">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full bg-blue-500 transition-all duration-300" style={{ width: `${progress.percent}%` }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{stackProgressText(progress.stage, progress.current, progress.total)}</span>
            <span className="flex items-center gap-2">
              <span className="font-mono">{elapsed}</span>
              <button
                onClick={() => { cancelProgress(STACK_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel stacking"
                aria-label="Cancel stacking"
                className="text-zinc-500 hover:text-red-400 transition-colors"
              >
                <X size={11} />
              </button>
            </span>
          </div>
        </div>
      )}

      <div className="flex items-center gap-3">
        <Toggle label="Drizzle" checked={useDrizzle} accent="blue" onChange={setUseDrizzle} />
        {useDrizzle && (
          <div className="flex-1">
            <Slider label="Scale" value={drizzleScale} min={DRIZZLE_SCALE.min} max={DRIZZLE_SCALE.max} step={DRIZZLE_SCALE.step} accent="blue"
                    format={(v) => `${v.toFixed(1)}x`} onChange={setDrizzleScale} />
          </div>
        )}
      </div>

      {useDrizzle && (
        <div className="flex items-center gap-3">
          <div className="flex-1">
            <Slider label="Pixfrac" value={drizzlePixfrac} min={DRIZZLE_PIXFRAC.min} max={DRIZZLE_PIXFRAC.max} step={DRIZZLE_PIXFRAC.step} accent="blue"
                    format={(v) => v.toFixed(2)} onChange={setDrizzlePixfrac} />
          </div>
          <label htmlFor={kernelId} className="text-[10px] text-zinc-400 shrink-0">Kernel</label>
          <select id={kernelId} data-testid="stack-drizzle-kernel" value={drizzleKernel} onChange={(e) => setDrizzleKernel(e.target.value as DrizzleKernel)} className="ab-select">
            {DRIZZLE_KERNELS.map((k) => <option key={k.value} value={k.value}>{k.label}</option>)}
          </select>
        </div>
      )}

      <div className="flex flex-col gap-2">
        <RejectionControls
          settings={rejectionSettings}
          onChange={(change) => setRejectionSettings((prev) => ({ ...prev, ...change }))}
          frameCount={smallestBinCount}
          accent="blue"
          fixed={useDrizzle ? DRIZZLE_FIXED_REJECTION : undefined}
        />
        {!useDrizzle && (
          <>
            <div className="flex items-center justify-between gap-2">
              <label htmlFor={combineId} className="text-xs text-zinc-400">Combine</label>
              <select id={combineId} value={combine} onChange={(e) => setCombine(e.target.value as CombineMethod)} className="ab-select">
                {COMBINE_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
              </select>
            </div>
            <div className="flex items-center justify-between gap-2">
              <label htmlFor={normalizationId} className="text-xs text-zinc-400">Normalization</label>
              <select id={normalizationId} data-testid="stack-normalization" value={normalization} onChange={(e) => setNormalization(e.target.value as NormalizationMethod)} className="ab-select">
                {NORMALIZATION_OPTIONS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
              </select>
            </div>
          </>
        )}
      </div>

      <div className="flex flex-col gap-1.5 p-2 rounded-lg border border-zinc-800/40 bg-zinc-900/30">
        <span className="text-[9px] text-zinc-500 uppercase tracking-wider">
          Analyze thresholds (frames outside these are rejected)
        </span>
        <Slider label="Max FWHM" value={maxFwhm} min={1} max={20} step={0.1} accent="teal"
                format={(v) => `${v.toFixed(1)} px`} onChange={setMaxFwhm}
                hint="raise it for poor seeing" />
        <Slider label="Max Eccentricity" value={maxEcc} min={0} max={1} step={0.01} accent="teal"
                format={(v) => v.toFixed(2)} onChange={setMaxEcc}
                hint="1.00 accepts any star elongation" />
        <Slider label="Min SNR" value={minSnr} min={0} max={50} step={0.5} accent="teal"
                format={(v) => v.toFixed(1)} onChange={setMinSnr} />
        <Slider label="Min Stars" value={minStars} min={0} max={100} step={1} accent="teal"
                format={(v) => `${v}`} onChange={setMinStars} />
        <span className="text-[8px] text-zinc-600">
          Re-run Analyze after changing a threshold.
        </span>
      </div>

      {useDrizzle && Object.keys(state.subframeResults).length > 0 && (
        <p className="text-[9px] text-amber-400/70">
          Drizzle ignores subframe quality weights — they apply only to sigma-clip stacking.
        </p>
      )}
      {!useDrizzle && combine === "median" && Object.keys(state.subframeResults).length > 0 && (
        <p className="text-[9px] text-amber-400/70">
          Median combination ignores subframe quality weights.
        </p>
      )}

      {stackableBins.map((bin) => {
        const isLoading = stackRun?.binId === bin.id;
        const result = results[bin.id];
        const error = errors[bin.id];
        const discardNotice = stackDiscarded[bin.id];
        const isStacked = !!state.stackedPaths[bin.id] || !!result;
        const isAnalyzing = analyzing[bin.id];
        const analyzeError = analyzeErrors[bin.id];
        const subResult = state.subframeResults[bin.id];
        const subStale = subResult ? subframeResultStale(subResult, thresholds) : false;
        const staleLineId = `${subframeStaleId}-${bin.id}`;
        const binOv = overrides[bin.id] ?? {};
        const excluded = state.excludedFiles[bin.id] ?? [];
        const effectiveCount = effectiveBinFiles(state, bin).length;

        return (
          <div key={bin.id} className="flex flex-col gap-1.5 p-2 rounded-lg border"
               style={{
                 borderColor: isStacked ? `${bin.color}40` : "rgba(63,63,70,0.3)",
                 background: isStacked ? `${bin.color}08` : "rgba(24,24,27,0.3)",
               }}>
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-1.5">
                <span className="w-2 h-2 rounded-full" style={{ background: bin.color }} />
                <span className="text-[10px] font-medium text-zinc-300">{bin.shortLabel}</span>
                <span className="text-[9px] text-zinc-600">{effectiveCount}/{bin.files.length} frames</span>
              </div>
              <div className="flex items-center gap-1">
                <button
                  onClick={() => handleAnalyze(bin.id, bin.files)}
                  disabled={isAnalyzing}
                  className="flex items-center gap-1 px-1.5 py-0.5 rounded text-[9px] bg-teal-600/20 text-teal-400 hover:bg-teal-600/30 disabled:opacity-40 transition-all"
                >
                  {isAnalyzing ? <Loader2 size={9} className="animate-spin" /> : <BarChart3 size={9} />}
                  Analyze
                </button>
                <button
                  onClick={() => { void handleStack(bin.id); }}
                  disabled={anyLoading || busyLine !== null}
                  aria-describedby={busyLine ? busyId : undefined}
                  className="flex items-center gap-1 px-2 py-0.5 rounded text-[9px] bg-blue-600/20 text-blue-400 hover:bg-blue-600/30 disabled:opacity-40 transition-all"
                >
                  {isLoading ? <Loader2 size={9} className="animate-spin" /> : null}
                  {isLoading ? "Stacking..." : isStacked ? "Re-stack" : "Stack"}
                </button>
              </div>
            </div>

            {analyzeError && <div className="text-[9px] text-red-400">{analyzeError}</div>}

            {subResult && (
              <div className="flex flex-col gap-1">
                <div className="flex items-center justify-between text-[9px] text-zinc-500">
                  <span>{subResult.elapsed_ms}ms | {subResult.accepted} accepted, {subResult.rejected} rejected</span>
                  <button
                    type="button"
                    data-testid="stack-subframe-apply"
                    onClick={() => applySelection(bin.id)}
                    disabled={subStale}
                    aria-describedby={subStale ? staleLineId : undefined}
                    className="px-1.5 py-0.5 rounded text-[8px] bg-teal-600/20 text-teal-300 hover:bg-teal-600/30 transition-colors disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-teal-600/20"
                  >
                    Apply
                  </button>
                </div>
                <div id={staleLineId} role="status" data-testid="stack-subframe-stale" className="text-[9px] text-amber-300 empty:hidden">
                  {subStale ? SUBFRAME_STALE_NOTE : ""}
                </div>
                <div className={`overflow-x-auto rounded border border-zinc-800/50 max-h-32 overflow-y-auto transition-opacity ${subStale ? "opacity-60" : ""}`}>
                  <table className="w-full text-[8px]">
                    <thead>
                    <tr className="bg-zinc-900/50 text-zinc-500 uppercase tracking-wider sticky top-0">
                      <th className="px-1.5 py-1 w-5"></th>
                      <th className="px-1.5 py-1 text-left">File</th>
                      <th className="px-1.5 py-1 text-right">Stars</th>
                      <th className="px-1.5 py-1 text-right">FWHM</th>
                      <th className="px-1.5 py-1 text-right">SNR</th>
                      <th className="px-1.5 py-1 text-right">Wt</th>
                    </tr>
                    </thead>
                    <tbody>
                    {subResult.subframes.map((sub) => {
                      const ov = binOv[sub.file_path];
                      const accepted = ov !== undefined ? ov : sub.accepted;
                      return (
                        <tr
                          key={sub.file_path}
                          className={`border-t border-zinc-800/30 cursor-pointer hover:bg-zinc-800/30 ${accepted ? "" : "opacity-40"}`}
                          onClick={() => toggleOverride(bin.id, sub.file_path)}
                        >
                          <td className="px-1.5 py-0.5">
                            {accepted ? <Check size={8} className="text-emerald-400" /> : <X size={8} className="text-red-400" />}
                          </td>
                          <td className="px-1.5 py-0.5 text-zinc-300 font-mono truncate max-w-[80px]" title={sub.file_name}>
                            {sub.file_name}
                          </td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-400">{sub.star_count}</td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-300">{sub.median_fwhm.toFixed(2)}</td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-300">{sub.median_snr.toFixed(1)}</td>
                          <td className="px-1.5 py-0.5 text-right font-mono text-zinc-400">{(sub.weight * 100).toFixed(0)}%</td>
                        </tr>
                      );
                    })}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {excluded.length > 0 && !subResult && (
              <div className="text-[8px] text-amber-400/60">{excluded.length} file(s) excluded</div>
            )}

            {result && (
              <div className="text-[9px] text-zinc-500">
                {result.frame_count} frames, {formatCount(result.rejected_pixels ?? 0)} rejected px, {result.elapsed_ms ?? "?"}ms
              </div>
            )}
            {result && <WarningList warnings={result.warnings} />}
            {cancelled[bin.id] && <div className="text-[9px] text-zinc-500">Stacking cancelled.</div>}
            {error && <div className="text-[9px] text-red-400">{error}</div>}
            {discardNotice && (
              <span data-testid="stack-discarded-notice" className="text-[9px] text-amber-400">{discardNotice}</span>
            )}
          </div>
        );
      })}

      {singleBins.length > 0 && (
        <div className="text-[9px] text-zinc-600 pt-1 border-t border-zinc-800/30">
          {singleBins.map((b) => b.shortLabel).join(", ")} have single files (no stacking needed).
        </div>
      )}
    </div>
  );
}
