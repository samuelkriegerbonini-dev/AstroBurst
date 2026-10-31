import { useState, useCallback, useEffect, useId } from "react";
import {
  GHS_DEFAULTS,
  arcsinhStretchComposite,
  recordArcsinhStretch,
  recordGhsStretch,
  stretchOptionsOf,
  type GhsSettings,
  type GhsStretchResult,
  type StretchSettings,
} from "../../services/processing";
import { useHistoryScope } from "../../hooks/useHistory";
import { useRerunInitial, useRerunUpdates } from "../../hooks/useRerunParams";
import type { ArcsinhResult } from "../../shared/types";
import { useRenderContext } from "../../context/PreviewContext";
import { useCompositeStf } from "../../context/CompositeContext";
import { useCompositeChain } from "../../hooks/useCompositeChain";
import { chainHoldsOutput } from "../../utils/processingChain";
import { COMPOSITE_RUN_KEY, compositeChainHolds } from "../../utils/compositeChain";
import { INPUT_CHANGED_MESSAGE, bustPreviewUrl, useProcessingRun } from "../../hooks/useProcessingRun";
import { opNameLabel } from "../../utils/historyModel";
import { COMPOSITE_RESTARTED_NOTICE, compositeModeNotice, describedByIds } from "./compositeProps";
import type { CompositePanelProps } from "./compositeProps";
import { Slider, RunButton, ResultGrid, CompareView, ChainBanner, ErrorAlert, SectionHeader } from "../ui";
import MaskSelect from "../masks/MaskSelect";
import { useRunMask } from "../masks/runMask";
import GhsStretchControls from "./GhsStretchControls";

type StretchMethod = StretchSettings["method"];

type StretchOutcome = { method: "arcsinh"; res: ArcsinhResult } | { method: "ghs"; res: GhsStretchResult };

type StretchRun = StretchOutcome & {
  resultUrl: string | undefined;
  baseUrl: string | null;
  baseLabel: string;
  chainRestarted: boolean;
};

interface ArcsinhStretchPanelProps extends CompositePanelProps {
  selectedFile: { path: string; result?: unknown } | null;
  outputDir?: string;
  onProcessingDone?: (result: ArcsinhResult) => void | Promise<void>;
  chainedFrom?: string;
  inputPreviewUrl?: string | null;
  inputLabel?: string;
  fileKey?: string | null;
  disabledReason?: string | null;
  disabledReasonId?: string;
}

const FACTOR_MIN = 1;
const FACTOR_MAX = 500;
const DEFAULT_FACTOR = 50.0;

const PRESETS = [5, 20, 50, 100, 200];

const METHODS: { id: StretchMethod; label: string }[] = [
  { id: "arcsinh", label: "Arcsinh" },
  { id: "ghs", label: "GHS" },
];

export const GHS_COMPOSITE_NOTICE = "GHS on the composite lives in Compose › Stretch.";

const SECTION_TITLES: Record<StretchMethod, { title: string; subtitle: string }> = {
  arcsinh: { title: "Arcsinh Stretch", subtitle: "arcsinh(I*S)/arcsinh(S)" },
  ghs: { title: "GHS Stretch", subtitle: "Generalized Hyperbolic Stretch" },
};

function runLabel(method: StretchMethod, factor: number, ghs: GhsSettings): string {
  return method === "ghs" ? `Apply GHS Stretch (D=${ghs.stretchFactor.toFixed(2)})` : `Apply Stretch (S=${factor.toFixed(0)})`;
}

function resultItems(run: StretchRun): { label: string; value: string | null | undefined }[] {
  const timing = [
    { label: "Time", value: run.res.elapsed_ms ? `${(run.res.elapsed_ms / 1000).toFixed(2)}s` : null },
    { label: "Size", value: run.res.dimensions ? `${run.res.dimensions[0]}x${run.res.dimensions[1]}` : null },
  ];
  if (run.method === "arcsinh") return [{ label: "Factor", value: run.res.stretch_factor?.toFixed(1) }, ...timing];
  return [
    { label: "D", value: run.res.stretch_factor?.toFixed(2) },
    { label: "b", value: run.res.local_intensity?.toFixed(1) },
    { label: "SP", value: run.res.symmetry_point?.toFixed(3) },
    ...timing,
  ];
}

