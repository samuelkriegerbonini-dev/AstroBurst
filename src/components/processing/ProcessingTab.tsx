import { lazy, Suspense, memo, useState, useCallback, useMemo, useRef, useEffect } from "react";
import { Loader2, ArrowRight } from "lucide-react";
import { fileKeyOf, useFileContext, useRenderActions, useRenderContext, useRgbContext } from "../../context/PreviewContext";
import { useCompositePreview, useCompositeStf, useCompositeActions } from "../../context/CompositeContext";
import type { ChainEntry, ChainStep, ProcessedKind, ProcessingChain } from "../../shared/types/preview";
import { CHAIN_ORDER, inputFor, lastStep, samePath, withStep, withVersionParam } from "../../utils/processingChain";
import { compositeSyncStore, recordCompositeSync, wizardStepStaleAfterChannelSync, type CompositeChannel } from "../../utils/compositeSync";
import { useComposeWizardContext } from "../../context/ComposeWizardContext";
import { beginCompositeCheck } from "../../hooks/useProcessingRun";
import { toDims } from "../../utils/stackingOutputs";
import { updateCompositeChannel, restretchComposite } from "../../services/compose";
import { getPreviewUrl } from "../../infrastructure/tauri";
import { getOutputDir } from "../../infrastructure/tauri";

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

