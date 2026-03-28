import { useState, useCallback, useRef, useEffect, useMemo, memo, lazy, Suspense } from "react";
import { Image, Loader2, X, Link, Unlink, RotateCcw, ChevronDown, ChevronUp } from "lucide-react";
import { useFileContext, useHistContext, useCubeContext, useRenderContext } from "../../context/PreviewContext";
import { restretchComposite } from "../../services/compose.service";
import { getPreviewUrl } from "../../infrastructure/tauri/client";
import { Slider } from "../ui";
import type { RawPixelData, StfParams } from "../../shared/types";

const GpuRenderer = lazy(() => import("../render/GpuRenderer"));

interface PreviewTabProps {
  useGpu: boolean;
  rawPixels: RawPixelData | null;
  onImageClick: (e: React.MouseEvent<HTMLImageElement>) => void;
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
}

const MAX_RETRIES = 2;
const RETRY_DELAYS = [300, 800] as const;
const DEBOUNCE_MS = 300;

const Overlay = memo(function Overlay({
                                        starOverlayRef,
                                        isCube,
                                      }: {
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
  isCube: boolean;
}) {
  return (
    <>
      <canvas
        ref={starOverlayRef}
        className="absolute inset-0 w-full h-full pointer-events-none"
        style={{ display: "none" }}
      />
      {isCube && (
        <div className="absolute bottom-2 right-2 bg-black/60 backdrop-blur-sm text-[10px] text-purple-300 px-2 py-1 rounded">
          Click to extract spectrum
        </div>
      )}
    </>
  );
});