const ICON = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-amber-400">
    <path d="M4 20 C8 4, 16 4, 20 20" />
  </svg>
);

export default function ArcsinhStretchPanel({ selectedFile, outputDir = "./output", onProcessingDone, chainedFrom, inputPreviewUrl, inputLabel, fileKey, compositeMode, compositeInput, onCompositeDone, fileName, disabledReason, disabledReasonId, runNoteId, onRunLabelChange }: ArcsinhStretchPanelProps) {
  const initial = useRerunInitial("stretch", stretchOptionsOf);
  const initialSettings = initial?.params ?? null;
  const [method, setMethod] = useState<StretchMethod>(initialSettings?.method ?? "arcsinh");
  const [factor, setFactor] = useState(initialSettings?.method === "arcsinh" ? initialSettings.factor : DEFAULT_FACTOR);
  const [ghs, setGhs] = useState<GhsSettings>(initialSettings?.method === "ghs" ? initialSettings.ghs : GHS_DEFAULTS);
  const applyRerun = useCallback((settings: StretchSettings) => {
    setMethod(settings.method);
    if (settings.method === "ghs") setGhs(settings.ghs);
    else setFactor(settings.factor);
  }, []);
  useRerunUpdates("stretch", initial, stretchOptionsOf, applyRerun);
  const activeMethod: StretchMethod = compositeMode ? "arcsinh" : method;
  useEffect(() => {
    onRunLabelChange?.(opNameLabel(activeMethod));
  }, [activeMethod, onRunLabelChange]);
  const compositeNoticeId = useId();
  const scope = useHistoryScope();
  const runMask = useRunMask("stretch", scope, compositeMode);
  const maskReason = runMask.blockedReason;
  const { running: isRunning, blocked, busyTitle, result: runResult, error, run } = useProcessingRun<StretchRun>("stretch", compositeMode ? COMPOSITE_RUN_KEY : (fileKey ?? null));
  const { chain } = useRenderContext();
  const compositeChain = useCompositeChain();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const holds = runResult !== null && (compositeMode
    ? compositeChainHolds(compositeChain, "stretch", runResult.resultUrl)
    : chainHoldsOutput(chain, "stretch", runResult.res.fits_path));
  const result = holds ? runResult : null;
  const canRun = compositeMode ? compositeInput !== null : !!selectedFile?.path && !!scope;

  const handleRun = useCallback(() => {
    const mask = runMask.value;
    if (compositeMode) {
      if (!compositeInput) return;
      const input = compositeInput;
      const displayStf = { r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked };
      void run(async () => {
        const res = await arcsinhStretchComposite(factor, outputDir, { chainInput: input.input, displayStf, mask });
        onCompositeDone("stretch", "Stretch", res, displayStf);
        return {
          method: "arcsinh",
          res,
          resultUrl: bustPreviewUrl(res.previewUrl, Date.now()),
          baseUrl: res.basePreviewUrl ?? input.previewUrl,
          baseLabel: res.basePreviewUrl ? "Composite" : input.label,
          chainRestarted: res.chain_restarted,
        };
      });
      return;
    }
    if (!selectedFile?.path || !scope) return;
    const path = selectedFile.path;
    const runScope = scope;
    const baseUrl = inputPreviewUrl ?? null;
    const baseLabel = inputLabel ?? "Original";
    void run(async (ctx) => {
      const outcome: StretchOutcome = method === "ghs"
        ? { method: "ghs", res: await recordGhsStretch(runScope, path, ghs, mask) }
        : { method: "arcsinh", res: await recordArcsinhStretch(runScope, path, factor, mask) };
      if (!ctx.inputUnchanged("stretch")) throw new Error(INPUT_CHANGED_MESSAGE);
      await onProcessingDone?.(outcome.res);
      return { ...outcome, resultUrl: bustPreviewUrl(outcome.res?.previewUrl, Date.now()), baseUrl, baseLabel, chainRestarted: false };
    });
  }, [compositeMode, compositeInput, compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, selectedFile?.path, scope, method, factor, ghs, runMask.value, outputDir, run, inputPreviewUrl, inputLabel, onProcessingDone, onCompositeDone]);

  const heading = SECTION_TITLES[activeMethod];

  return (
    <div className="flex flex-col gap-3 p-4">
      <SectionHeader icon={ICON} title={heading.title} subtitle={heading.subtitle} />
      <ChainBanner chainedFrom={chainedFrom} accent="amber" />

      {!selectedFile && !compositeMode && (
        <div className="text-xs text-zinc-500 italic px-1">Select a FITS file to apply stretch.</div>
      )}
      {compositeMode && (
        <div className="text-[10px] text-teal-300 bg-teal-900/20 border border-teal-800/30 rounded-lg px-3 py-1.5">
          {compositeModeNotice(fileName)}
        </div>
      )}

      <div className="flex flex-col gap-1">
        <div role="group" aria-label="Stretch method" className="flex gap-1.5">
          {METHODS.map(({ id, label }) => {
            const unavailable = compositeMode && id === "ghs";
            const active = activeMethod === id;
            return (
              <button
                key={id}
                type="button"
                data-stretch-method={id}
                aria-pressed={active}
                disabled={isRunning || unavailable}
                title={unavailable ? GHS_COMPOSITE_NOTICE : undefined}
                aria-describedby={unavailable ? compositeNoticeId : undefined}
                onClick={() => setMethod(id)}
                className={`flex-1 py-1 rounded-md text-[10px] font-medium transition-all duration-150 disabled:opacity-40 ${
                  active
                    ? "bg-amber-500/20 text-amber-300 ring-1 ring-amber-500/30"
                    : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
                }`}
              >
                {label}
              </button>
            );
          })}
        </div>
        {compositeMode && (
          <div id={compositeNoticeId} className="text-[10px] text-zinc-500 px-1">{GHS_COMPOSITE_NOTICE}</div>
        )}
      </div>

      {activeMethod === "ghs" ? (
        <GhsStretchControls settings={ghs} disabled={isRunning} onChange={setGhs} />
      ) : (
        <>
          <div className="flex flex-col gap-1">
            <Slider
              label="Stretch Factor (S)"
              value={factor}
              min={FACTOR_MIN}
              max={FACTOR_MAX}
              step={0.1}
              scale="log"
              disabled={isRunning}
              accent="amber"
              format={(v) => v.toFixed(1)}
              onChange={(v) => setFactor(Math.round(v * 10) / 10)}
            />
            <div className="flex justify-between text-[9px] text-zinc-600 font-mono px-0.5">
              <span>1.0 (linear)</span>
              <span>500 (strong)</span>
            </div>
          </div>

          <div className="flex gap-1.5">
            {PRESETS.map((preset) => (
              <button
                key={preset}
                onClick={() => setFactor(preset)}
                disabled={isRunning}
                className={`flex-1 py-1.5 rounded-md text-[10px] font-mono transition-all duration-150 ${
                  Math.abs(factor - preset) < 0.5
                    ? "bg-amber-500/20 text-amber-300 ring-1 ring-amber-500/30"
                    : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800"
                }`}
              >
                {preset}
              </button>
            ))}
          </div>
        </>
      )}

      <MaskSelect {...runMask.select} busy={isRunning} />
      <div title={disabledReason ?? maskReason ?? busyTitle}>
        <RunButton label={runLabel(activeMethod, factor, ghs)} runningLabel="Stretching..." running={isRunning} disabled={!canRun || blocked || !!disabledReason || !!maskReason} describedBy={describedByIds(disabledReason ? disabledReasonId : null, runNoteId, maskReason ? runMask.select.noteId : null)} accent="amber" onClick={handleRun} />
      </div>
      <ErrorAlert message={error} />

      {result && (
        <div className="flex flex-col gap-2 animate-fade-in">
          {result.chainRestarted && (
            <div className="text-[10px] text-amber-300 bg-amber-900/20 border border-amber-800/30 rounded-lg px-3 py-1.5">
              {COMPOSITE_RESTARTED_NOTICE}
            </div>
          )}
          <ResultGrid items={resultItems(result)} />

          {result.baseUrl && result.resultUrl && (
            <CompareView originalUrl={result.baseUrl} resultUrl={result.resultUrl} originalLabel={result.baseLabel} resultLabel="Stretched" accent="amber" height={180} />
          )}
        </div>
      )}
    </div>
  );
}
