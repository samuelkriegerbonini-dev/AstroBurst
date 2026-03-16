import { useState, useCallback, useRef, useEffect, useMemo, memo, lazy, Suspense } from "react";
import { Image, Loader2 } from "lucide-react";
import { useFileContext, useHistContext, useCubeContext, useRenderContext } from "../../context/PreviewContext";
import type { RawPixelData } from "../../shared/types";

const GpuRenderer = lazy(() => import("../render/GpuRenderer"));

interface PreviewTabProps {
  useGpu: boolean;
  rawPixels: RawPixelData | null;
  onImageClick: (e: React.MouseEvent<HTMLImageElement>) => void;
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
}

const MAX_RETRIES = 2;
const RETRY_DELAYS = [300, 800] as const;

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

function PreviewTabInner({ useGpu, rawPixels, onImageClick, starOverlayRef }: PreviewTabProps) {
  const { file } = useFileContext();
  const { stfParams } = useHistContext();
  const { isCube } = useCubeContext();
  const { renderedPreviewUrl } = useRenderContext();

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
