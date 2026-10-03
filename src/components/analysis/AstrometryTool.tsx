import { useCallback, useMemo, lazy, Suspense, memo } from "react";
import MeasurementBadge from "./MeasurementBadge";
import SectionChips from "./SectionChips";
import { detectStars, detectStarsComposite } from "../../services/analysis";
import { useRenderActions, useStarOverlayContext } from "../../context/PreviewContext";
import { ANALYSIS_SECTION, toolSections } from "../../utils/analysisSections";
import { useAnalysisMeasure } from "../../hooks/useAnalysisMeasure";
import { starDetectionStore, useStarDetection } from "../../hooks/useStarDetectionStore";
import { SOLVED_WCS_LABEL } from "../../utils/solvedWcs";
import type { WriteSolvedWcsResult } from "../../shared/types/astrometry";
import type { Star } from "./PlateSolvePanel";
import TabSpinner from "./TabSpinner";

const PlateSolvePanel = lazy(() => import("./PlateSolvePanel"));
const ObservationGeometryPanel = lazy(() => import("./ObservationGeometryPanel"));
const CatalogPanel = lazy(() => import("./CatalogPanel"));
const TargetsPanel = lazy(() => import("./TargetsPanel"));

const EMPTY_STARS: Star[] = [];
const ASTROMETRY_SECTIONS = toolSections("astrometry", { hasHistogram: false, isCube: false, showFft: false, showDeepZoom: false });

function AstrometryTool() {
  const { publishProcessed } = useRenderActions();
  const { starOverlayRef } = useStarOverlayContext();
  const { effectivePath, compositeOnScreen, rgbPath, regionKey, targetWidth, targetHeight, filePath, fileKey, detectionScope, starsOnMeasuredImage } =
    useAnalysisMeasure();
  const { result: starResult, loading: starLoading, error: detectError } = useStarDetection(detectionScope);

  const handleDetectStars = useCallback(
    async (sigma: number) => {
      const seq = starDetectionStore.begin(detectionScope);
      try {
        const result = compositeOnScreen
          ? await detectStarsComposite(sigma, 200, rgbPath)
          : effectivePath
            ? await detectStars(effectivePath, sigma, 200)
            : null;
        starDetectionStore.commit(seq, result);
      } catch (e) {
        console.error("Star detection failed:", e);
        starDetectionStore.fail(seq, e instanceof Error ? e.message : String(e));
      }
    },
    [detectionScope, effectivePath, compositeOnScreen, rgbPath],
  );

  const handleWcsWritten = useCallback(
    (res: WriteSolvedWcsResult) => {
      if (!fileKey || !filePath) return;
      publishProcessed(fileKey, {
        fitsPath: res.fits_path,
        previewUrl: res.previewUrl ?? null,
        dimensions: res.dimensions,
        label: SOLVED_WCS_LABEL,
        kind: "processing",
        inputPath: filePath,
      });
    },
    [publishProcessed, fileKey, filePath],
  );

  const stars = starResult?.stars ?? EMPTY_STARS;
  const compositeMeasurementBadge = useMemo(() => <MeasurementBadge measuresComposite />, []);

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
        <SectionChips label="Astrometry" sections={ASTROMETRY_SECTIONS} />

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
            annotationsOnView={starsOnMeasuredImage}
            onWcsWritten={handleWcsWritten}
          />
        </section>

        <section id={ANALYSIS_SECTION.geometry.id}>
          <ObservationGeometryPanel filePath={regionKey} />
        </section>

        <section id={ANALYSIS_SECTION.catalog.id}>
          <CatalogPanel filePath={regionKey} measurePath={effectivePath} />
        </section>

        <section id={ANALYSIS_SECTION.targets.id}>
          <TargetsPanel filePath={regionKey} measurePath={effectivePath} />
        </section>
      </div>
    </Suspense>
  );
}

export default memo(AstrometryTool);
