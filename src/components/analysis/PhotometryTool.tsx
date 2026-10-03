import { lazy, Suspense, memo } from "react";
import { ANALYSIS_SECTION } from "../../utils/analysisSections";
import { useAnalysisMeasure } from "../../hooks/useAnalysisMeasure";
import { useStarDetection } from "../../hooks/useStarDetectionStore";
import type { Star } from "./PlateSolvePanel";
import TabSpinner from "./TabSpinner";

const PhotometryPanel = lazy(() => import("./PhotometryPanel"));
const PhotometryTablePanel = lazy(() => import("./PhotometryTablePanel"));
const TimeSeriesPanel = lazy(() => import("./TimeSeriesPanel"));

const EMPTY_STARS: Star[] = [];

function PhotometryTool() {
  const { effectivePath, regionKey, measureKey, detectionScope, starsOnMeasuredImage } = useAnalysisMeasure();
  const { result: starResult } = useStarDetection(detectionScope);

  const stars = starResult?.stars ?? EMPTY_STARS;
  const tableStars = starsOnMeasuredImage ? stars : EMPTY_STARS;

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
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
      </div>
    </Suspense>
  );
}

export default memo(PhotometryTool);
