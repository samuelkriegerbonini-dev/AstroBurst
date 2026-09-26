import { useState, useCallback, useEffect, useRef, useMemo, lazy, Suspense, memo } from "react";
import HistogramPanel from "./HistogramPanel";
import RgbStfPanel from "./RgbStfPanel";
import MeasurementBadge from "./MeasurementBadge";
import { detectStars, detectStarsComposite, computeFftSpectrum, applyStfRender, computeHistogram } from "../../services/analysis";
import { getOutputDir } from "../../infrastructure/tauri";
import { getPreviewUrl } from "../../infrastructure/tauri";
import { fileKeyOf, useFileContext, useHistContext, useCubeContext, useRenderActions, useRenderContext, useRawPixelsContext, useDisplayContext, useDqContext } from "../../context/PreviewContext";
import { useToolHost } from "../../context/ToolHostContext";
import { histogramSkyWindow } from "../../utils/histogramWindow";
import type { HistogramRange } from "../../utils/histogramWindow";
import { ANALYSIS_SECTION, analysisSections, deepZoomAvailable } from "../../utils/analysisSections";
import { frameRecordLabel } from "../../utils/cubeNavigation";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useAnalysisTarget } from "../../hooks/useAnalysisTarget";
import {
  analysisMeasureKey,
  cpuStfRenderAllowed,
  detectedStarsOnMeasuredImage,
  histogramStfLock,
  rgbStfPanelMode,
} from "../../utils/analysisTarget";
import type { StfParams } from "../../shared/types";
import type { Star } from "./PlateSolvePanel";
import type { CubeResult } from "./SpectroscopyPanel";
import type { StarDetectionResult, HistogramData } from "../../shared/types";

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
const MeasurementLogPanel = lazy(() => import("./MeasurementLogPanel"));
const ObservationGeometryPanel = lazy(() => import("./ObservationGeometryPanel"));
const PvPanel = lazy(() => import("./PvPanel"));

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
  const { histData, histDataPath, stfParams, setStfParams } = useHistContext();
  const { isCube, cubeDims } = useCubeContext();
  const { publishProcessed, setStfPreviewUrl } = useRenderActions();
  const { processed, processedVersion } = useRenderContext();
  const { rawPixels, rawPixelsLoading, rgbRawPixels, rgbRawPixelsLoading } = useRawPixelsContext();
  const { display } = useDisplayContext();
  const { excludeDq } = useDqContext();
  const { gpuDisplay } = useToolHost();

  const [starResult, setStarResult] = useState<StarDetectionResult | null>(null);
  const [starLoading, setStarLoading] = useState(false);
  const [detectError, setDetectError] = useState<string | null>(null);

  const target = useAnalysisTarget();
  const effectivePath = target.path;
  const compositeOnScreen = target.composite;
  const rgbPath = target.rgbPath;
  const regionKey = useRegionKey();
  const measureKey = analysisMeasureKey({
    path: effectivePath,
    composite: compositeOnScreen,
    processedFitsPath: processed?.fitsPath ?? null,
    processedVersion,
  });
  const stfLock = histogramStfLock({
    stretch: gpuDisplay ? display.stretch : "mtf",
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
        frameIndex: result.frameIndex,
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
        const label = frameRecordLabel(frameIndex, cubeDims?.frames ?? 0, cubeDims?.spectral_axis ?? null);
        const dimensions: [number, number] | null = fitsPath && cubeDims ? [cubeDims.width, cubeDims.height] : null;
        publishCube({ label, previewUrl: url, fitsPath: fitsPath ?? null, dimensions, frameIndex });
      } catch (e) {
        console.error("Frame preview failed:", e);
      }
    },
    [publishCube, cubeDims],
  );

  const histPath = processed?.fitsPath ?? filePath ?? null;
  const histOnPath = histPath !== null && histDataPath === histPath;
  const skyWindow = useMemo(
    () =>
      histData
        ? histogramSkyWindow({ median: histData.median, sigma: histData.sigma, dataMin: histData.data_min, dataMax: histData.data_max })
        : null,
    [histData],
  );
  const [skyZoom, setSkyZoom] = useState(false);
  const [skyHist, setSkyHist] = useState<{ source: HistogramData; window: HistogramRange; bins: number[] } | null>(null);
  const skySeqRef = useRef(0);

  useEffect(() => {
    const seq = ++skySeqRef.current;
    setSkyHist(null);
    if (!skyZoom || !skyWindow || !histPath || !histData || !histOnPath) return;
    computeHistogram(histPath, excludeDq, skyWindow)
      .then((data) => {
        if (skySeqRef.current === seq) setSkyHist({ source: histData, window: skyWindow, bins: data.bins });
      })
      .catch((e) => {
        if (skySeqRef.current !== seq) return;
        console.error("Sky histogram failed:", e);
        setSkyZoom(false);
      });
  }, [skyZoom, skyWindow, histPath, histData, histOnPath, excludeDq]);

  const skyShown = skyZoom && histOnPath && skyHist !== null && skyHist.source === histData ? skyHist : null;

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
  const starsOnMeasuredImage = detectedStarsOnMeasuredImage({ compositeOnScreen, measuresFilePlanes: rgbPath !== null });
  const tableStars = starsOnMeasuredImage ? stars : EMPTY_STARS;
  const rgbStfMode = rgbStfPanelMode({
    compositeOnScreen,
    hasRgbRawPixels: rgbRawPixels !== null,
    displayReferred: rgbRawPixels?.displayReferred === true,
  });
  const targetWidth = target.dimensions?.[0];
  const targetHeight = target.dimensions?.[1];
  const measurementBadge = useMemo(() => <MeasurementBadge />, []);
  const compositeMeasurementBadge = useMemo(() => <MeasurementBadge measuresComposite />, []);

  const showFft = Boolean(effectivePath) && !isCube && (targetWidth ?? 0) >= 64;
  const showDeepZoom = Boolean(effectivePath) && deepZoomAvailable(targetWidth, targetHeight);
  const sections = analysisSections({
    hasHistogram: histData !== null && histData.bins.length > 0,
    isCube,
    showFft,
    showDeepZoom,
  });
  const navRef = useRef<HTMLElement>(null);
  const jumpTo = useCallback((id: string) => {
    const el = document.getElementById(id);
    if (!el) return;
    el.style.scrollMarginTop = `${(navRef.current?.offsetHeight ?? 0) + 8}px`;
    el.scrollIntoView({ block: "start", behavior: "smooth" });
  }, []);

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
        <nav
          ref={navRef}
          aria-label="Analysis panels"
          className="sticky top-0 z-20 -mx-3 -mt-3 px-3 py-1.5 flex flex-wrap gap-1"
          style={{ background: "rgba(5,5,16,0.94)", borderBottom: "1px solid var(--ab-border)" }}
        >
          {sections.map((s) => (
            <button
              key={s.id}
              type="button"
              onClick={() => jumpTo(s.id)}
              className="text-[9px] px-1.5 py-0.5 rounded-full text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800/70 transition-colors"
              style={{ border: "1px solid rgba(63,63,70,0.5)" }}
            >
              {s.label}
            </button>
          ))}
        </nav>

        {histData && histStats && histData.bins.length > 0 && (
          <section id={ANALYSIS_SECTION.histogram.id}>
            <HistogramPanel
              bins={skyShown ? skyShown.bins : histData.bins}
              binsWindow={skyShown ? skyShown.window : null}
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
              skyZoom={skyZoom && skyWindow !== null}
              skyAvailable={skyWindow !== null}
              skyLoading={skyZoom && skyWindow !== null && skyShown === null}
              onSkyZoomChange={setSkyZoom}
            />
          </section>
        )}

        {rgbStfMode === "live" && <RgbStfPanel showSlotHistogram={!target.fileRgbView} />}
        {rgbStfMode === "baked" && (
          <div className="px-3 py-2 rounded-lg border border-violet-600/20 bg-violet-900/10 text-[10px] text-violet-300/80">
            RGB Channel STF is baked in: the composite on screen is display-referred (stretch or curves applied), so
            channel sliders would not change it.
          </div>
        )}

        <section id={ANALYSIS_SECTION.stars.id}>
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
            detectedTotal={starResult?.n_detected ?? null}
          />
        </section>

        <section id={ANALYSIS_SECTION.photometry.id}>
          <PhotometryPanel filePath={effectivePath} />
        </section>

        <section id={ANALYSIS_SECTION.table.id}>
          <PhotometryTablePanel
            filePath={effectivePath}
            overlayKey={regionKey}
            stars={tableStars}
            starsElsewhere={!starsOnMeasuredImage && stars.length > 0}
            measureKey={measureKey}
            detectedTotal={starsOnMeasuredImage ? starResult?.n_detected ?? null : null}
          />
        </section>

        <section id={ANALYSIS_SECTION.series.id}>
          <TimeSeriesPanel filePath={regionKey} />
        </section>

        <section id={ANALYSIS_SECTION.geometry.id}>
          <ObservationGeometryPanel filePath={regionKey} />
        </section>

        <section id={ANALYSIS_SECTION.catalog.id}>
          <CatalogPanel filePath={regionKey} />
        </section>

        <section id={ANALYSIS_SECTION.targets.id}>
          <TargetsPanel filePath={regionKey} measurePath={effectivePath} />
        </section>

        <section id={ANALYSIS_SECTION.statistics.id}>
          <StatisticsPanel filePath={effectivePath} composite={compositeOnScreen} rgbPath={target.rgbPath} />
        </section>

        <section id={ANALYSIS_SECTION.pixels.id}>
          <PixelTablePanel filePath={effectivePath} measureKey={measureKey} />
        </section>

        <section id={ANALYSIS_SECTION.regions.id}>
          <RegionsPanel filePath={regionKey} measurePath={effectivePath} />
        </section>

        <section id={ANALYSIS_SECTION.profiles.id}>
          <RegionProfilesPanel filePath={regionKey} measurePath={effectivePath} />
        </section>

        <section id={ANALYSIS_SECTION.contours.id}>
          <ContourPanel
            filePath={effectivePath}
            overlayKey={regionKey}
            imageWidth={targetWidth}
            imageHeight={targetHeight}
            measureKey={measureKey}
          />
        </section>

        {showFft && effectivePath && (
          <section id={ANALYSIS_SECTION.fft.id}>
            <FFTPanel filePath={effectivePath} computeFftSpectrum={computeFftSpectrum} />
          </section>
        )}

        {isCube && (
          <section id={ANALYSIS_SECTION.spectrum.id}>
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
          </section>
        )}
        {isCube && (
          <section id={ANALYSIS_SECTION.pv.id}>
            <PvPanel filePath={filePath} fileKey={fileKey} cubeDims={cubeDims} />
          </section>
        )}

        {showDeepZoom && (
          <section id={ANALYSIS_SECTION.deepZoom.id}>
            <TileViewerPanel
              filePath={effectivePath}
              composite={compositeOnScreen}
              rgbPath={rgbPath}
              imageWidth={targetWidth}
              imageHeight={targetHeight}
            />
          </section>
        )}
        <section id={ANALYSIS_SECTION.log.id}>
          <MeasurementLogPanel />
        </section>
      </div>
    </Suspense>
  );
}

export default memo(AnalysisTabInner);
