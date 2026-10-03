import { useMemo } from "react";
import { fileKeyOf, useFileContext, useRenderContext } from "../context/PreviewContext";
import { useCompositePreview } from "../context/CompositeContext";
import { analysisMeasureKey, detectedStarsOnMeasuredImage, starDetectionScope } from "../utils/analysisTarget";
import { useAnalysisTarget, type AnalysisTarget } from "./useAnalysisTarget";
import { useRegionKey } from "./useRegionKey";

export interface AnalysisMeasure {
  target: AnalysisTarget;
  effectivePath: string | null;
  compositeOnScreen: boolean;
  rgbPath: string | null;
  regionKey: string | null;
  measureKey: string | null;
  targetWidth: number | undefined;
  targetHeight: number | undefined;
  filePath: string | undefined;
  fileKey: string | null;
  detectionScope: string;
  starsOnMeasuredImage: boolean;
}

export function useAnalysisMeasure(): AnalysisMeasure {
  const { file } = useFileContext();
  const { processed, processedVersion } = useRenderContext();
  const target = useAnalysisTarget();
  const regionKey = useRegionKey();
  const { compositeVersion } = useCompositePreview();
  const processedFitsPath = processed?.fitsPath ?? null;
  const filePath = file?.path;
  const fileKey = fileKeyOf(file);
  return useMemo(() => {
    const effectivePath = target.path;
    const compositeOnScreen = target.composite;
    const rgbPath = target.rgbPath;
    return {
      target,
      effectivePath,
      compositeOnScreen,
      rgbPath,
      regionKey,
      measureKey: analysisMeasureKey({ path: effectivePath, composite: compositeOnScreen, processedFitsPath, processedVersion }),
      targetWidth: target.dimensions?.[0],
      targetHeight: target.dimensions?.[1],
      filePath,
      fileKey,
      detectionScope: starDetectionScope({ path: effectivePath, composite: compositeOnScreen, rgbPath, compositeVersion }),
      starsOnMeasuredImage: detectedStarsOnMeasuredImage({ compositeOnScreen, measuresFilePlanes: rgbPath !== null }),
    };
  }, [target, regionKey, processedFitsPath, processedVersion, filePath, fileKey, compositeVersion]);
}
