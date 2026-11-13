import { fileStore } from "./useFileStore";
import { photometryStatusStore } from "./usePhotometryStatus";
import type { PlaneInfo, ProcessedFile } from "../shared/types";

export function applyLayerChange(file: Pick<ProcessedFile, "id" | "path" | "result">, plane: PlaneInfo): void {
  if (file.result === null) return;
  fileStore.fileReloaded(file.id, { ...file.result, plane });
  photometryStatusStore.forget(file.path);
}
