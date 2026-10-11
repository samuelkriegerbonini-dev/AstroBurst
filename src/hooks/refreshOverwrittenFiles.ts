import { processFitsFull } from "../services/fits";
import { fileStore } from "./useFileStore";
import { overwrittenFileIds, writtenFitsPaths } from "../utils/overwrittenFiles";
import { withVersionParam } from "../utils/processingChain";
import { FILE_STATUS } from "../utils/constants";
import { ingestUnlessLoaded } from "./useFileIngest";

let reloadSeq = 0;

function retryErroredRows(written: readonly string[]): void {
  const errored = fileStore.getFiles().filter((f) => f.status === FILE_STATUS.ERROR);
  for (const path of written) {
    if (overwrittenFileIds(errored, [path]).length > 0) ingestUnlessLoaded(path);
  }
}

export async function refreshOverwrittenFiles(result: unknown): Promise<string[]> {
  const written = writtenFitsPaths(result);
  retryErroredRows(written);
  const ids = overwrittenFileIds(fileStore.getDoneFiles(), written);
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
