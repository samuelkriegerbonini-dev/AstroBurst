import { lazy, Suspense, memo, useState, useCallback, useMemo, useRef, useEffect, useId } from "react";
import { Loader2, ArrowRight } from "lucide-react";
import { fileKeyOf, useFileContext, useRenderActions, useRenderContext } from "../../context/PreviewContext";
import { useCompositePreview, useCompositeActions } from "../../context/CompositeContext";
import type { ChainEntry, ChainStep, ProcessedKind, ProcessingChain } from "../../shared/types/preview";
import type {
  CompositeChain,
  CompositeChainEntry,
  CompositeDeconvolveResult,
  CompositeStepResult,
  DisplayStf,
  PsfSource,
} from "../../shared/types/compositeChain";
import { CHAIN_ORDER, inputFor, lastStep, psfUseOf, samePath, showsPsfCrumb, withStep, withVersionParam, type PsfUse } from "../../utils/processingChain";
import {
  compositeChainHolds,
  compositeInputFor,
  compositeInputLabel,
  compositeInputPreview,
  compositeStepOutcome,
  samePreview,
  withCompositePsfKernel,
  withCompositeStep,
} from "../../utils/compositeChain";
import { compositeChainStore } from "../../utils/compositeChainStore";
import { wizardStepStaleAfterCompositeWrite } from "../../utils/compositeSync";
import { useComposeWizardContext } from "../../context/ComposeWizardContext";
import { useCompositeMode } from "../../hooks/useCompositeMode";
import { useCompositeChain } from "../../hooks/useCompositeChain";
import { toDims } from "../../utils/stackingOutputs";
import { getOutputDir } from "../../infrastructure/tauri";
import type { CompositeInputView } from "./compositeProps";
import { rgbFitsDisabledReason } from "./rgbFitsNotice";

const DeconvolutionPanel = lazy(() => import("./DeconvolutionPanel"));
const BackgroundPanel = lazy(() => import("./BackgroundPanel"));
const WaveletPanel = lazy(() => import("./WaveletPanel"));
const PsfPanel = lazy(() => import("./PsfPanel"));
const ArcsinhStretchPanel = lazy(() => import("./ArcsinhStretchPanel"));
const MaskedStretchPanel = lazy(() => import("./MaskedStretchPanel"));
const DebayerPanel = lazy(() => import("./DebayerPanel"));
const PixelMathPanel = lazy(() => import("./PixelMathPanel"));
const LocalContrastPanel = lazy(() => import("./LocalContrastPanel"));
const HdrPanel = lazy(() => import("./HdrPanel"));

type ProcessingSection =
  | "debayer"
  | "background"
  | "denoise"
  | "psf"
  | "deconvolution"
  | "stretch"
  | "masked_stretch"
  | "local_contrast"
  | "hdr"
  | "pixelmath";

const SECTIONS: { id: ProcessingSection; label: string; color: string }[] = [
  { id: "debayer", label: "Debayer", color: "orange" },
  { id: "background", label: "Background", color: "emerald" },
  { id: "denoise", label: "Denoise", color: "sky" },
  { id: "psf", label: "PSF", color: "violet" },
  { id: "deconvolution", label: "Deconv", color: "indigo" },
  { id: "stretch", label: "Stretch", color: "amber" },
  { id: "masked_stretch", label: "Masked", color: "rose" },
  { id: "local_contrast", label: "LHE", color: "teal" },
  { id: "hdr", label: "HDRMT", color: "violet" },
  { id: "pixelmath", label: "PixelMath", color: "violet" },
];

const SECTION_STEP: Partial<Record<ProcessingSection, ChainStep>> = {
  background: "background",
  denoise: "denoise",
  deconvolution: "deconv",
  stretch: "stretch",
  masked_stretch: "maskedStretch",
  local_contrast: "localContrast",
  hdr: "localContrast",
  pixelmath: "pixelMath",
};

interface StepDoneResult {
  previewUrl?: string;
  corrected_fits?: string;
  fits_path?: string;
  dimensions?: number[];
  psf_source?: PsfSource;
}

type CompositePsfEntry = CompositeChainEntry & PsfUse;

