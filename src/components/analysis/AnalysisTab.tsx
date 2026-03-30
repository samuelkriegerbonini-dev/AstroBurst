import { useState, useCallback, useRef, useMemo, lazy, Suspense, memo } from "react";
import { Loader2 } from "lucide-react";
import HistogramPanel from "./HistogramPanel";
import { detectStars, computeFftSpectrum, applyStfRender } from "../../services/analysis";
import { getOutputDir } from "../../infrastructure/tauri";
import { useFileContext, useHistContext, useCubeContext, useRenderContext, useRawPixelsContext } from "../../context/PreviewContext";
import type { StfParams } from "../../shared/types";

const FFTPanel = lazy(() => import("./FFTPanel"));
const SpectroscopyPanel = lazy(() => import("./SpectroscopyPanel"));
const PlateSolvePanel = lazy(() => import("./PlateSolvePanel"));
const TileViewerPanel = lazy(() => import("./TileViewerPanel"));

const EMPTY_STARS: any[] = [];

function TabSpinner() {
  return (
    <div className="flex items-center justify-center py-12">
      <div
        className="w-5 h-5 rounded-full animate-spin"
        style={{ border: "2px solid transparent", borderTopColor: "var(--ab-teal)", borderRightColor: "rgba(20,184,166,0.3)" }}
      />
    </div>
  );
}

interface AnalysisTabProps {
  spectrum: number[];
  specWavelengths: number[] | null;
  specCoord: { x: number; y: number } | null;
  specLoading: boolean;
  specElapsed: number;
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
}

function AnalysisTabInner({
                            spectrum,
                            specWavelengths,
                            specCoord,
                            specLoading,
                            specElapsed,
                            starOverlayRef,
                          }: AnalysisTabProps) {
  const { file } = useFileContext();
  const { histData, stfParams, setStfParams } = useHistContext();
  const { isCube, cubeDims } = useCubeContext();
  const { setRenderedPreviewUrl, activeImagePath, isShowingComposite } = useRenderContext();
  const { rawPixels } = useRawPixelsContext();

  const [starResult, setStarResult] = useState<any>(null);
  const [starLoading, setStarLoading] = useState(false);

  const effectivePath = (isShowingComposite && activeImagePath) ? activeImagePath : file?.path;

  const rafIdRef = useRef<number | null>(null);
  const pendingStfRef = useRef<StfParams | null>(null);
  const ipcBusyRef = useRef(false);
  const ipcFailCountRef = useRef(0);

  const flushStfIpc = useCallback(async () => {
    if (ipcBusyRef.current || !pendingStfRef.current || !effectivePath) return;
    if (ipcFailCountRef.current >= 3) {
      pendingStfRef.current = null;
      ipcFailCountRef.current = 0;
      return;
    }
    const params = pendingStfRef.current;
    pendingStfRef.current = null;
    ipcBusyRef.current = true;
    try {
      const result = await applyStfRender(
        effectivePath,
        await getOutputDir(),
        params.shadow,
        params.midtone,
        params.highlight,
      );
      ipcFailCountRef.current = 0;
      if (result.previewUrl) {
        const bust = `${result.previewUrl}${result.previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`;
        setRenderedPreviewUrl(bust);
      }
    } catch (e) {
      ipcFailCountRef.current++;
      console.error("STF render failed:", e);
    } finally {
      ipcBusyRef.current = false;
      if (pendingStfRef.current) queueMicrotask(() => flushStfIpc());
    }
  }, [effectivePath, setRenderedPreviewUrl]);

  const handleStfChange = useCallback(
    (params: StfParams) => {
      setStfParams(params);
      if (rawPixels) return;
      pendingStfRef.current = params;
      if (rafIdRef.current) cancelAnimationFrame(rafIdRef.current);
      rafIdRef.current = requestAnimationFrame(() => {
        rafIdRef.current = null;
        flushStfIpc();
      });
    },
    [setStfParams, flushStfIpc, rawPixels],
  );

  const handleAutoStf = useCallback(() => {
    if (histData?.auto_stf) {
      const params = histData.auto_stf;
      setStfParams(params);
      handleStfChange(params);
    }
  }, [histData, handleStfChange, setStfParams]);

  const handleResetStf = useCallback(() => {
    setStfParams({ shadow: 0, midtone: 0.5, highlight: 1 });
  }, [setStfParams]);

  const handleDetectStars = useCallback(
    async (sigma: number) => {
      if (!effectivePath) return;
      setStarLoading(true);
      try {
        const result = await detectStars(effectivePath, sigma, 200);
        setStarResult(result);
      } catch (e) {
        console.error("Star detection failed:", e);
      } finally {
        setStarLoading(false);
      }
    },
    [effectivePath],
  );

  const handleCollapsePreview = useCallback(
    (previewUrl: string) => {
      const bust = `${previewUrl}${previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`;
      setRenderedPreviewUrl(bust);
    },
    [setRenderedPreviewUrl],
  );

  const histStats = useMemo(
    () =>
      histData
        ? { median: histData.median, mean: histData.mean, sigma: histData.sigma }
        : null,
    [histData?.median, histData?.mean, histData?.sigma],
  );

  const stars = starResult?.stars || EMPTY_STARS;

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3">
        {histData && histStats && (
          <HistogramPanel
            bins={histData.bins as any}
            dataMin={histData.data_min}
            dataMax={histData.data_max}
            autoStf={histData.auto_stf}
            shadow={stfParams.shadow}
            midtone={stfParams.midtone}
            highlight={stfParams.highlight}
            onChange={handleStfChange}
            onAutoStf={handleAutoStf}
            onReset={handleResetStf}
            stats={histStats}
          />
        )}

        <PlateSolvePanel
          stars={stars}
          isLoading={starLoading}
          onDetect={handleDetectStars}
          backgroundMedian={starResult?.background_median}
          backgroundSigma={starResult?.background_sigma}
          imageWidth={starResult?.image_width || file?.result?.dimensions?.[0]}
          imageHeight={starResult?.image_height || file?.result?.dimensions?.[1]}
          elapsed={starResult?.elapsed_ms || 0}
          overlayCanvasRef={starOverlayRef}
          filePath={effectivePath ?? null}
        />

        {effectivePath && !isCube && (file?.result?.dimensions?.[0] ?? 0) >= 64 && (
          <FFTPanel filePath={effectivePath} computeFftSpectrum={computeFftSpectrum} />
        )}

        {isCube && (
          <SpectroscopyPanel
            spectrum={spectrum}
            wavelengths={specWavelengths}
            pixelCoord={specCoord}
            isLoading={specLoading}
            cubeDims={cubeDims}
            elapsed={specElapsed}
            filePath={effectivePath}
            onCollapsePreview={handleCollapsePreview}
          />
        )}

        <TileViewerPanel
          filePath={effectivePath || null}
          imageWidth={file?.result?.dimensions?.[0]}
          imageHeight={file?.result?.dimensions?.[1]}
        />
      </div>
    </Suspense>
  );
}

export default memo(AnalysisTabInner);
