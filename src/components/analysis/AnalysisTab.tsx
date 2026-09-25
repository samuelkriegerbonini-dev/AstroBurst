import { useState, useCallback, useEffect, useRef, useMemo, lazy, Suspense, memo } from "react";
import HistogramPanel from "./HistogramPanel";
import RgbStfPanel from "./RgbStfPanel";
import MeasurementBadge from "./MeasurementBadge";
import { detectStars, detectStarsComposite, computeFftSpectrum, applyStfRender } from "../../services/analysis";
import { getOutputDir } from "../../infrastructure/tauri";
import { getPreviewUrl } from "../../infrastructure/tauri";
import { fileKeyOf, useFileContext, useHistContext, useCubeContext, useRenderActions, useRawPixelsContext, useDisplayContext } from "../../context/PreviewContext";
import { formatFrameLabel, frameAxisValue } from "../../utils/cubeNavigation";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useAnalysisTarget } from "../../hooks/useAnalysisTarget";
import { cpuStfRenderAllowed, histogramStfLock, rgbStfPanelMode } from "../../utils/analysisTarget";
import type { StfParams } from "../../shared/types";
import type { Star } from "./PlateSolvePanel";
import type { CubeResult } from "./SpectroscopyPanel";
import type { StarDetectionResult } from "../../shared/types";

