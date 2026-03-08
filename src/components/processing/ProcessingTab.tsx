import { lazy, Suspense, memo, useState, useCallback, useMemo } from "react";
import { Loader2, ArrowRight, RotateCcw } from "lucide-react";
import { usePreviewContext } from "../../context/PreviewContext";

const DeconvolutionPanel = lazy(() => import("./DeconvolutionPanel"));
const BackgroundPanel = lazy(() => import("./BackgroundPanel"));
const WaveletPanel = lazy(() => import("./WaveletPanel"));

type ProcessingSection = "background" | "denoise" | "deconvolution";

const SECTIONS: { id: ProcessingSection; label: string; color: string }[] = [
  { id: "background", label: "Background", color: "emerald" },
  { id: "denoise", label: "Denoise", color: "sky" },
  { id: "deconvolution", label: "Deconvolution", color: "indigo" },
];

export interface ProcessingChain {
  backgroundFits: string | null;
  denoiseFits: string | null;
  deconvFits: string | null;
}

function ChainIndicator({ chain, originalName }: { chain: ProcessingChain; originalName: string }) {
  const steps: string[] = [originalName];
  if (chain.backgroundFits) steps.push("BG");
  if (chain.denoiseFits) steps.push("Denoise");
  if (chain.deconvFits) steps.push("Deconv");

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

function ProcessingTabInner() {
  const { file, setRenderedPreviewUrl } = usePreviewContext();
  const [active, setActive] = useState<ProcessingSection>("background");

  const [chain, setChain] = useState<ProcessingChain>({
    backgroundFits: null,
    denoiseFits: null,
    deconvFits: null,
  });

  const handlePreviewUpdate = useCallback(
    (url: string | null | undefined) => {
      if (!url) return;
      const bust = `${url}${url.includes("?") ? "&" : "?"}t=${Date.now()}`;
      setRenderedPreviewUrl(bust);
    },
    [setRenderedPreviewUrl],
  );

  const handleBackgroundDone = useCallback(
    (result: any) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.corrected_fits) {
        setChain((prev) => ({
          ...prev,
          backgroundFits: result.corrected_fits,
          denoiseFits: null,
          deconvFits: null,
        }));
      }
    },
    [handlePreviewUpdate],
  );

  const handleDenoiseDone = useCallback(
    (result: any) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        setChain((prev) => ({
          ...prev,
          denoiseFits: result.fits_path,
          deconvFits: null,
        }));
      }
    },
    [handlePreviewUpdate],
  );

  const handleDeconvDone = useCallback(
    (result: any) => {
      handlePreviewUpdate(result?.previewUrl);
      if (result?.fits_path) {
        setChain((prev) => ({
          ...prev,
          deconvFits: result.fits_path,
        }));
      }
    },
    [handlePreviewUpdate],
  );

  const handleResetChain = useCallback(() => {
    setChain({ backgroundFits: null, denoiseFits: null, deconvFits: null });
  }, []);

  const backgroundInput = file;

  const denoiseInput = useMemo(() => {
    if (!file) return null;
    if (chain.backgroundFits) {
      return {
        ...file,
        path: chain.backgroundFits,
        result: { ...file.result, previewUrl: file.result?.previewUrl },
      };
    }
    return file;
  }, [file, chain.backgroundFits]);

  const deconvInput = useMemo(() => {
    if (!file) return null;
    const path = chain.denoiseFits || chain.backgroundFits || file.path;
    return {
      ...file,
      path,
      result: { ...file.result, previewUrl: file.result?.previewUrl },
    };
  }, [file, chain.denoiseFits, chain.backgroundFits]);

  const hasChain = chain.backgroundFits || chain.denoiseFits || chain.deconvFits;

  return (
    <div className="flex flex-col h-full">
      <div className="flex items-center gap-1 px-4 pt-3 pb-1">
        <div className="flex gap-1 flex-1">
          {SECTIONS.map((s) => {
            const isActive = active === s.id;
            const hasResult =
              (s.id === "background" && chain.backgroundFits) ||
              (s.id === "denoise" && chain.denoiseFits) ||
              (s.id === "deconvolution" && chain.deconvFits);
            return (
              <button
                key={s.id}
                onClick={() => setActive(s.id)}
                className={`px-3 py-1.5 rounded-md text-xs font-medium transition-all duration-150 relative ${
                  isActive
                    ? s.color === "emerald"
                      ? "bg-emerald-600/20 text-emerald-400 ring-1 ring-emerald-500/30"
                      : s.color === "sky"
                        ? "bg-sky-600/20 text-sky-400 ring-1 ring-sky-500/30"
                        : "bg-indigo-600/20 text-indigo-400 ring-1 ring-indigo-500/30"
                    : "text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800/50"
                }`}
              >
                {s.label}
                {hasResult && (
                  <span className="absolute -top-1 -right-1 w-2 h-2 rounded-full bg-emerald-400" />
                )}
              </button>
            );
          })}
        </div>
        {hasChain && (
          <button
            onClick={handleResetChain}
            className="p-1 rounded text-zinc-600 hover:text-zinc-400 transition-colors"
            title="Reset processing chain"
          >
            <RotateCcw size={12} />
          </button>
        )}
      </div>

      <ChainIndicator
        chain={chain}
        originalName={file?.name?.split(/[/\\]/).pop()?.replace(/\.(fits?|asdf)$/i, "") || "original"}
      />

      <Suspense
        fallback={
          <div className="flex items-center justify-center py-12">
            <Loader2 size={20} className="animate-spin text-zinc-500" />
          </div>
        }
      >
        <div className="flex-1 overflow-y-auto">
          <div style={{ display: active === "background" ? "block" : "none" }}>
            <BackgroundPanel
              selectedFile={backgroundInput}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleBackgroundDone} chainedFrom={undefined}            />
          </div>
          <div style={{ display: active === "denoise" ? "block" : "none" }}>
            <WaveletPanel
              selectedFile={denoiseInput}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleDenoiseDone}
              chainedFrom={chain.backgroundFits ? "background" : undefined}
            />
          </div>
          <div style={{ display: active === "deconvolution" ? "block" : "none" }}>
            <DeconvolutionPanel
              selectedFile={deconvInput}
              onPreviewUpdate={handlePreviewUpdate}
              onProcessingDone={handleDeconvDone}
              chainedFrom={
                chain.denoiseFits ? "denoise" : chain.backgroundFits ? "background" : undefined
              }
            />
          </div>
        </div>
      </Suspense>
    </div>
  );
}

export default memo(ProcessingTabInner);
