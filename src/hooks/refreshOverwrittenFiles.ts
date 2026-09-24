import { processFitsFull } from "../services/fits";
import { fileStore } from "./useFileStore";
import { overwrittenFileIds, writtenFitsPaths } from "../utils/overwrittenFiles";
import { withVersionParam } from "../utils/processingChain";

let reloadSeq = 0;

export async function refreshOverwrittenFiles(result: unknown): Promise<string[]> {
  const ids = overwrittenFileIds(fileStore.getDoneFiles(), writtenFitsPaths(result));
  await Promise.all(
    ids.map(async (id) => {
      const file = fileStore.getFile(id);
      if (!file) return;
      try {
        const fresh = await processFitsFull(file.path);
        reloadSeq += 1;
        const previewUrl = fresh.previewUrl ? withVersionParam(fresh.previewUrl, reloadSeq) : fresh.previewUrl;
        fileStore.fileReloaded(id, { ...fresh, previewUrl });
      } catch (e) {
        console.warn(`[AstroBurst] ${file.path} was rewritten by a step but could not be reloaded:`, e);
      }
    }),
  );
  return ids;
}
