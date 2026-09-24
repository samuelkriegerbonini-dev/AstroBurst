import { useMemo } from "react";
import { useCompositePreview } from "../context/CompositeContext";
import { useDisplayedImage, useFileContext, type DisplayedImage } from "../context/PreviewContext";
import { describeMeasurementSource, isCompositeOnScreen, isFileRgbView, rgbMeasurePath, type MeasurementSource } from "../utils/analysisTarget";

export interface AnalysisTarget {
  composite: boolean;
  fileRgbView: boolean;
  rgbPath: string | null;
  path: string | null;
  dimensions: [number, number] | null;
  displayed: DisplayedImage;
  fileIsRgb: boolean;
  fileName: string | null;
}

export function useAnalysisTarget(): AnalysisTarget {
  const { file } = useFileContext();
  const displayed = useDisplayedImage();
  const { compositePreviewUrl } = useCompositePreview();
  const fileIsRgb = file?.result?.is_rgb === true;
  const filePath = file?.path ?? null;
  const fileName = file?.name ?? null;
  const fileDims = file?.result?.dimensions ?? null;
  const filePreviewUrl = file?.result?.previewUrl ?? null;
  const composite = isCompositeOnScreen({
    compositePreviewUrl,
    fileIsRgb,
    filePreviewUrl,
    hasProcessed: displayed.isProcessed,
  });
  const fileRgbView = isFileRgbView({ compositePreviewUrl, fileIsRgb, filePreviewUrl });
  const rgbPath = rgbMeasurePath({ composite, fileRgbView, filePath });
  return useMemo(
    () => ({
      composite,
      fileRgbView,
      rgbPath,
      path: composite ? filePath : displayed.path,
      dimensions: composite ? fileDims : displayed.dimensions,
      displayed,
      fileIsRgb,
      fileName,
    }),
    [composite, fileRgbView, rgbPath, filePath, fileDims, displayed, fileIsRgb, fileName],
  );
}

export function useMeasurementSource(measuresComposite: boolean): MeasurementSource | null {
  const target = useAnalysisTarget();
  const { composite, displayed, rgbPath, fileName } = target;
  return useMemo(
    () =>
      describeMeasurementSource({
        measuresComposite,
        compositeOnScreen: composite,
        measuresFilePlanes: rgbPath !== null,
        fileName,
        processedLabel: displayed.isProcessed ? displayed.label : null,
        previewOnly: displayed.previewOnly,
      }),
    [measuresComposite, composite, rgbPath, fileName, displayed],
  );
}
