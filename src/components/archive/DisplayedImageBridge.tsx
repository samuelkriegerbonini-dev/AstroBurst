import { useEffect } from "react";
import { useDisplayedImage, useFileContext } from "../../context/PreviewContext";
import { displayedImageRef, displayedImageStore } from "./state/displayedImage";

export default function DisplayedImageBridge() {
  const { file } = useFileContext();
  const displayed = useDisplayedImage();
  const ref = displayedImageRef(file?.path ?? null, displayed.path);
  const filePath = ref?.filePath ?? null;
  const projectionPath = ref?.projectionPath ?? null;
  useEffect(() => {
    displayedImageStore.set(filePath !== null && projectionPath !== null ? { filePath, projectionPath } : null);
  }, [filePath, projectionPath]);
  useEffect(() => () => displayedImageStore.set(null), []);
  return null;
}