function psfSourceOf(result: CompositeStepResult | CompositeDeconvolveResult): PsfSource | undefined {
  return "psf_source" in result ? result.psf_source : undefined;
}

interface PixelMathDoneResult {
  fits_path?: string;
  previewUrl?: string;
  dimensions?: number[];
}

const STEP_LABELS: Record<ChainStep, string> = {
  background: "Background",
  denoise: "Denoise",
  deconv: "Deconvolution",
  stretch: "Stretch",
  maskedStretch: "Masked stretch",
  localContrast: "LHE / HDRMT",
  pixelMath: "PixelMath",
};

const INDICATOR_LABELS: Record<ChainStep, string> = {
  background: "BG",
  denoise: "Denoise",
  deconv: "Deconv",
  stretch: "Stretch",
  maskedStretch: "Masked",
  localContrast: "LHE/HDRMT",
  pixelMath: "PixelMath",
};

const BANNER_LABELS: Record<ChainStep, string> = {
  background: "Background Extraction",
  denoise: "Wavelet Denoise",
  deconv: "Deconvolution",
  stretch: "Arcsinh Stretch",
  maskedStretch: "Masked Stretch",
  localContrast: "LHE / HDRMT",
  pixelMath: "PixelMath",
};

interface ChainInput {
  path: string;
  from: ChainStep | null;
  entry: ChainEntry | null;
}

function chainInput(chain: ProcessingChain, step: ChainStep, originalPath: string): ChainInput {
  const path = inputFor(chain, step, originalPath);
  for (let i = CHAIN_ORDER.length - 1; i >= 0; i--) {
    const s = CHAIN_ORDER[i];
    const entry = chain.steps[s];
    if (s !== step && entry && entry.fitsPath === path) return { path, from: s, entry };
  }
  return { path, from: null, entry: null };
}

interface Crumb {
  label: string;
  current: boolean;
}

function chainCrumbs(first: string, showPsf: boolean, currentOf: (step: ChainStep) => boolean | null): Crumb[] {
  const crumbs: Crumb[] = [{ label: first, current: false }];
  for (const s of CHAIN_ORDER) {
    if (s === "deconv" && showPsf) crumbs.push({ label: "PSF", current: false });
    const current = currentOf(s);
    if (current === null) continue;
    crumbs.push({ label: INDICATOR_LABELS[s], current });
  }
  return crumbs;
}

function fileCrumbs(chain: ProcessingChain, displayedFits: string | null, originalName: string): Crumb[] {
  return chainCrumbs(originalName, showsPsfCrumb(chain.psfKernel, chain.steps.deconv), (s) => {
    const entry = chain.steps[s];
    if (!entry) return null;
    return displayedFits !== null && samePath(entry.fitsPath, displayedFits);
  });
}

function compositeCrumbs(chain: CompositeChain, compositePreviewUrl: string | null): Crumb[] {
  const deconv: CompositePsfEntry | undefined = chain.steps.deconv;
  return chainCrumbs("Composite", showsPsfCrumb(chain.psfKernel, deconv), (s) => {
    const entry = chain.steps[s];
    if (!entry) return null;
    return compositePreviewUrl !== null && samePreview(entry.previewUrl, compositePreviewUrl);
  });
}

function ChainIndicator({ crumbs }: { crumbs: Crumb[] }) {
  if (crumbs.length <= 1) return null;

  return (
    <div className="flex items-center gap-1 px-4 py-1.5 text-[10px] font-mono text-zinc-600 border-b border-zinc-800/30">
      {crumbs.map((s, i) => (
        <span key={i} className="flex items-center gap-1">
          {i > 0 && <ArrowRight size={8} className="text-zinc-700" />}
          <span className={s.current ? "text-emerald-400/80" : "text-zinc-500"}>
            {s.label}
          </span>
        </span>
      ))}
    </div>
  );
}

