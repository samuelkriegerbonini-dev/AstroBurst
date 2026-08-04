import { useState, useCallback, useRef, useMemo, lazy, Suspense, memo } from "react";
import HistogramPanel from "./HistogramPanel";
import RgbStfPanel from "./RgbStfPanel";
import { detectStars, detectStarsComposite, computeFftSpectrum, applyStfRender } from "../../services/analysis";
import { getOutputDir } from "../../infrastructure/tauri";
import { getPreviewUrl } from "../../infrastructure/tauri";
import { useFileContext, useHistContext, useCubeContext, useRenderContext, useRawPixelsContext } from "../../context/PreviewContext";
import { useCompositePreview } from "../../context/CompositeContext";
import type { StfParams } from "../../shared/types";
import type { Star } from "./PlateSolvePanel";
import type { StarDetectionResult } from "../../shared/types";

const FFTPanel = lazy(() => import("./FFTPanel"));
const SpectroscopyPanel = lazy(() => import("./SpectroscopyPanel"));
const PlateSolvePanel = lazy(() => import("./PlateSolvePanel"));
const PhotometryPanel = lazy(() => import("./PhotometryPanel"));
const TileViewerPanel = lazy(() => import("./TileViewerPanel"));

const EMPTY_STARS: Star[] = [];

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
  specError?: string | null;
  starOverlayRef: React.RefObject<HTMLCanvasElement | null>;
}

function AnalysisTabInner({
                            spectrum,
                            specWavelengths,
                            specCoord,
                            specLoading,
                            specElapsed,
                            specError = null,
                            starOverlayRef,
                          }: AnalysisTabProps) {
  const { file } = useFileContext();
  const { histData, stfParams, setStfParams } = useHistContext();
  const { isCube, cubeDims } = useCubeContext();
  const { setRenderedPreviewUrl, activeImagePath } = useRenderContext();
  const { isShowingComposite } = useCompositePreview();
  const { rawPixels, rgbRawPixels } = useRawPixelsContext();

  const [starResult, setStarResult] = useState<StarDetectionResult | null>(null);
  const [starLoading, setStarLoading] = useState(false);
  const [detectError, setDetectError] = useState<string | null>(null);

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
      if (rawPixels || rgbRawPixels) return;
      pendingStfRef.current = params;
      if (rafIdRef.current) cancelAnimationFrame(rafIdRef.current);
      rafIdRef.current = requestAnimationFrame(() => {
        rafIdRef.current = null;
        flushStfIpc();
      });
    },
    [setStfParams, flushStfIpc, rawPixels, rgbRawPixels],
  );

  const handleAutoStf = useCallback(() => {
    if (histData?.auto_stf) {
      const params = histData.auto_stf;
      setStfParams(params);
      handleStfChange(params);
    }
  }, [histData, handleStfChange, setStfParams]);

  const handleResetStf = useCallback(() => {
    handleStfChange({ shadow: 0, midtone: 0.5, highlight: 1 });
  }, [handleStfChange]);

  const handleDetectStars = useCallback(
    async (sigma: number) => {
      setStarLoading(true);
      setDetectError(null);
      try {
        const result = isShowingComposite
          ? await detectStarsComposite(sigma, 200)
          : effectivePath
            ? await detectStars(effectivePath, sigma, 200)
            : null;
        setStarResult(result);
      } catch (e) {
        console.error("Star detection failed:", e);
        setDetectError(e instanceof Error ? e.message : String(e));
      } finally {
        setStarLoading(false);
      }
    },
    [effectivePath, isShowingComposite],
  );

  const handleCollapsePreview = useCallback(
    (previewUrl: string) => {
      const bust = `${previewUrl}${previewUrl.includes("?") ? "&" : "?"}t=${Date.now()}`;
      setRenderedPreviewUrl(bust);
    },
    [setRenderedPreviewUrl],
  );

  const frameSeqRef = useRef(0);
  const handleFramePreview = useCallback(
    async (outputPath: string) => {
      const seq = ++frameSeqRef.current;
      try {
        const url = await getPreviewUrl(outputPath);
        if (frameSeqRef.current !== seq) return;
        setRenderedPreviewUrl(`${url}${url.includes("?") ? "&" : "?"}t=${Date.now()}`);
      } catch (e) {
        console.error("Frame preview failed:", e);
      }
    },
    [setRenderedPreviewUrl],
  );

  const hasHist = histData !== null;
  const histMedian = histData?.median;
  const histMean = histData?.mean;
  const histSigma = histData?.sigma;
  const histStats = useMemo(
    () =>
      hasHist
        ? { median: histMedian as number, mean: histMean as number, sigma: histSigma as number }
        : null,
    [hasHist, histMedian, histMean, histSigma],
  );

  const stars = starResult?.stars || EMPTY_STARS;

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
        {histData && histStats && (
          <HistogramPanel
            bins={histData.bins}
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

        {isShowingComposite && rgbRawPixels && <RgbStfPanel />}

        <PlateSolvePanel
          stars={stars}
          isLoading={starLoading}
          onDetect={handleDetectStars}
          detectError={detectError}
          backgroundMedian={starResult?.background_median}
          backgroundSigma={starResult?.background_sigma}
          imageWidth={starResult?.image_width || file?.result?.dimensions?.[0]}
          imageHeight={starResult?.image_height || file?.result?.dimensions?.[1]}
          elapsed={starResult?.elapsed_ms || 0}
          overlayCanvasRef={starOverlayRef}
          filePath={effectivePath ?? null}
        />

        <PhotometryPanel filePath={effectivePath ?? null} />

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
            error={specError}
            filePath={effectivePath}
            onCollapsePreview={handleCollapsePreview}
            onFramePreview={handleFramePreview}
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