const FFTPanel = lazy(() => import("./FFTPanel"));
const SpectroscopyPanel = lazy(() => import("./SpectroscopyPanel"));
const PlateSolvePanel = lazy(() => import("./PlateSolvePanel"));
const PhotometryPanel = lazy(() => import("./PhotometryPanel"));
const PhotometryTablePanel = lazy(() => import("./PhotometryTablePanel"));
const TimeSeriesPanel = lazy(() => import("./TimeSeriesPanel"));
const TileViewerPanel = lazy(() => import("./TileViewerPanel"));
const RegionsPanel = lazy(() => import("../regions/RegionsPanel"));
const RegionProfilesPanel = lazy(() => import("../regions/RegionProfilesPanel"));
const ContourPanel = lazy(() => import("./ContourPanel"));
const StatisticsPanel = lazy(() => import("./StatisticsPanel"));
const PixelTablePanel = lazy(() => import("./PixelTablePanel"));
const CatalogPanel = lazy(() => import("./CatalogPanel"));
const TargetsPanel = lazy(() => import("./TargetsPanel"));

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
  const { publishProcessed, setStfPreviewUrl } = useRenderActions();
  const { rawPixels, rawPixelsLoading, rgbRawPixels, rgbRawPixelsLoading } = useRawPixelsContext();
  const { display } = useDisplayContext();

  const [starResult, setStarResult] = useState<StarDetectionResult | null>(null);
  const [starLoading, setStarLoading] = useState(false);
  const [detectError, setDetectError] = useState<string | null>(null);

  const target = useAnalysisTarget();
  const effectivePath = target.path;
  const compositeOnScreen = target.composite;
  const rgbPath = target.rgbPath;
  const regionKey = useRegionKey();
  const stfLock = histogramStfLock({
    stretch: display.stretch,
    compositeOnScreen,
    previewOnly: target.displayed.previewOnly,
  });
  const detectSeqRef = useRef(0);

  useEffect(() => {
    detectSeqRef.current++;
    setStarResult(null);
    setDetectError(null);
    setStarLoading(false);
  }, [effectivePath, compositeOnScreen, rgbPath]);

  const rafIdRef = useRef<number | null>(null);
  const pendingStfRef = useRef<{ params: StfParams; path: string } | null>(null);
  const ipcBusyRef = useRef(false);
  const ipcFailCountRef = useRef(0);
  const flushStfRef = useRef<() => void>(() => {});

  const flushStfIpc = useCallback(async () => {
    if (ipcBusyRef.current || !pendingStfRef.current || !effectivePath) return;
    if (ipcFailCountRef.current >= 3) {
      pendingStfRef.current = null;
      ipcFailCountRef.current = 0;
      return;
    }
    const { params, path } = pendingStfRef.current;
    pendingStfRef.current = null;
    if (path !== effectivePath) return;
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
        setStfPreviewUrl(bust);
      }
    } catch (e) {
      ipcFailCountRef.current++;
      console.error("STF render failed:", e);
    } finally {
      ipcBusyRef.current = false;
      if (pendingStfRef.current) queueMicrotask(() => flushStfRef.current());
    }
  }, [effectivePath, setStfPreviewUrl]);

  useEffect(() => {
    flushStfRef.current = flushStfIpc;
  }, [flushStfIpc]);

  const cpuRender = cpuStfRenderAllowed({
    hasRawPixels: rawPixels !== null || rgbRawPixels !== null,
    rawPixelsLoading: rawPixelsLoading || rgbRawPixelsLoading,
    locked: stfLock !== null,
  });

  const handleStfChange = useCallback(
    (params: StfParams) => {
      setStfParams(params);
      if (!cpuRender || !effectivePath) return;
      pendingStfRef.current = { params, path: effectivePath };
      if (rafIdRef.current) cancelAnimationFrame(rafIdRef.current);
      rafIdRef.current = requestAnimationFrame(() => {
        rafIdRef.current = null;
        flushStfRef.current();
      });
    },
    [setStfParams, cpuRender, effectivePath],
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
      const seq = ++detectSeqRef.current;
      setStarLoading(true);
      setDetectError(null);
      try {
        const result = compositeOnScreen
          ? await detectStarsComposite(sigma, 200, rgbPath)
          : effectivePath
            ? await detectStars(effectivePath, sigma, 200)
            : null;
        if (detectSeqRef.current !== seq) return;
        setStarResult(result);
      } catch (e) {
        console.error("Star detection failed:", e);
        if (detectSeqRef.current === seq) setDetectError(e instanceof Error ? e.message : String(e));
      } finally {
        if (detectSeqRef.current === seq) setStarLoading(false);
      }
    },
    [effectivePath, compositeOnScreen, rgbPath],
  );

  const filePath = file?.path;
  const fileKey = fileKeyOf(file);
  const publishCube = useCallback(
    (result: CubeResult) => {
      if (!fileKey || !filePath) return;
      publishProcessed(fileKey, {
        previewUrl: result.previewUrl,
        fitsPath: result.fitsPath,
        dimensions: result.dimensions,
        label: result.label,
        kind: "cube",
        inputPath: filePath,
      });
    },
    [publishProcessed, fileKey, filePath],
  );

  const frameSeqRef = useRef(0);
  const handleFramePreview = useCallback(
    async (outputPath: string, frameIndex: number, fitsPath?: string) => {
      const seq = ++frameSeqRef.current;
      try {
        const url = await getPreviewUrl(outputPath);
        if (frameSeqRef.current !== seq) return;
        const total = cubeDims?.frames ?? 0;
        const axis = cubeDims?.spectral_axis ?? null;
        const label = formatFrameLabel(frameIndex, total, frameAxisValue(frameIndex, axis?.values), axis?.unit ?? "");
        const dimensions: [number, number] | null = fitsPath && cubeDims ? [cubeDims.width, cubeDims.height] : null;
        publishCube({ label, previewUrl: url, fitsPath: fitsPath ?? null, dimensions });
      } catch (e) {
        console.error("Frame preview failed:", e);
      }
    },
    [publishCube, cubeDims],
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
  const rgbStfMode = rgbStfPanelMode({
    compositeOnScreen,
    hasRgbRawPixels: rgbRawPixels !== null,
    displayReferred: rgbRawPixels?.displayReferred === true,
  });
  const targetWidth = target.dimensions?.[0];
  const targetHeight = target.dimensions?.[1];
  const measurementBadge = useMemo(() => <MeasurementBadge />, []);
  const compositeMeasurementBadge = useMemo(() => <MeasurementBadge measuresComposite />, []);

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
            disabled={stfLock !== null}
            disabledHint={stfLock ?? undefined}
            badge={measurementBadge}
          />
        )}

        {rgbStfMode === "live" && <RgbStfPanel showSlotHistogram={!target.fileRgbView} />}
        {rgbStfMode === "baked" && (
          <div className="px-3 py-2 rounded-lg border border-violet-600/20 bg-violet-900/10 text-[10px] text-violet-300/80">
            RGB Channel STF is baked in: the composite on screen is display-referred (stretch or curves applied), so
            channel sliders would not change it.
          </div>
        )}

        <PlateSolvePanel
          stars={stars}
          isLoading={starLoading}
          onDetect={handleDetectStars}
          detectError={detectError}
          backgroundMedian={starResult?.background_median}
          backgroundSigma={starResult?.background_sigma}
          imageWidth={starResult?.image_width || targetWidth}
          imageHeight={starResult?.image_height || targetHeight}
          elapsed={starResult?.elapsed_ms || 0}
          overlayCanvasRef={starOverlayRef}
          filePath={regionKey}
          sourceBadge={compositeMeasurementBadge}
        />

        <PhotometryPanel filePath={effectivePath} />

        <PhotometryTablePanel filePath={effectivePath} overlayKey={regionKey} stars={stars} />

        <TimeSeriesPanel filePath={regionKey} />

        <CatalogPanel filePath={regionKey} />

        <TargetsPanel filePath={regionKey} />

        <StatisticsPanel filePath={effectivePath} composite={compositeOnScreen} rgbPath={target.rgbPath} />

        <PixelTablePanel filePath={effectivePath} />

        <RegionsPanel filePath={regionKey} measurePath={effectivePath} />

        <RegionProfilesPanel filePath={regionKey} measurePath={effectivePath} />

        <ContourPanel filePath={effectivePath} overlayKey={regionKey} imageWidth={targetWidth} imageHeight={targetHeight} />

        {effectivePath && !isCube && (targetWidth ?? 0) >= 64 && (
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
            filePath={filePath}
            onCubeResult={publishCube}
            onFramePreview={handleFramePreview}
          />
        )}

        <TileViewerPanel
          filePath={effectivePath}
          composite={compositeOnScreen}
          rgbPath={rgbPath}
          imageWidth={targetWidth}
          imageHeight={targetHeight}
        />
      </div>
    </Suspense>
  );
}

export default memo(AnalysisTabInner);
