import { fileStore } from "../../hooks/useFileStore";
import { dockStore } from "../../hooks/useDockLayout";
import { samePath } from "../../utils/processingChain";

export function solveInAstrometry(path: string): boolean {
  const file = fileStore.getFiles().find((f) => samePath(f.path, path) || samePath(f.sourcePath, path));
  if (!file) return false;
  fileStore.selectFile(file.id);
  dockStore.dispatch({ type: "open", tool: "astrometry" });
  return true;
}