function ChainIndicator({ chain, displayedFits, originalName }: { chain: ProcessingChain; displayedFits: string | null; originalName: string }) {
  const steps: { label: string; current: boolean }[] = [{ label: originalName, current: false }];
  for (const s of CHAIN_ORDER) {
    if (s === "deconv" && chain.psfKernel) steps.push({ label: "PSF", current: false });
    const entry = chain.steps[s];
    if (!entry) continue;
    steps.push({ label: INDICATOR_LABELS[s], current: displayedFits !== null && samePath(entry.fitsPath, displayedFits) });
  }

  if (steps.length <= 1) return null;

  return (
    <div className="flex items-center gap-1 px-4 py-1.5 text-[10px] font-mono text-zinc-600 border-b border-zinc-800/30">
      {steps.map((s, i) => (
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

function ProcessingTabInner() {
  const { file } = useFileContext();
  const { chain, processed } = useRenderContext();
  const { publishProcessed, setChain, currentFileKey } = useRenderActions();
  const { compositePreviewUrl } = useCompositePreview();
  const { setCompositePreviewUrl } = useCompositeActions();
  const { compositeStfR, compositeStfG, compositeStfB, compositeStfLinked } = useCompositeStf();
  const { rgbChannels } = useRgbContext();
  const { state: wizardState, dispatch: wizardDispatch } = useComposeWizardContext();
  const wizardReadyRef = useRef(wizardState.compositeReady);
  wizardReadyRef.current = wizardState.compositeReady;
  const [active, setActive] = useState<ProcessingSection>("background");

  const [compositeSyncError, setCompositeSyncError] = useState<string | null>(null);
  const [resolvedDir, setResolvedDir] = useState("./output");
  useEffect(() => { getOutputDir().then(setResolvedDir); }, []);

  const compositeStfRef = useRef({ r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked });
  useEffect(() => {
    compositeStfRef.current = { r: compositeStfR, g: compositeStfG, b: compositeStfB, linked: compositeStfLinked };
  }, [compositeStfR, compositeStfG, compositeStfB, compositeStfLinked]);

  const liveCompositeRef = useRef({ rgbChannels, compositePreviewUrl });
  liveCompositeRef.current = { rgbChannels, compositePreviewUrl };

  const findLiveChannel = useCallback((filePath: string): CompositeChannel | null => {
    const { rgbChannels: channels, compositePreviewUrl: url } = liveCompositeRef.current;
    if (!channels || !url) return null;
    const norm = (p: string) => p.replace(/\\/g, "/");
    const fp = norm(filePath);
    if (channels.r && norm(channels.r) === fp) return "r";
    if (channels.g && norm(channels.g) === fp) return "g";
    if (channels.b && norm(channels.b) === fp) return "b";
    return null;
  }, []);

  const syncComposite = useCallback(async (fileKey: string, fitsPath: string, channel: CompositeChannel) => {
    const stillSameComposite = beginCompositeCheck(currentFileKey);
    try {
      await updateCompositeChannel(channel, fitsPath);
      const staleStep = wizardStepStaleAfterChannelSync(wizardReadyRef.current);
      if (staleStep) wizardDispatch({ type: "INVALIDATE_FROM", stepId: staleStep });
      const stf = compositeStfRef.current;
      const dir = await getOutputDir();
      const result = await restretchComposite(dir, stf.r, stf.g, stf.b, undefined, undefined, stf.linked);
      if (!stillSameComposite()) return;
      if (result?.png_path) {
        const url = compositeSyncStore.tagUrl(await getPreviewUrl(result.png_path));
        if (!stillSameComposite()) return;
        const liveUrl = liveCompositeRef.current.compositePreviewUrl;
        compositeSyncStore.set(recordCompositeSync(compositeSyncStore.get(), liveUrl, url, channel, fileKey));
        setCompositePreviewUrl(url);
      }
      setCompositeSyncError(null);
    } catch (e) {
      console.error("[AstroBurst] Composite channel sync failed:", e);
      if (stillSameComposite()) setCompositeSyncError(e instanceof Error ? e.message : String(e));
    }
  }, [setCompositePreviewUrl, currentFileKey, wizardDispatch]);

  const runKey = fileKeyOf(file);
  const runPath = file?.path ?? null;
  const compositeAtRun = compositePreviewUrl !== null;

  const inputs = useMemo(() => {
    const original = runPath ?? "";
    return {
      denoise: chainInput(chain, "denoise", original),
      deconv: chainInput(chain, "deconv", original),
      stretch: chainInput(chain, "stretch", original),
      localContrast: chainInput(chain, "localContrast", original),
    };
  }, [chain, runPath]);

  const makeStepDone = useCallback(
    (step: ChainStep, label: string, inputPath: string | null, actsOnComposite: boolean) =>
      (result: StepDoneResult) => {
        if (!runKey || !runPath || !result) return;
        if (actsOnComposite) return;
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
        };
        publishProcessed(runKey, { fitsPath: fits, previewUrl, dimensions, label, kind, inputPath: input }, (c) => withStep(c, step, entry));
        if (currentFileKey() !== runKey) return;
        const ch = findLiveChannel(runPath);
        if (ch) syncComposite(runKey, fits, ch);
      },
    [runKey, runPath, publishProcessed, currentFileKey, findLiveChannel, syncComposite],
  );

  const handleBackgroundDone = useMemo(
    () => makeStepDone("background", STEP_LABELS.background, runPath, false),
    [makeStepDone, runPath],
  );
  const handleDenoiseDone = useMemo(
    () => makeStepDone("denoise", STEP_LABELS.denoise, inputs.denoise.path, false),
    [makeStepDone, inputs.denoise.path],
  );
  const handleDeconvDone = useMemo(
    () => makeStepDone("deconv", STEP_LABELS.deconv, inputs.deconv.path, false),
    [makeStepDone, inputs.deconv.path],
  );
  const handleStretchDone = useMemo(
    () => makeStepDone("stretch", STEP_LABELS.stretch, inputs.stretch.path, false),
    [makeStepDone, inputs.stretch.path],
  );
  const handleMaskedStretchDone = useMemo(
    () => makeStepDone("maskedStretch", STEP_LABELS.maskedStretch, inputs.stretch.path, false),
    [makeStepDone, inputs.stretch.path],
  );
  const handleLheDone = useMemo(
    () => makeStepDone("localContrast", "LHE", inputs.localContrast.path, compositeAtRun),
    [makeStepDone, inputs.localContrast.path, compositeAtRun],
  );
  const handleHdrDone = useMemo(
    () => makeStepDone("localContrast", "HDRMT", inputs.localContrast.path, compositeAtRun),
    [makeStepDone, inputs.localContrast.path, compositeAtRun],
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

  const originalPreviewUrl = file?.result?.previewUrl ?? null;
  const inputPreviewOf = (input: ChainInput): string | null => input.entry?.previewUrl ?? (input.from ? null : originalPreviewUrl);
  const inputLabelOf = (input: ChainInput): string => (input.from ? STEP_LABELS[input.from] : "Original");
  const bannerOf = (input: ChainInput): string | undefined => (input.from ? BANNER_LABELS[input.from] : undefined);

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

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1.5 px-3 pt-3 pb-1.5">
        <div className="flex gap-1 flex-1 flex-wrap">
          {SECTIONS.map((s) => {
            const isActive = active === s.id;
            const step = SECTION_STEP[s.id];
            const hasResult = s.id === "psf" ? chain.psfKernel !== null : step ? chain.steps[step] !== undefined : false;
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

      <ChainIndicator
        chain={chain}
        displayedFits={displayedFits}
        originalName={file?.name?.split(/[/\\]/).pop()?.replace(/\.(fits?|asdf)$/i, "") || "original"}
      />

      {compositeSyncError && (
        <div className="flex items-center gap-2 mx-3 mb-1 px-2 py-1.5 rounded text-[10px] text-amber-300/90 bg-amber-900/15 border border-amber-700/25">
          <span className="flex-1 truncate" title={compositeSyncError}>
            Composite not updated: {compositeSyncError} — re-run Blend to sync
          </span>
          <button
            onClick={() => setCompositeSyncError(null)}
            className="shrink-0 text-amber-500/70 hover:text-amber-300 transition-colors"
            title="Dismiss"
          >
            ×
          </button>
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
            />
          </div>
          <div style={{ display: active === "background" ? "block" : "none" }}>
            <BackgroundPanel
              selectedFile={file}
              outputDir={resolvedDir}
              onProcessingDone={handleBackgroundDone}
              chainedFrom={undefined}
              fileKey={runKey}
            />
          </div>
          <div style={{ display: active === "denoise" ? "block" : "none" }}>
            <WaveletPanel
              selectedFile={denoiseInput}
              outputDir={resolvedDir}
              onProcessingDone={handleDenoiseDone}
              chainedFrom={bannerOf(inputs.denoise)}
              inputPreviewUrl={inputPreviewOf(inputs.denoise)}
              inputLabel={inputLabelOf(inputs.denoise)}
              fileKey={runKey}
            />
          </div>
          <div style={{ display: active === "psf" ? "block" : "none" }}>
            <PsfPanel
              selectedFile={deconvInput}
              onPsfReady={handlePsfReady}
              fileKey={runKey}
            />
          </div>
          <div style={{ display: active === "deconvolution" ? "block" : "none" }}>
            <DeconvolutionPanel
              selectedFile={deconvInput}
              outputDir={resolvedDir}
              onProcessingDone={handleDeconvDone}
              chainedFrom={bannerOf(inputs.deconv)}
              psfKernel={chain.psfKernel}
              inputPreviewUrl={inputPreviewOf(inputs.deconv)}
              inputLabel={inputLabelOf(inputs.deconv)}
              fileKey={runKey}
            />
          </div>
          <div style={{ display: active === "stretch" ? "block" : "none" }}>
            <ArcsinhStretchPanel
              selectedFile={stretchInput}
              outputDir={resolvedDir}
              onProcessingDone={handleStretchDone}
              chainedFrom={bannerOf(inputs.stretch)}
              inputPreviewUrl={inputPreviewOf(inputs.stretch)}
              inputLabel={inputLabelOf(inputs.stretch)}
              fileKey={runKey}
            />
          </div>
          <div style={{ display: active === "masked_stretch" ? "block" : "none" }}>
            <MaskedStretchPanel
              selectedFile={stretchInput}
              outputDir={resolvedDir}
              onProcessingDone={handleMaskedStretchDone}
              chainedFrom={bannerOf(inputs.stretch)}
              inputPreviewUrl={inputPreviewOf(inputs.stretch)}
              inputLabel={inputLabelOf(inputs.stretch)}
              fileKey={runKey}
            />
          </div>
          <div style={{ display: active === "local_contrast" ? "block" : "none" }}>
            <LocalContrastPanel
              selectedFile={nonLinearInput}
              outputDir={resolvedDir}
              onProcessingDone={handleLheDone}
              chainedFrom={bannerOf(inputs.localContrast)}
              inputPreviewUrl={inputPreviewOf(inputs.localContrast)}
              inputLabel={inputLabelOf(inputs.localContrast)}
              fileKey={runKey}
            />
          </div>
          <div style={{ display: active === "hdr" ? "block" : "none" }}>
            <HdrPanel
              selectedFile={nonLinearInput}
              outputDir={resolvedDir}
              onProcessingDone={handleHdrDone}
              chainedFrom={bannerOf(inputs.localContrast)}
              inputPreviewUrl={inputPreviewOf(inputs.localContrast)}
              inputLabel={inputLabelOf(inputs.localContrast)}
              fileKey={runKey}
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
            />
          </div>
        </div>
      </Suspense>
    </div>
  );
}

export default memo(ProcessingTabInner);
