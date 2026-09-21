import { lazy, Suspense, memo, useState, useCallback, useMemo, useRef, useEffect } from "react";
import { Loader2, ArrowRight, RotateCcw } from "lucide-react";
import { useFileContext, useRenderActions, useRgbContext } from "../../context/PreviewContext";
import { useCompositePreview, useCompositeStf, useCompositeScnr, useCompositeActions } from "../../context/CompositeContext";
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

export interface ProcessingChain {
  backgroundFits: string | null;
  denoiseFits: string | null;
  deconvFits: string | null;
  psfKernel: number[][] | null;
  stretchFits: string | null;
  maskedStretchFits: string | null;
  localContrastFits: string | null;
  pixelMathFits: string | null;
}

interface StepDoneResult {
  previewUrl?: string;
  corrected_fits?: string;
  fits_path?: string;
}

const CHAIN_FITS_KEYS = [
  "backgroundFits",
  "denoiseFits",
  "deconvFits",
  "stretchFits",
  "maskedStretchFits",
  "localContrastFits",
  "pixelMathFits",
] as const;

function normalizePath(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
}

function ChainIndicator({ chain, originalName }: { chain: ProcessingChain; originalName: string }) {
  const steps: string[] = [originalName];
  if (chain.backgroundFits) steps.push("BG");
  if (chain.denoiseFits) steps.push("Denoise");
  if (chain.psfKernel) steps.push("PSF");
  if (chain.deconvFits) steps.push("Deconv");
  if (chain.stretchFits) steps.push("Stretch");
  if (chain.maskedStretchFits) steps.push("Masked");
  if (chain.localContrastFits) steps.push("LHE/HDRMT");
  if (chain.pixelMathFits) steps.push("PixelMath");

  if (steps.length <= 1) return null;

  return (
    <div className="flex items-center gap-1 px-4 py-1.5 text-[10px] font-mono text-zinc-600 border-b border-zinc-800/30">
      {steps.map((s, i) => (
        <span key={i} className="flex items-center gap-1">
          {i > 0 && <ArrowRight size={8} className="text-zinc-700" />}
          <span className={i === steps.length - 1 ? "text-emerald-400/80" : "text-zinc-500"}>
            {s}
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
  const { setRenderedPreviewUrl, setProcessedSource } = useRenderActions();
  const { compositePreviewUrl } = useCompositePreview();
  const { setCompositePreviewUrl } = useCompositeActions();
  const { compositeStfR, compositeStfG, compositeStfB } = useCompositeStf();
  const { compositeScnr } = useCompositeScnr();
  const { rgbChannels } = useRgbContext();
  const [active, setActive] = useState<ProcessingSection>("background");

  const [chain, setChain] = useState<ProcessingChain>({
    backgroundFits: null,
    denoiseFits: null,
    deconvFits: null,
    psfKernel: null,
    stretchFits: null,
    maskedStretchFits: null,
    localContrastFits: null,
    pixelMathFits: null,
  });

  const [compositeSyncError, setCompositeSyncError] = useState<string | null>(null);
  const [resolvedDir, setResolvedDir] = useState("./output");
  useEffect(() => { getOutputDir().then(setResolvedDir); }, []);

  const compositeStfRef = useRef({ r: compositeStfR, g: compositeStfG, b: compositeStfB });
  useEffect(() => {
    compositeStfRef.current = { r: compositeStfR, g: compositeStfG, b: compositeStfB };
  }, [compositeStfR, compositeStfG, compositeStfB]);

  const compositeScnrRef = useRef(compositeScnr);
  useEffect(() => {
    compositeScnrRef.current = compositeScnr;
  }, [compositeScnr]);

  const findChannel = useCallback((filePath: string | undefined | null): string | null => {
    if (!filePath || !rgbChannels || !compositePreviewUrl) return null;
    const norm = (p: string) => p.replace(/\\/g, "/");
    const fp = norm(filePath);
    if (rgbChannels.r && norm(rgbChannels.r) === fp) return "r";
    if (rgbChannels.g && norm(rgbChannels.g) === fp) return "g";
    if (rgbChannels.b && norm(rgbChannels.b) === fp) return "b";
    return null;
  }, [rgbChannels, compositePreviewUrl]);

  const syncComposite = useCallback(async (fitsPath: string, channel: string) => {
    try {
      await updateCompositeChannel(channel, fitsPath);
      const stf = compositeStfRef.current;
      const scnr = compositeScnrRef.current;
      const dir = await getOutputDir();
      const result = await restretchComposite(dir, stf.r, stf.g, stf.b, scnr?.enabled ? scnr : undefined);
      if (result?.png_path) {
        const url = await getPreviewUrl(result.png_path);
        setCompositePreviewUrl(url);
      }
      setCompositeSyncError(null);
    } catch (e) {
      console.error("[AstroBurst] Composite channel sync failed:", e);
      setCompositeSyncError(e instanceof Error ? e.message : String(e));
    }
  }, [setCompositePreviewUrl]);

  const handlePreviewUpdate = useCallback(
    (url: string | null | undefined) => {
      if (!url) return;
      const bust = `${url}${url.includes("?") ? "&" : "?"}t=${Date.now()}`;
      setRenderedPreviewUrl(bust);
    },
    [setRenderedPreviewUrl],
  );

  const handleBackgroundDone = useCallback(
    (result: StepDoneResult) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.corrected_fits) {
        const fits = result.corrected_fits;
        setChain((prev) => ({
          ...prev,
          backgroundFits: fits,
          denoiseFits: null,
          deconvFits: null,
        }));
        setProcessedSource(fits);
        const ch = findChannel(file?.path);
        if (ch) syncComposite(fits, ch);
      }
    },
    [handlePreviewUpdate, file?.path, findChannel, syncComposite, setProcessedSource],
  );

  const handleDenoiseDone = useCallback(
    (result: StepDoneResult) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        const fits = result.fits_path;
        setChain((prev) => ({
          ...prev,
          denoiseFits: fits,
          deconvFits: null,
        }));
        setProcessedSource(fits);
        const ch = findChannel(file?.path);
        if (ch) syncComposite(fits, ch);
      }
    },
    [handlePreviewUpdate, file?.path, findChannel, syncComposite, setProcessedSource],
  );

  const handleDeconvDone = useCallback(
    (result: StepDoneResult) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        const fits = result.fits_path;
        setChain((prev) => ({
          ...prev,
          deconvFits: fits,
        }));
        setProcessedSource(fits);
        const ch = findChannel(file?.path);
        if (ch) syncComposite(fits, ch);
      }
    },
    [handlePreviewUpdate, file?.path, findChannel, syncComposite, setProcessedSource],
  );

  const handlePsfReady = useCallback((kernel: number[][]) => {
    setChain((prev) => ({ ...prev, psfKernel: kernel }));
  }, []);

  const handleStretchDone = useCallback(
    (result: StepDoneResult) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        const fits = result.fits_path;
        setChain((prev) => ({
          ...prev,
          stretchFits: fits,
          localContrastFits: null,
        }));
        setProcessedSource(fits);
        const ch = findChannel(file?.path);
        if (ch) syncComposite(fits, ch);
      }
    },
    [handlePreviewUpdate, file?.path, findChannel, syncComposite, setProcessedSource],
  );

  const handleMaskedStretchDone = useCallback(
    (result: StepDoneResult) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        const fits = result.fits_path;
        setChain((prev) => ({
          ...prev,
          maskedStretchFits: fits,
          localContrastFits: null,
        }));
        setProcessedSource(fits);
        const ch = findChannel(file?.path);
        if (ch) syncComposite(fits, ch);
      }
    },
    [handlePreviewUpdate, file?.path, findChannel, syncComposite, setProcessedSource],
  );

  const handleLocalContrastDone = useCallback(
    (result: StepDoneResult) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        const fits = result.fits_path;
        setChain((prev) => ({
          ...prev,
          localContrastFits: fits,
        }));
        setProcessedSource(fits);
        const ch = findChannel(file?.path);
        if (ch) syncComposite(fits, ch);
      }
    },
    [handlePreviewUpdate, file?.path, findChannel, syncComposite, setProcessedSource],
  );

  const handlePixelMathDone = useCallback(
    (result: { fits_path?: string; previewUrl?: string; cleaned_paths?: string[] }) => {
      if (!result?.fits_path) return;
      setProcessedSource(result.fits_path);
      const deleted = new Set((result.cleaned_paths ?? []).map(normalizePath));
      setChain((prev) => {
        const next: ProcessingChain = { ...prev, pixelMathFits: result.fits_path ?? null };
        if (deleted.size === 0) return next;
        for (const key of CHAIN_FITS_KEYS) {
          const value = next[key];
          if (value && deleted.has(normalizePath(value))) next[key] = null;
        }
        return next;
      });
    },
    [setProcessedSource],
  );

  const clearChain = useCallback(() => {
    setChain({ backgroundFits: null, denoiseFits: null, deconvFits: null, psfKernel: null, stretchFits: null, maskedStretchFits: null, localContrastFits: null, pixelMathFits: null });
  }, []);

  const handleResetChain = useCallback(() => {
    clearChain();
    setProcessedSource(null);
    setRenderedPreviewUrl(null);
  }, [clearChain, setProcessedSource, setRenderedPreviewUrl]);

  const chainFileIdRef = useRef<string | null>(null);
  useEffect(() => {
    if (chainFileIdRef.current === (file?.id ?? null)) return;
    chainFileIdRef.current = file?.id ?? null;
    clearChain();
  }, [file?.id, clearChain]);

  const backgroundInput = file;

  const denoiseInput = useMemo(() => {
    if (!file) return null;
    if (chain.backgroundFits) {
      return { ...file, path: chain.backgroundFits };
    }
    return file;
  }, [file, chain.backgroundFits]);

  const deconvInput = useMemo(() => {
    if (!file) return null;
    const path = chain.denoiseFits || chain.backgroundFits || file.path;
    return { ...file, path };
  }, [file, chain.denoiseFits, chain.backgroundFits]);

  const stretchInput = useMemo(() => {
    if (!file) return null;
    const path = chain.deconvFits || chain.denoiseFits || chain.backgroundFits || file.path;
    return { ...file, path };
  }, [file, chain.deconvFits, chain.denoiseFits, chain.backgroundFits]);

  const maskedStretchInput = useMemo(() => {
    if (!file) return null;
    const path = chain.deconvFits || chain.denoiseFits || chain.backgroundFits || file.path;
    return { ...file, path };
  }, [file, chain.deconvFits, chain.denoiseFits, chain.backgroundFits]);

  const nonLinearInput = useMemo(() => {
    if (!file) return null;
    const path = chain.localContrastFits || chain.maskedStretchFits || chain.stretchFits || file.path;
    return { ...file, path };
  }, [file, chain.localContrastFits, chain.maskedStretchFits, chain.stretchFits]);

  const nonLinearChainedFrom = chain.localContrastFits
    ? "LHE / HDRMT"
    : chain.maskedStretchFits
      ? "masked_stretch"
      : chain.stretchFits
        ? "stretch"
        : undefined;

  const pixelMathChainNotice = chain.localContrastFits
    ? "LHE / HDRMT"
    : chain.maskedStretchFits
      ? "masked stretch"
      : chain.stretchFits
        ? "stretch"
        : chain.deconvFits
          ? "deconvolution"
          : chain.denoiseFits
            ? "denoise"
            : chain.backgroundFits
              ? "background extraction"
              : undefined;

  const hasChain = chain.backgroundFits || chain.denoiseFits || chain.deconvFits || chain.psfKernel || chain.stretchFits || chain.maskedStretchFits || chain.localContrastFits || chain.pixelMathFits;

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1.5 px-3 pt-3 pb-1.5">
        <div className="flex gap-1 flex-1 flex-wrap">
          {SECTIONS.map((s) => {
            const isActive = active === s.id;
            const hasResult =
              (s.id === "background" && chain.backgroundFits) ||
              (s.id === "denoise" && chain.denoiseFits) ||
              (s.id === "psf" && chain.psfKernel) ||
              (s.id === "deconvolution" && chain.deconvFits) ||
              (s.id === "stretch" && chain.stretchFits) ||
              (s.id === "masked_stretch" && chain.maskedStretchFits) ||
              ((s.id === "local_contrast" || s.id === "hdr") && chain.localContrastFits) ||
              (s.id === "pixelmath" && chain.pixelMathFits);
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
        {hasChain && (
          <button
            onClick={handleResetChain}
            className="p-1.5 rounded-md text-zinc-600 hover:text-zinc-400 hover:bg-zinc-800/40 transition-all"
            title="Reset processing chain"
          >
            <RotateCcw size={13} />
          </button>
        )}
      </div>

      <ChainIndicator
        chain={chain}
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
              onPreviewUpdate={handlePreviewUpdate}
            />
          </div>
          <div style={{ display: active === "background" ? "block" : "none" }}>
            <BackgroundPanel
              selectedFile={backgroundInput}
              outputDir={resolvedDir}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleBackgroundDone}
              chainedFrom={undefined}
            />
          </div>
          <div style={{ display: active === "denoise" ? "block" : "none" }}>
            <WaveletPanel
              selectedFile={denoiseInput}
              outputDir={resolvedDir}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleDenoiseDone}
              chainedFrom={chain.backgroundFits ? "background" : undefined}
            />
          </div>
          <div style={{ display: active === "psf" ? "block" : "none" }}>
            <PsfPanel
              selectedFile={deconvInput}
              onPsfReady={handlePsfReady}
            />
          </div>
          <div style={{ display: active === "deconvolution" ? "block" : "none" }}>
            <DeconvolutionPanel
              selectedFile={deconvInput}
              outputDir={resolvedDir}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleDeconvDone}
              chainedFrom={
                chain.denoiseFits ? "denoise" : chain.backgroundFits ? "background" : undefined
              }
              psfKernel={chain.psfKernel}
            />
          </div>
          <div style={{ display: active === "stretch" ? "block" : "none" }}>
            <ArcsinhStretchPanel
              selectedFile={stretchInput}
              outputDir={resolvedDir}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleStretchDone}
              chainedFrom={
                chain.deconvFits ? "deconv" : chain.denoiseFits ? "denoise" : chain.backgroundFits ? "background" : undefined
              }
            />
          </div>
          <div style={{ display: active === "masked_stretch" ? "block" : "none" }}>
            <MaskedStretchPanel
              selectedFile={maskedStretchInput}
              outputDir={resolvedDir}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleMaskedStretchDone}
              chainedFrom={
                chain.deconvFits ? "deconv" : chain.denoiseFits ? "denoise" : chain.backgroundFits ? "background" : undefined
              }
            />
          </div>
          <div style={{ display: active === "local_contrast" ? "block" : "none" }}>
            <LocalContrastPanel
              selectedFile={nonLinearInput}
              outputDir={resolvedDir}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleLocalContrastDone}
              chainedFrom={nonLinearChainedFrom}
            />
          </div>
          <div style={{ display: active === "hdr" ? "block" : "none" }}>
            <HdrPanel
              selectedFile={nonLinearInput}
              outputDir={resolvedDir}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleLocalContrastDone}
              chainedFrom={nonLinearChainedFrom}
            />
          </div>
          <div style={{ display: active === "pixelmath" ? "block" : "none" }}>
            <PixelMathPanel
              selectedFile={file}
              outputDir={resolvedDir}
              chainedFrom={pixelMathChainNotice}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handlePixelMathDone}
            />
          </div>
        </div>
      </Suspense>
    </div>
  );
}

export default memo(ProcessingTabInner);
