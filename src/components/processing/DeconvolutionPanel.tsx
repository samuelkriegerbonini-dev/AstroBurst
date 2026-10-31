import { useState, useCallback, useMemo, useId, useRef } from "react";
import { X } from "lucide-react";
import { recordDeconvolveRL } from "../../services/processing";
import { compositeDeconvolveRL } from "../../services/compositeChain";
import { cancelProgress } from "../../services/progress";
import { useProgress } from "../../hooks/useProgress";
import { INPUT_CHANGED_MESSAGE, bustPreviewUrl, isCancelMessage, useProcessingRun } from "../../hooks/useProcessingRun";
import { useCompositeChain } from "../../hooks/useCompositeChain";
import { useHistoryScope } from "../../hooks/useHistory";
import { useRerunInitial, useRerunUpdates } from "../../hooks/useRerunParams";
import { Slider, Toggle, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import MaskSelect from "../masks/MaskSelect";
import { useRunMask } from "../masks/runMask";
import type { ProcessedFile } from "../../shared/types";
import { useRenderContext } from "../../context/PreviewContext";
import { useCompositeStf } from "../../context/CompositeContext";
import { chainHoldsOutput } from "../../utils/processingChain";
import { COMPOSITE_RUN_KEY, compositeChainHolds } from "../../utils/compositeChain";
import { DECONV_PROGRESS_EVENT } from "../../shared/types/processing";
import { COMPOSITE_RESTARTED_NOTICE, channelTriple, compositeModeNotice, describedByIds } from "./compositeProps";
import type { ChannelTriple, CompositeDeconvolveResult, CompositePanelProps } from "./compositeProps";
import type { PsfSource } from "../../shared/types/compositeChain";
import {
  MEASURED_PSF_CHIP,
  PSF_CHOICES,
  deconvPanelSettingsOf,
  effectivePsfChoice,
  measuredPsfNote,
  offersMeasuredPsf,
  psfChoiceAvailable,
  psfChoiceHint,
  psfChoiceLabel,
  psfRequest,
  psfSourceLabel,
  type DeconvPanelSettings,
} from "./deconvPsf";

const RESULT_LABEL = "Deconvolved";

function enforceOdd(value: number): number {
  const v = Math.round(value);
  return v % 2 === 0 ? v + 1 : v;
}

function formatCount(v: number): string {
  return String(v);
}

function formatConvergence(v: number): string {
  return v.toExponential(2);
}

function compositeEarlyStop(iterationsRun: ChannelTriple, requested: number): boolean {
  return Math.min(...iterationsRun) < requested;
}

interface DeconvResult {
  previewUrl?: string;
  fits_path?: string;
  iterations_run?: number;
  convergence?: number;
  elapsed_ms?: number;
  psf_source?: PsfSource;
}

type DeconvParams = DeconvPanelSettings;

interface DeconvFileRun {
  composite: false;
  res: DeconvResult;
  resultUrl: string | undefined;
  baseUrl: string | null;
  baseLabel: string;
  requestedIterations: number;
}

interface DeconvCompositeRun {
  composite: true;
  res: CompositeDeconvolveResult;
  resultUrl: string | undefined;
  baseUrl: string | null;
  baseLabel: string;
  requestedIterations: number;
}

type DeconvRun = DeconvFileRun | DeconvCompositeRun;

interface DeconvolutionPanelProps extends CompositePanelProps {
  selectedFile: ProcessedFile | null;
  outputDir?: string;
  onProcessingDone?: (result: DeconvResult) => void | Promise<void>;
  chainedFrom?: string | null;
  psfKernel?: number[][] | null;
  inputPreviewUrl?: string | null;
  inputLabel?: string;
  fileKey?: string | null;
  disabledReason?: string | null;
  disabledReasonId?: string;
}

const ICON = (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-indigo-400">
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
  </svg>
);

export default function DeconvolutionPanel({
  selectedFile,
  outputDir = "./output",
  onProcessingDone,
  chainedFrom,
  psfKernel,
  inputPreviewUrl,
  inputLabel,
  fileKey,
  compositeMode,
  compositeInput,
  onCompositeDone,
  fileName,
  disabledReason,
  disabledReasonId,
  runNoteId,
}: DeconvolutionPanelProps) {
  const progress = useProgress(DECONV_PROGRESS_EVENT);
  const resetProgress = progress.reset;
  const initial = useRerunInitial("deconvolution", deconvPanelSettingsOf);
  const [params, setParams] = useState<DeconvParams>(
    () =>
      initial?.params ?? {
        iterations: 20,
        psfSigma: 2.0,
        psfSize: 15,
        regularization: 0.001,
        deringing: true,
        deringThreshold: 0.1,
        psf: "gaussian",
      },
  );
  useRerunUpdates("deconvolution", initial, deconvPanelSettingsOf, setParams);
  const psfGroupId = useId();
  const activePsf = effectivePsfChoice(params.psf, psfKernel);
  const psfNote = measuredPsfNote(params.psf, psfKernel);
  const scope = useHistoryScope();
  const runMask = useRunMask("deconvolution", scope, compositeMode);
  const maskReason = runMask.blockedReason;
  const { running: isRunning, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<DeconvRun>("deconv", compositeMode ? COMPOSITE_RUN_KEY : fileKey ?? null);
  const { chain } = useRenderContext();
  const compositeChain = useCompositeChain();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const displayStf = useMemo(
    () => ({ r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked }),
    [compositeStfR, compositeStfG, compositeStfB, compositeStfLinked],
  );
  const held = runResult
    ? runResult.composite
      ? compositeChainHolds(compositeChain, "deconv", runResult.resultUrl)
      : chainHoldsOutput(chain, "deconv", runResult.res.fits_path)
    : false;
  const result = held ? runResult : null;
  const canRun = compositeMode ? compositeInput !== null : !!selectedFile && !!scope;

  const update = useCallback(<K extends keyof DeconvParams>(key: K, value: DeconvParams[K]) => {
    setParams((prev) => ({ ...prev, [key]: value }));
  }, []);

  const measuredRadio = useRef<HTMLInputElement>(null);
  const chooseMeasuredPsf = useCallback(() => {
    update("psf", "measured");
    measuredRadio.current?.focus();
  }, [update]);

  const handleFileRun = useCallback(() => {
    if (!selectedFile?.path || !scope) return;
    const path = selectedFile.path;
    const runScope = scope;
    const baseUrl = inputPreviewUrl ?? null;
    const baseLabel = inputLabel ?? "Original";
    const requestedIterations = params.iterations;
    const mask = runMask.value;
    resetProgress();
    void run(async (ctx) => {
      const res = await recordDeconvolveRL(runScope, path, {
        iterations: params.iterations,
        psfSigma: params.psfSigma,
        psfSize: enforceOdd(params.psfSize),
        regularization: params.regularization,
        deringing: params.deringing,
        deringThreshold: params.deringThreshold,
        ...psfRequest(params.psf, psfKernel),
      }, mask);
      if (!ctx.inputUnchanged("deconv")) throw new Error(INPUT_CHANGED_MESSAGE);
      await onProcessingDone?.(res);
      return { composite: false as const, res, resultUrl: bustPreviewUrl(res?.previewUrl, Date.now()), baseUrl, baseLabel, requestedIterations };
    }, isCancelMessage).finally(resetProgress);
  }, [selectedFile, scope, params, psfKernel, runMask.value, resetProgress, run, inputPreviewUrl, inputLabel, onProcessingDone]);

  const handleCompositeRun = useCallback(() => {
    if (!compositeInput) return;
    const chainCall = { chainInput: compositeInput.input, displayStf, mask: runMask.value };
    const inputUrl = compositeInput.previewUrl;
    const inputLabelAtRun = compositeInput.label;
    const requestedIterations = params.iterations;
    resetProgress();
    void run(async () => {
      const res = await compositeDeconvolveRL(outputDir, chainCall, {
        iterations: params.iterations,
        psfSigma: params.psfSigma,
        psfSize: enforceOdd(params.psfSize),
        regularization: params.regularization,
        deringing: params.deringing,
        deringThreshold: params.deringThreshold,
        ...psfRequest(params.psf, psfKernel),
      });
      onCompositeDone("deconv", RESULT_LABEL, res, chainCall.displayStf);
      return {
        composite: true as const,
        res,
        resultUrl: bustPreviewUrl(res.previewUrl, Date.now()),
        baseUrl: res.basePreviewUrl ?? inputUrl,
        baseLabel: res.chain_restarted ? "Composite" : inputLabelAtRun,
        requestedIterations,
      };
    }, isCancelMessage).finally(resetProgress);
  }, [compositeInput, displayStf, runMask.value, outputDir, params, psfKernel, resetProgress, run, onCompositeDone]);

  const handleRun = compositeMode ? handleCompositeRun : handleFileRun;

  const earlyStop = result
    ? result.composite
      ? compositeEarlyStop(result.res.iterations_run, result.requestedIterations)
      : result.res.iterations_run != null && result.res.iterations_run < result.requestedIterations
    : false;

  return (
    <div className="flex flex-col gap-4 p-4 h-full overflow-y-auto">
      <SectionHeader icon={ICON} title="Richardson-Lucy Deconvolution" subtitle="FFT-accelerated" />
      <ChainBanner chainedFrom={chainedFrom} accent="indigo" />

      {!compositeMode && !selectedFile && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to enable deconvolution.</div>
      )}
      {compositeMode && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          {compositeModeNotice(fileName)}
        </div>
      )}

      <div className="flex flex-col gap-3">
        <Slider label="Iterations" value={params.iterations} min={1} max={200} step={1} disabled={isRunning} accent="indigo" onChange={(v) => update("iterations", v)} />

        <div className="flex flex-col gap-1.5">
          <span id={`${psfGroupId}-label`} className="text-xs text-zinc-400">PSF</span>
          <div
            role="radiogroup"
            aria-labelledby={`${psfGroupId}-label`}
            aria-describedby={psfNote ? `${psfGroupId}-hint ${psfGroupId}-note` : `${psfGroupId}-hint`}
            data-testid="deconv-psf"
            className="flex flex-col gap-1"
          >
            {PSF_CHOICES.map((choice) => {
              const selectable = !isRunning && psfChoiceAvailable(choice, psfKernel);
              return (
                <label key={choice} className={`flex items-center gap-2 text-xs ${selectable ? "text-zinc-300 cursor-pointer" : "text-zinc-600"}`}>
                  <input
                    ref={choice === "measured" ? measuredRadio : undefined}
                    type="radio"
                    name={psfGroupId}
                    value={choice}
                    checked={activePsf === choice}
                    disabled={!selectable}
                    data-testid={`deconv-psf-${choice}`}
                    onChange={() => update("psf", choice)}
                    className="accent-indigo-400"
                  />
                  {psfChoiceLabel(choice, psfKernel)}
                </label>
              );
            })}
          </div>
          <div id={`${psfGroupId}-hint`} className="text-[10px] text-zinc-500 px-0.5">{psfChoiceHint(activePsf)}</div>
          {offersMeasuredPsf(params.psf, psfKernel) && (
            <button
              type="button"
              data-testid="deconv-psf-chip"
              disabled={isRunning}
              onClick={chooseMeasuredPsf}
              className="self-start text-[10px] text-violet-200 bg-violet-900/30 border border-violet-700/40 rounded-full px-2.5 py-0.5 hover:bg-violet-900/50 disabled:opacity-40 transition-colors"
            >
              {MEASURED_PSF_CHIP}
            </button>
          )}
          {psfNote && (
            <div
              id={`${psfGroupId}-note`}
              role="note"
              className={activePsf === params.psf ? "text-[10px] text-zinc-500 px-0.5" : "text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded-lg px-3 py-1.5"}
            >
              {psfNote}
            </div>
          )}
        </div>

        {activePsf === "gaussian" && (
          <>
            <Slider label="PSF Sigma" value={params.psfSigma} min={0.5} max={10} step={0.1} disabled={isRunning} accent="indigo" format={(v) => v.toFixed(1)} onChange={(v) => update("psfSigma", v)} />
            <Slider label="PSF Size" value={params.psfSize} min={3} max={31} step={2} disabled={isRunning} accent="indigo" onChange={(v) => update("psfSize", v)} />
          </>
        )}

        <Slider label="Regularization" value={params.regularization} min={0} max={0.1} step={0.001} disabled={isRunning} accent="indigo" format={(v) => v.toFixed(3)} onChange={(v) => update("regularization", v)} />

        <Toggle label="Deringing" checked={params.deringing} disabled={isRunning} accent="indigo" onChange={(v) => update("deringing", v)} />

        {params.deringing && (
          <Slider label="Dering Threshold" value={params.deringThreshold} min={0} max={1} step={0.01} disabled={isRunning} accent="indigo" format={(v) => v.toFixed(2)} onChange={(v) => update("deringThreshold", v)} />
        )}
      </div>

      <MaskSelect {...runMask.select} busy={isRunning} />
      <div title={disabledReason ?? maskReason ?? busyTitle}>
        <RunButton label="Run Deconvolution" runningLabel="Deconvolving..." running={isRunning} disabled={!canRun || blocked || !!disabledReason || !!maskReason} describedBy={describedByIds(disabledReason ? disabledReasonId : null, runNoteId, maskReason ? runMask.select.noteId : null)} accent="indigo" onClick={handleRun} />
      </div>

      {isRunning && progress.active && (
        <div className="flex flex-col gap-1.5 animate-fade-in">
          <div className="w-full h-1.5 bg-zinc-800 rounded-full overflow-hidden">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${progress.percent}%`, background: "linear-gradient(90deg, var(--ab-indigo), #818cf8)" }} />
          </div>
          <div className="flex justify-between items-center text-[10px] text-zinc-500">
            <span>{progress.stage}</span>
            <span className="flex items-center gap-2">
              {progress.percent}%
              <button
                onClick={() => { cancelProgress(DECONV_PROGRESS_EVENT).catch(() => {}); }}
                title="Cancel deconvolution"
                aria-label="Cancel deconvolution"
                className="text-zinc-500 hover:text-red-400 transition-colors"
              >
                <X size={11} />
              </button>
            </span>
          </div>
        </div>
      )}

      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-3 animate-fade-in">
          {result.composite && result.res.chain_restarted && (
            <div className="text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25 rounded-lg px-3 py-1.5">
              {COMPOSITE_RESTARTED_NOTICE}
            </div>
          )}

          <ResultGrid items={[
            { label: "Iterations", value: result.composite ? channelTriple(result.res.iterations_run, formatCount) : result.res.iterations_run },
            { label: "Convergence", value: result.composite ? channelTriple(result.res.convergence, formatConvergence) : result.res.convergence?.toExponential(2) },
            { label: "PSF", value: psfSourceLabel(result.res.psf_source) },
            { label: "Time", value: `${((result.res.elapsed_ms ?? 0) / 1000).toFixed(1)}s` },
          ]} />

          {earlyStop && (
            <div className="text-[10px] text-emerald-400 bg-emerald-900/20 border border-emerald-800/30 rounded-lg px-3 py-1.5">
              Early stop: converged at iteration {result.composite ? channelTriple(result.res.iterations_run, formatCount) : result.res.iterations_run}/{result.requestedIterations}
            </div>
          )}

          {result.baseUrl && result.resultUrl && (
            <CompareView originalUrl={result.baseUrl} resultUrl={result.resultUrl} originalLabel={result.baseLabel} resultLabel={RESULT_LABEL} accent="indigo" />
          )}
        </div>
      )}
    </div>
  );
}
