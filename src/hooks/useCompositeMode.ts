import { useCompositePreview } from "../context/CompositeContext";
import { useFileContext } from "../context/PreviewContext";
import { isFileRgbView } from "../utils/analysisTarget";

export interface CompositeModeInput {
  compositePreviewUrl: string | null;
  fileIsRgb: boolean;
  filePreviewUrl: string | null;
}

export function isCompositeMode(input: CompositeModeInput): boolean {
  return input.compositePreviewUrl !== null && !isFileRgbView(input);
}

export function useCompositeMode(): boolean {
  const { file } = useFileContext();
  const { compositePreviewUrl } = useCompositePreview();
  return isCompositeMode({
    compositePreviewUrl,
    fileIsRgb: !!file?.result?.is_rgb,
    filePreviewUrl: file?.result?.previewUrl ?? null,
  });
}
