import { useCompositePreview } from "../context/CompositeContext";
import { useFileContext, useRenderContext } from "../context/PreviewContext";

export function useRegionKey(): string | null {
  const { file } = useFileContext();
  const { activeImagePath } = useRenderContext();
  const { isShowingComposite } = useCompositePreview();
  if (isShowingComposite && activeImagePath) return activeImagePath;
  return file?.path ?? null;
}
