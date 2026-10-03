import { useState, useCallback, useEffect, useRef, useMemo, lazy, Suspense, memo } from "react";
import HistogramPanel from "./HistogramPanel";
import RgbStfPanel from "./RgbStfPanel";
import MeasurementBadge from "./MeasurementBadge";
import SectionChips from "./SectionChips";
import { computeFftSpectrum, applyStfRender, computeHistogram } from "../../services/analysis";
import { getOutputDir } from "../../infrastructure/tauri";
import { useHistContext, useCubeContext, useRenderActions, useRenderContext, useRawPixelsContext, useDisplayContext, useDqContext } from "../../context/PreviewContext";
import { useToolHost } from "../../context/ToolHostContext";
import { histogramSkyWindow } from "../../utils/histogramWindow";
import type { HistogramRange } from "../../utils/histogramWindow";
import { ANALYSIS_SECTION, deepZoomAvailable, toolSections, type AnalysisSectionsInput } from "../../utils/analysisSections";
import { useAnalysisMeasure } from "../../hooks/useAnalysisMeasure";
import { cpuStfRenderAllowed, histogramStfLock, rgbStfPanelMode } from "../../utils/analysisTarget";
import type { StfParams, HistogramData } from "../../shared/types";
import TabSpinner from "./TabSpinner";

const FFTPanel = lazy(() => import("./FFTPanel"));
const TileViewerPanel = lazy(() => import("./TileViewerPanel"));
const RegionsPanel = lazy(() => import("../regions/RegionsPanel"));
const RegionProfilesPanel = lazy(() => import("../regions/RegionProfilesPanel"));
const ContourPanel = lazy(() => import("./ContourPanel"));
const StatisticsPanel = lazy(() => import("./StatisticsPanel"));
const PixelTablePanel = lazy(() => import("./PixelTablePanel"));

function ImageTool() {
  const { histData, histDataPath, stfParams, setStfParams } = useHistContext();
  const { isCube, ramp } = useCubeContext();
  const { setStfPreviewUrl } = useRenderActions();
  const { processed } = useRenderContext();
  const { rawPixels, rawPixelsLoading, rgbRawPixels, rgbRawPixelsLoading } = useRawPixelsContext();
  const { display } = useDisplayContext();
  const { excludeDq } = useDqContext();
  const { gpuDisplay } = useToolHost();
  const { target, effectivePath, compositeOnScreen, rgbPath, regionKey, measureKey, targetWidth, targetHeight, filePath } =
    useAnalysisMeasure();

  const stfLock = histogramStfLock({
    stretch: gpuDisplay ? display.stretch : "mtf",
    compositeOnScreen,
    previewOnly: target.displayed.previewOnly,
  });

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

  const rgbStfMode = rgbStfPanelMode({
    compositeOnScreen,
    hasRgbRawPixels: rgbRawPixels !== null,
    displayReferred: rgbRawPixels?.displayReferred === true,
  });
  const measurementBadge = useMemo(() => <MeasurementBadge />, []);

  const showFft = Boolean(effectivePath) && !isCube && (targetWidth ?? 0) >= 64;
  const showDeepZoom = Boolean(effectivePath) && deepZoomAvailable(targetWidth, targetHeight);
  const sectionsInput: AnalysisSectionsInput = {
    hasHistogram: histData !== null && histData.bins.length > 0,
    isCube,
    isRamp: ramp !== null,
    showFft,
    showDeepZoom,
  };
  const sections = toolSections("image", sectionsInput);

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
        <SectionChips label="Image" sections={sections} />

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

        <section id={ANALYSIS_SECTION.statistics.id}>
          <StatisticsPanel filePath={effectivePath} composite={compositeOnScreen} rgbPath={rgbPath} />
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
      </div>
    </Suspense>
  );
}

export default memo(ImageTool);