const COLOR_MAP: Record<string, { active: string; dot: string }> = {
  orange: { active: "bg-orange-600/20 text-orange-400 ring-1 ring-orange-500/30", dot: "bg-orange-400" },
  emerald: { active: "bg-emerald-600/20 text-emerald-400 ring-1 ring-emerald-500/30", dot: "bg-emerald-400" },
  sky: { active: "bg-sky-600/20 text-sky-400 ring-1 ring-sky-500/30", dot: "bg-sky-400" },
  violet: { active: "bg-violet-600/20 text-violet-400 ring-1 ring-violet-500/30", dot: "bg-violet-400" },
  indigo: { active: "bg-indigo-600/20 text-indigo-400 ring-1 ring-indigo-500/30", dot: "bg-indigo-400" },
  amber: { active: "bg-amber-600/20 text-amber-400 ring-1 ring-amber-500/30", dot: "bg-amber-400" },
  rose: { active: "bg-rose-600/20 text-rose-400 ring-1 ring-rose-500/30", dot: "bg-rose-400" },
  teal: { active: "bg-teal-600/20 text-teal-400 ring-1 ring-teal-500/30", dot: "bg-teal-400" },
};

function compositeInputView(chain: CompositeChain, step: ChainStep, liveUrl: string | null): CompositeInputView {
  const input = compositeInputFor(chain, step);
  const entry = input === "base" ? undefined : chain.steps[input];
  const previewUrl = compositeInputPreview(chain, input) ?? (input === "base" ? liveUrl : null);
  return { input, previewUrl, label: entry?.label ?? compositeInputLabel(input) };
}

function compositeBannerOf(view: CompositeInputView): string | undefined {
  return view.input === "base" ? undefined : BANNER_LABELS[view.input];
}