function CompositeStfPanel() {
  const {
    compositeStfR, compositeStfG, compositeStfB, setCompositeStf,
    compositeStfLinked, setCompositeStfLinked,
    compositeAutoStfR, compositeAutoStfG, compositeAutoStfB,
    setCompositePreviewUrl,
  } = useRenderContext();

  const [expanded, setExpanded] = useState(true);
  const [restretching, setRestretching] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const seqRef = useRef(0);

  const scheduleRestretch = useCallback((r: StfParams, g: StfParams, b: StfParams) => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      const seq = ++seqRef.current;
      setRestretching(true);
      restretchComposite("./output", r, g, b)
        .then(async (result: any) => {
          if (seqRef.current !== seq) return;
          if (result?.png_path) {
            const base = await getPreviewUrl(result.png_path);
            const sep = base.includes("?") ? "&" : "?";
            setCompositePreviewUrl(`${base}${sep}t=${Date.now()}`);
          }
        })
        .catch((err: any) => {
          if (seqRef.current !== seq) return;
          console.error("[AstroBurst] Restretch failed:", err);
        })
        .finally(() => {
          if (seqRef.current !== seq) return;
          setRestretching(false);
        });
    }, DEBOUNCE_MS);
  }, [setCompositePreviewUrl]);

  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, []);

  const handleShadowR = useCallback((v: number) => {
    const r = { ...compositeStfR, shadow: v };
    if (compositeStfLinked) {
      const g = { ...compositeStfG, shadow: v };
      const b = { ...compositeStfB, shadow: v };
      setCompositeStf(r, g, b);
      scheduleRestretch(r, g, b);
    } else {
      setCompositeStf(r, compositeStfG, compositeStfB);
      scheduleRestretch(r, compositeStfG, compositeStfB);
    }
  }, [compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, setCompositeStf, scheduleRestretch]);

  const handleMidtoneR = useCallback((v: number) => {
    const r = { ...compositeStfR, midtone: v };
    if (compositeStfLinked) {
      const g = { ...compositeStfG, midtone: v };
      const b = { ...compositeStfB, midtone: v };
      setCompositeStf(r, g, b);
      scheduleRestretch(r, g, b);
    } else {
      setCompositeStf(r, compositeStfG, compositeStfB);
      scheduleRestretch(r, compositeStfG, compositeStfB);
    }
  }, [compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, setCompositeStf, scheduleRestretch]);

  const handleHighlightR = useCallback((v: number) => {
    const r = { ...compositeStfR, highlight: v };
    if (compositeStfLinked) {
      const g = { ...compositeStfG, highlight: v };
      const b = { ...compositeStfB, highlight: v };
      setCompositeStf(r, g, b);
      scheduleRestretch(r, g, b);
    } else {
      setCompositeStf(r, compositeStfG, compositeStfB);
      scheduleRestretch(r, compositeStfG, compositeStfB);
    }
  }, [compositeStfR, compositeStfG, compositeStfB, compositeStfLinked, setCompositeStf, scheduleRestretch]);

  const handleShadowG = useCallback((v: number) => {
    const g = { ...compositeStfG, shadow: v };
    setCompositeStf(compositeStfR, g, compositeStfB);
    scheduleRestretch(compositeStfR, g, compositeStfB);
  }, [compositeStfR, compositeStfG, compositeStfB, setCompositeStf, scheduleRestretch]);

  const handleMidtoneG = useCallback((v: number) => {
    const g = { ...compositeStfG, midtone: v };
    setCompositeStf(compositeStfR, g, compositeStfB);
    scheduleRestretch(compositeStfR, g, compositeStfB);
  }, [compositeStfR, compositeStfG, compositeStfB, setCompositeStf, scheduleRestretch]);

  const handleHighlightG = useCallback((v: number) => {
    const g = { ...compositeStfG, highlight: v };
    setCompositeStf(compositeStfR, g, compositeStfB);
    scheduleRestretch(compositeStfR, g, compositeStfB);
  }, [compositeStfR, compositeStfG, compositeStfB, setCompositeStf, scheduleRestretch]);

  const handleShadowB = useCallback((v: number) => {
    const b = { ...compositeStfB, shadow: v };
    setCompositeStf(compositeStfR, compositeStfG, b);
    scheduleRestretch(compositeStfR, compositeStfG, b);
  }, [compositeStfR, compositeStfG, compositeStfB, setCompositeStf, scheduleRestretch]);

  const handleMidtoneB = useCallback((v: number) => {
    const b = { ...compositeStfB, midtone: v };
    setCompositeStf(compositeStfR, compositeStfG, b);
    scheduleRestretch(compositeStfR, compositeStfG, b);
  }, [compositeStfR, compositeStfG, compositeStfB, setCompositeStf, scheduleRestretch]);

  const handleHighlightB = useCallback((v: number) => {
    const b = { ...compositeStfB, highlight: v };
    setCompositeStf(compositeStfR, compositeStfG, b);
    scheduleRestretch(compositeStfR, compositeStfG, b);
  }, [compositeStfR, compositeStfG, compositeStfB, setCompositeStf, scheduleRestretch]);

  const handleResetAuto = useCallback(() => {
    if (compositeAutoStfR && compositeAutoStfG && compositeAutoStfB) {
      setCompositeStf(compositeAutoStfR, compositeAutoStfG, compositeAutoStfB);
      scheduleRestretch(compositeAutoStfR, compositeAutoStfG, compositeAutoStfB);
    }
  }, [compositeAutoStfR, compositeAutoStfG, compositeAutoStfB, setCompositeStf, scheduleRestretch]);

  const hasAutoStf = compositeAutoStfR !== null;
  const fmtStf = (v: number) => v.toFixed(4);

  return (
    <div className="border-b border-zinc-800/50 bg-zinc-900/40">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 w-full px-3 py-1.5 text-[10px] text-zinc-400 hover:text-zinc-200 transition-colors"
      >
        {expanded ? <ChevronUp size={10} /> : <ChevronDown size={10} />}
        <span>STF Stretch</span>
        {restretching && <Loader2 size={10} className="animate-spin text-violet-400 ml-auto" />}
      </button>

      {expanded && (
        <div className="px-3 pb-2 flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <button
              onClick={() => setCompositeStfLinked(!compositeStfLinked)}
              className={`flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded transition-colors ${
                compositeStfLinked ? "text-violet-300 bg-violet-900/30" : "text-zinc-500 hover:text-zinc-300"
              }`}
              title={compositeStfLinked ? "Linked: same STF for all channels" : "Per-channel STF"}
            >
              {compositeStfLinked ? <Link size={10} /> : <Unlink size={10} />}
              {compositeStfLinked ? "Linked" : "Per-channel"}
            </button>
            {hasAutoStf && (
              <button
                onClick={handleResetAuto}
                className="flex items-center gap-1 text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors ml-auto"
              >
                <RotateCcw size={10} />
                Reset
              </button>
            )}
          </div>

          {compositeStfLinked ? (
            <div className="flex flex-col gap-1.5">
              <Slider label="Shadow" value={compositeStfR.shadow} min={0} max={1} step={0.0001} accent="violet" format={fmtStf} onChange={handleShadowR} />
              <Slider label="Midtone" value={compositeStfR.midtone} min={0} max={1} step={0.0001} accent="violet" format={fmtStf} onChange={handleMidtoneR} />
              <Slider label="Highlight" value={compositeStfR.highlight} min={0} max={1} step={0.0001} accent="violet" format={fmtStf} onChange={handleHighlightR} />
            </div>
          ) : (
            <div className="flex flex-col gap-2">
              <div className="flex flex-col gap-1">
                <span className="text-[9px] text-red-400 font-medium uppercase tracking-wider">Red</span>
                <Slider label="S" value={compositeStfR.shadow} min={0} max={1} step={0.0001} accent="red" format={fmtStf} onChange={handleShadowR} />
                <Slider label="M" value={compositeStfR.midtone} min={0} max={1} step={0.0001} accent="red" format={fmtStf} onChange={handleMidtoneR} />
                <Slider label="H" value={compositeStfR.highlight} min={0} max={1} step={0.0001} accent="red" format={fmtStf} onChange={handleHighlightR} />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-[9px] text-green-400 font-medium uppercase tracking-wider">Green</span>
                <Slider label="S" value={compositeStfG.shadow} min={0} max={1} step={0.0001} accent="green" format={fmtStf} onChange={handleShadowG} />
                <Slider label="M" value={compositeStfG.midtone} min={0} max={1} step={0.0001} accent="green" format={fmtStf} onChange={handleMidtoneG} />
                <Slider label="H" value={compositeStfG.highlight} min={0} max={1} step={0.0001} accent="green" format={fmtStf} onChange={handleHighlightG} />
              </div>
              <div className="flex flex-col gap-1">
                <span className="text-[9px] text-blue-400 font-medium uppercase tracking-wider">Blue</span>
                <Slider label="S" value={compositeStfB.shadow} min={0} max={1} step={0.0001} accent="blue" format={fmtStf} onChange={handleShadowB} />
                <Slider label="M" value={compositeStfB.midtone} min={0} max={1} step={0.0001} accent="blue" format={fmtStf} onChange={handleMidtoneB} />
                <Slider label="H" value={compositeStfB.highlight} min={0} max={1} step={0.0001} accent="blue" format={fmtStf} onChange={handleHighlightB} />
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function PreviewTabInner({ useGpu, rawPixels, onImageClick, starOverlayRef }: PreviewTabProps) {
  const { file } = useFileContext();
  const { stfParams } = useHistContext();
  const { isCube } = useCubeContext();
  const { renderedPreviewUrl, compositePreviewUrl, clearComposite } = useRenderContext();

  const [previewError, setPreviewError] = useState(false);
  const [retryKey, setRetryKey] = useState(0);
  const retryRef = useRef<{ timer: ReturnType<typeof setTimeout> | null; count: number }>({
    timer: null,
    count: 0,
  });

  useEffect(() => {
    setPreviewError(false);
    setRetryKey(0);
    retryRef.current.count = 0;
  }, [file?.id, renderedPreviewUrl]);

  useEffect(() => {
    return () => {
      if (retryRef.current.timer) clearTimeout(retryRef.current.timer);
    };
  }, []);

  const handlePreviewError = useCallback(() => {
    const r = retryRef.current;
    if (r.timer) return;
    if (r.count < MAX_RETRIES) {
      const delay = RETRY_DELAYS[r.count];
      r.timer = setTimeout(() => {
        r.timer = null;
        r.count += 1;
        setRetryKey((k) => k + 1);
      }, delay);
    } else {
      setPreviewError(true);
    }
  }, []);

  const baseUrl = renderedPreviewUrl || file?.result?.previewUrl;

  const previewUrl = useMemo(() => {
    if (!baseUrl) return null;
    if (retryKey === 0) return baseUrl;
    return `${baseUrl}${baseUrl.includes("?") ? "&" : "?"}t=${retryKey}`;
  }, [baseUrl, retryKey]);

  if (compositePreviewUrl) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex items-center gap-2 px-3 py-1.5 bg-violet-900/30 border-b border-violet-600/20">
          <span className="text-[10px] text-violet-300">RGB Composite</span>
          <button
            onClick={clearComposite}
            className="ml-auto flex items-center gap-1 text-[10px] text-zinc-400 hover:text-zinc-200 transition-colors"
          >
            Back to file
            <X size={10} />
          </button>
        </div>
        <CompositeStfPanel />
        <div className="relative flex-1 min-h-0 flex items-center justify-center">
          <img src={compositePreviewUrl} alt="RGB composite" className="max-w-full max-h-full object-contain" />
        </div>
      </div>
    );
  }

  if (useGpu && rawPixels) {
    return (
      <div className="flex flex-col h-full">
        <div className="relative flex-1 min-h-0 flex items-center justify-center">
          <Suspense fallback={<Loader2 size={20} className="animate-spin text-zinc-600" />}>
            <GpuRenderer
              rawData={rawPixels.data}
              width={rawPixels.width}
              height={rawPixels.height}
              dataMin={rawPixels.min}
              dataMax={rawPixels.max}
              shadow={stfParams.shadow}
              midtone={stfParams.midtone}
              highlight={stfParams.highlight}
              className="max-w-full max-h-full object-contain"
            />
          </Suspense>
          <Overlay starOverlayRef={starOverlayRef} isCube={isCube} />
        </div>
      </div>
    );
  }

  if (previewUrl && !previewError) {
    return (
      <div className="flex flex-col h-full">
        <div className="relative flex-1 min-h-0 flex items-center justify-center">
          <img
            src={previewUrl}
            alt={file?.name}
            className={`max-w-full max-h-full object-contain ${isCube ? "cursor-crosshair" : ""}`}
            onClick={onImageClick}
            onError={handlePreviewError}
            loading="eager"
            decoding="async"
          />
          <Overlay starOverlayRef={starOverlayRef} isCube={isCube} />
        </div>
      </div>
    );
  }

  if (previewError) {
    return (
      <div className="flex flex-col h-full">
        <div className="flex-1 flex flex-col items-center justify-center gap-2 text-zinc-600">
          <Image size={32} strokeWidth={1} />
          <p className="text-xs">Preview unavailable</p>
          <button
            onClick={() => {
              retryRef.current.count = 0;
              setPreviewError(false);
              setRetryKey((k) => k + 1);
            }}
            className="text-[10px] hover:text-zinc-300 mt-1"
            style={{ color: "var(--ab-teal)" }}
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  return null;
}

export default memo(PreviewTabInner);
