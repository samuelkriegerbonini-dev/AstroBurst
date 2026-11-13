import { lazy, Suspense, memo } from "react";
import { ANALYSIS_SECTION, toolSections } from "../../utils/analysisSections";
import { useAnalysisMeasure } from "../../hooks/useAnalysisMeasure";
import { useStarDetection } from "../../hooks/useStarDetectionStore";
import type { Star } from "./PlateSolvePanel";
import TabSpinner from "./TabSpinner";
import SectionChips, { SectionTipNote } from "./SectionChips";
import { useSectionTip } from "./useSectionTip";

const PhotometryPanel = lazy(() => import("./PhotometryPanel"));
const PhotometryTablePanel = lazy(() => import("./PhotometryTablePanel"));
const TimeSeriesPanel = lazy(() => import("./TimeSeriesPanel"));
const ForcedPhotometryPanel = lazy(() => import("./ForcedPhotometryPanel"));
const DepthPanel = lazy(() => import("./DepthPanel"));
const ResolutionPanel = lazy(() => import("./ResolutionPanel"));
const CatalogComparePanel = lazy(() => import("./CatalogComparePanel"));

const EMPTY_STARS: Star[] = [];
const SECTIONS = toolSections("photometry", { hasHistogram: false, isCube: false, showFft: false, showDeepZoom: false });
const SECTION_TIPS = {
  [ANALYSIS_SECTION.photometry.id]: {
    title: "Aperture photometry",
    body: "Turn on Measure on image click, select Crosshair in the viewer, then click a star. Leave the aperture fields blank for automatic sizes or enter radii in pixels.",
  },
  [ANALYSIS_SECTION.depth.id]: {
    title: "Image depth",
    body: "How faint a source this image can show: the N sigma limit of empty apertures, and how much the errors of the file underestimate.",
  },
  [ANALYSIS_SECTION.resolution.id]: {
    title: "Resolved or not",
    body: "The PSF of the image, the redshift and the thresholds that decide whether a source is resolved.",
  },
  [ANALYSIS_SECTION.compare.id]: {
    title: "Catalogue comparison",
    body: "Measures a catalogue table you saved on this image, at the catalogue positions and in the catalogue aperture, and reports measured over catalogue with its scatter and N.",
  },
};

function PhotometryTool() {
  const { effectivePath, regionKey, measureKey, detectionScope, starsOnMeasuredImage } = useAnalysisMeasure();
  const { result: starResult } = useStarDetection(detectionScope);
  const sectionTip = useSectionTip(SECTIONS, SECTION_TIPS);

  const stars = starResult?.stars ?? EMPTY_STARS;
  const tableStars = starsOnMeasuredImage ? stars : EMPTY_STARS;

  return (
    <Suspense fallback={<TabSpinner />}>
      <div className="flex flex-col gap-3 p-3">
        <SectionChips label="Photometry" sections={SECTIONS} tip={sectionTip} />
        <section id={ANALYSIS_SECTION.photometry.id}>
          <SectionTipNote tip={sectionTip} sectionId={ANALYSIS_SECTION.photometry.id} />
          <PhotometryPanel filePath={effectivePath} measureKey={measureKey} />
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

        <section id={ANALYSIS_SECTION.forced.id}>
          <ForcedPhotometryPanel filePath={effectivePath} measureKey={measureKey} />
        </section>

        <section id={ANALYSIS_SECTION.depth.id}>
          <SectionTipNote tip={sectionTip} sectionId={ANALYSIS_SECTION.depth.id} />
          <DepthPanel filePath={effectivePath} measureKey={measureKey} />
        </section>

        <section id={ANALYSIS_SECTION.resolution.id}>
          <SectionTipNote tip={sectionTip} sectionId={ANALYSIS_SECTION.resolution.id} />
          <ResolutionPanel filePath={effectivePath} measureKey={measureKey} />
        </section>

        <section id={ANALYSIS_SECTION.compare.id}>
          <SectionTipNote tip={sectionTip} sectionId={ANALYSIS_SECTION.compare.id} />
          <CatalogComparePanel filePath={effectivePath} measureKey={measureKey} />
        </section>
      </div>
    </Suspense>
  );
}

export default memo(PhotometryTool);