function ProcessingTabInner() {
  const { file } = useFileContext();
  const { chain, processed } = useRenderContext();
  const { publishProcessed, setChain } = useRenderActions();
  const { compositePreviewUrl } = useCompositePreview();
  const { setCompositePreviewUrl, setCompositeStf, setCompositeAutoStf, setCompositeStfLinked, replaceParked } = useCompositeActions();
  const { state: wizardState, dispatch: wizardDispatch } = useComposeWizardContext();
  const wizardReadyRef = useRef(wizardState.compositeReady);
  wizardReadyRef.current = wizardState.compositeReady;
  const compositeMode = useCompositeMode();
  const compositeModeRef = useRef(compositeMode);
  compositeModeRef.current = compositeMode;
  const compositeChain = useCompositeChain();
  const [active, setActive] = useState<ProcessingSection>("background");
  const disabledReason = rgbFitsDisabledReason({ fileIsRgb: !!file?.result?.is_rgb, compositeMode });
  const disabledReasonId = useId();

  const [resolvedDir, setResolvedDir] = useState("./output");
  useEffect(() => { getOutputDir().then(setResolvedDir); }, []);

  const runKey = fileKeyOf(file);
  const runPath = file?.path ?? null;
  const fileName = file?.name ?? "";

  const inputs = useMemo(() => {
    const original = runPath ?? "";
    return {
      denoise: chainInput(chain, "denoise", original),
      deconv: chainInput(chain, "deconv", original),
      stretch: chainInput(chain, "stretch", original),
      localContrast: chainInput(chain, "localContrast", original),
    };
  }, [chain, runPath]);

  const compositeInputs = useMemo(() => {
    if (!compositeMode) return null;
    const view = (step: ChainStep) => compositeInputView(compositeChain, step, compositePreviewUrl);
    return {
      background: view("background"),
      denoise: view("denoise"),
      deconv: view("deconv"),
      stretch: view("stretch"),
      localContrast: view("localContrast"),
      pixelMath: view("pixelMath"),
    };
  }, [compositeMode, compositeChain, compositePreviewUrl]);

  const makeStepDone = useCallback(
    (step: ChainStep, label: string, inputPath: string | null) =>
      (result: StepDoneResult) => {
        if (!runKey || !runPath || !result) return;
        const fits = (step === "background" ? result.corrected_fits : result.fits_path) ?? null;
        const previewUrl = result.previewUrl ?? null;
        const dimensions = toDims(result.dimensions);
        const kind: ProcessedKind = "processing";
        const input = inputPath ?? runPath;
        if (!fits) {
          if (previewUrl) publishProcessed(runKey, { fitsPath: null, previewUrl, dimensions: null, label, kind, inputPath: input });
          return;
        }
        const entry: ChainEntry = {
          fitsPath: fits,
          previewUrl: previewUrl ? withVersionParam(previewUrl, Date.now()) : null,
          dimensions,
          ...psfUseOf(step, result.psf_source),
        };
        publishProcessed(runKey, { fitsPath: fits, previewUrl, dimensions, label, kind, inputPath: input }, (c) => withStep(c, step, entry));
      },
    [runKey, runPath, publishProcessed],
  );

  const handleBackgroundDone = useMemo(
    () => makeStepDone("background", STEP_LABELS.background, runPath),
    [makeStepDone, runPath],
  );
  const handleDenoiseDone = useMemo(
    () => makeStepDone("denoise", STEP_LABELS.denoise, inputs.denoise.path),
    [makeStepDone, inputs.denoise.path],
  );
  const handleDeconvDone = useMemo(
    () => makeStepDone("deconv", STEP_LABELS.deconv, inputs.deconv.path),
    [makeStepDone, inputs.deconv.path],
  );
  const handleStretchDone = useMemo(
    () => makeStepDone("stretch", STEP_LABELS.stretch, inputs.stretch.path),
    [makeStepDone, inputs.stretch.path],
  );
  const handleMaskedStretchDone = useMemo(
    () => makeStepDone("maskedStretch", STEP_LABELS.maskedStretch, inputs.stretch.path),
    [makeStepDone, inputs.stretch.path],
  );
  const handleLheDone = useMemo(
    () => makeStepDone("localContrast", "LHE", inputs.localContrast.path),
    [makeStepDone, inputs.localContrast.path],
  );
  const handleHdrDone = useMemo(
    () => makeStepDone("localContrast", "HDRMT", inputs.localContrast.path),
    [makeStepDone, inputs.localContrast.path],
  );

  const handleCompositeDone = useCallback(
    (step: ChainStep, label: string, result: CompositeStepResult, callStf: DisplayStf) => {
      const previewUrl = result.previewUrl;
      if (!previewUrl) return;
      const entry: CompositePsfEntry = {
        previewUrl,
        label,
        displayed: result.displayed,
        modelPreviewUrl: result.modelPreviewUrl ?? null,
        ...psfUseOf(step, psfSourceOf(result)),
      };
      compositeChainStore.update((c) => withCompositeStep(c, step, entry, result, callStf));
      const outcome = compositeStepOutcome({
        step,
        displayed: result.displayed,
        accepted: compositeChainHolds(compositeChainStore.get(), step, previewUrl),
        compositeMode: compositeModeRef.current,
      });
      if (outcome.invalidatesWizard) {
        const staleStep = wizardStepStaleAfterCompositeWrite(wizardReadyRef.current);
        if (staleStep) wizardDispatch({ type: "INVALIDATE_FROM", stepId: staleStep });
      }
      if (outcome.target === "parked") replaceParked(previewUrl, result.stf);
      if (outcome.target !== "screen") return;
      const stf = result.stf;
      if (stf) {
        setCompositeAutoStf(stf.r, stf.g, stf.b);
        setCompositeStf(stf.r, stf.g, stf.b);
        setCompositeStfLinked(stf.linked);
      }
      setCompositePreviewUrl(previewUrl);
    },
    [setCompositeAutoStf, setCompositeStf, setCompositeStfLinked, setCompositePreviewUrl, replaceParked, wizardDispatch],
  );

  const displayedPath = processed?.fitsPath ?? runPath;
  const handlePixelMathDone = useCallback(
    (result: PixelMathDoneResult) => {
      if (!runKey || !runPath || !result?.fits_path) return;
      const fits = result.fits_path;
      const previewUrl = result.previewUrl ?? null;
      const dimensions = toDims(result.dimensions);
      const entry: ChainEntry = {
        fitsPath: fits,
        previewUrl: previewUrl ? withVersionParam(previewUrl, Date.now()) : null,
        dimensions,
      };
      publishProcessed(
        runKey,
        { fitsPath: fits, previewUrl, dimensions, label: STEP_LABELS.pixelMath, kind: "pixelmath", inputPath: displayedPath ?? runPath },
        (c) => withStep(c, "pixelMath", entry),
      );
    },
    [runKey, runPath, displayedPath, publishProcessed],
  );

  const handleDebayerPreview = useCallback(
    (url: string | null | undefined) => {
      if (!runKey || !runPath || !url) return;
      publishProcessed(runKey, { fitsPath: null, previewUrl: url, dimensions: null, label: "Debayer", kind: "debayer", inputPath: runPath });
    },
    [runKey, runPath, publishProcessed],
  );

  const handlePsfReady = useCallback(
    (kernel: number[][]) => {
      if (!runKey) return;
      setChain(runKey, (c) => ({ ...c, psfKernel: kernel }));
    },
    [runKey, setChain],
  );

  const handleCompositePsfReady = useCallback((kernel: number[][], liveGeneration: number) => {
    compositeChainStore.update((c) => withCompositePsfKernel(c, kernel, liveGeneration));
  }, []);

  const originalPreviewUrl = file?.result?.previewUrl ?? null;
  const inputPreviewOf = (input: ChainInput): string | null => input.entry?.previewUrl ?? (input.from ? null : originalPreviewUrl);
  const inputLabelOf = (input: ChainInput): string => (input.from ? STEP_LABELS[input.from] : "Original");
  const bannerOf = (input: ChainInput): string | undefined => (input.from ? BANNER_LABELS[input.from] : undefined);
  const bannerFor = (fileInput: ChainInput, compositeInput: CompositeInputView | undefined): string | undefined =>
    compositeInput ? compositeBannerOf(compositeInput) : bannerOf(fileInput);

  const withPath = useCallback(
    (path: string) => (file ? (path === file.path ? file : { ...file, path }) : null),
    [file],
  );
  const denoiseInput = useMemo(() => withPath(inputs.denoise.path), [withPath, inputs.denoise.path]);
  const deconvInput = useMemo(() => withPath(inputs.deconv.path), [withPath, inputs.deconv.path]);
  const stretchInput = useMemo(() => withPath(inputs.stretch.path), [withPath, inputs.stretch.path]);
  const nonLinearInput = useMemo(() => withPath(inputs.localContrast.path), [withPath, inputs.localContrast.path]);

  const latest = lastStep(chain);
  const latestEntry = latest ? chain.steps[latest] : undefined;
  const pixelMathChainNotice =
    latest && latestEntry && !(processed?.fitsPath && samePath(processed.fitsPath, latestEntry.fitsPath))
      ? BANNER_LABELS[latest].toLowerCase()
      : undefined;

  const displayedFits = processed?.fitsPath ?? null;
  const originalName = file?.name?.split(/[/\\]/).pop()?.replace(/\.(fits?|asdf)$/i, "") || "original";
  const crumbs = compositeMode ? compositeCrumbs(compositeChain, compositePreviewUrl) : fileCrumbs(chain, displayedFits, originalName);
  const activeSteps: Partial<Record<ChainStep, unknown>> = compositeMode ? compositeChain.steps : chain.steps;
  const activePsfKernel = compositeMode ? compositeChain.psfKernel : chain.psfKernel;

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1.5 px-3 pt-3 pb-1.5">
        <div className="flex gap-1 flex-1 flex-wrap">
          {SECTIONS.map((s) => {
            const isActive = active === s.id;
            const step = SECTION_STEP[s.id];
            const hasResult = s.id === "psf" ? activePsfKernel !== null : step ? activeSteps[step] !== undefined : false;
            const colors = COLOR_MAP[s.color];
            return (
              <button
                key={s.id}
                onClick={() => setActive(s.id)}
                className={`ab-processing-pill ${isActive ? colors.active : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"}`}
                title={`${s.label} processing step`}
              >
                {s.label}
                {hasResult && (
                  <span className={`ab-processing-pill-dot ${colors.dot}`} />
                )}
              </button>
            );
          })}
        </div>
      </div>

      <ChainIndicator crumbs={crumbs} />

      {disabledReason && (
        <div id={disabledReasonId} role="note" className="mx-3 mb-1 px-2 py-1.5 rounded text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25">
          {disabledReason}
        </div>
      )}

      <Suspense
        fallback={
          <div className="flex items-center justify-center py-12">
            <Loader2 size={20} className="animate-spin text-zinc-500" />
          </div>
        }
      >
        <div className="flex-1 overflow-y-auto">
          <div style={{ display: active === "debayer" ? "block" : "none" }}>
            <DebayerPanel
              selectedFile={file}
              outputDir={resolvedDir}
              onPreviewUpdate={handleDebayerPreview}
              fileKey={runKey}
              compositeMode={compositeMode}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "background" ? "block" : "none" }}>
            <BackgroundPanel
              selectedFile={file}
              outputDir={resolvedDir}
              onProcessingDone={handleBackgroundDone}
              chainedFrom={undefined}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.background ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "denoise" ? "block" : "none" }}>
            <WaveletPanel
              selectedFile={denoiseInput}
              outputDir={resolvedDir}
              onProcessingDone={handleDenoiseDone}
              chainedFrom={bannerFor(inputs.denoise, compositeInputs?.denoise)}
              inputPreviewUrl={inputPreviewOf(inputs.denoise)}
              inputLabel={inputLabelOf(inputs.denoise)}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.denoise ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "psf" ? "block" : "none" }}>
            <PsfPanel
              selectedFile={deconvInput}
              onPsfReady={handlePsfReady}
              onCompositePsfReady={handleCompositePsfReady}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.deconv ?? null}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "deconvolution" ? "block" : "none" }}>
            <DeconvolutionPanel
              selectedFile={deconvInput}
              outputDir={resolvedDir}
              onProcessingDone={handleDeconvDone}
              chainedFrom={bannerFor(inputs.deconv, compositeInputs?.deconv)}
              psfKernel={activePsfKernel}
              inputPreviewUrl={inputPreviewOf(inputs.deconv)}
              inputLabel={inputLabelOf(inputs.deconv)}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.deconv ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "stretch" ? "block" : "none" }}>
            <ArcsinhStretchPanel
              selectedFile={stretchInput}
              outputDir={resolvedDir}
              onProcessingDone={handleStretchDone}
              chainedFrom={bannerFor(inputs.stretch, compositeInputs?.stretch)}
              inputPreviewUrl={inputPreviewOf(inputs.stretch)}
              inputLabel={inputLabelOf(inputs.stretch)}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.stretch ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "masked_stretch" ? "block" : "none" }}>
            <MaskedStretchPanel
              selectedFile={stretchInput}
              outputDir={resolvedDir}
              onProcessingDone={handleMaskedStretchDone}
              chainedFrom={bannerFor(inputs.stretch, compositeInputs?.stretch)}
              inputPreviewUrl={inputPreviewOf(inputs.stretch)}
              inputLabel={inputLabelOf(inputs.stretch)}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.stretch ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "local_contrast" ? "block" : "none" }}>
            <LocalContrastPanel
              selectedFile={nonLinearInput}
              outputDir={resolvedDir}
              onProcessingDone={handleLheDone}
              chainedFrom={bannerFor(inputs.localContrast, compositeInputs?.localContrast)}
              inputPreviewUrl={inputPreviewOf(inputs.localContrast)}
              inputLabel={inputLabelOf(inputs.localContrast)}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.localContrast ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "hdr" ? "block" : "none" }}>
            <HdrPanel
              selectedFile={nonLinearInput}
              outputDir={resolvedDir}
              onProcessingDone={handleHdrDone}
              chainedFrom={bannerFor(inputs.localContrast, compositeInputs?.localContrast)}
              inputPreviewUrl={inputPreviewOf(inputs.localContrast)}
              inputLabel={inputLabelOf(inputs.localContrast)}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.localContrast ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
          <div style={{ display: active === "pixelmath" ? "block" : "none" }}>
            <PixelMathPanel
              selectedFile={file}
              outputDir={resolvedDir}
              chainedFrom={pixelMathChainNotice}
              onProcessingDone={handlePixelMathDone}
              inputPreviewUrl={processed?.previewUrl ?? originalPreviewUrl}
              inputLabel={processed?.label ?? "Original"}
              fileKey={runKey}
              compositeMode={compositeMode}
              compositeInput={compositeInputs?.pixelMath ?? null}
              onCompositeDone={handleCompositeDone}
              fileName={fileName}
              disabledReason={disabledReason}
              disabledReasonId={disabledReasonId}
            />
          </div>
        </div>
      </Suspense>
    </div>
  );
}

export default memo(ProcessingTabInner);
