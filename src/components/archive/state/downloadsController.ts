import type { AstroFile } from "../../../shared/types";
import type { MastDownloadInput, MastDownloadItem, MastMissionId } from "../../../shared/types/mast";
import { mastDownload, mastDownloadCancel, mastDownloads, mastErrorOf, onMastDownload } from "../../../services/mast";
import { ingestUnlessLoaded, openUnlessLoaded, showFile } from "../../../hooks/useFileIngest";
import { fileStore } from "../../../hooks/useFileStore";
import { refreshOverwrittenFiles } from "../../../hooks/refreshOverwrittenFiles";
import { spectrum1dStore } from "../../../hooks/useSpectrum1dStore";
import { appModeStore, type AppMode } from "../../../hooks/useAppMode";
import { FILE_STATUS } from "../../../utils/constants";
import { overwrittenFileIds } from "../../../utils/overwrittenFiles";
import { astroFileFromPath } from "../../../utils/validation";
import { openableDownload, recoveryInput } from "../logic/downloadView";
import { downloadsStore, pendingOpens, unsettledDownloads, type DownloadsStore } from "./downloadsStore";

export interface OpenDeps {
  ingest: (files: AstroFile[], show?: boolean) => boolean;
  setMode: (mode: AppMode) => void;
  reload: (path: string) => void;
}

function rowsHolding(path: string) {
  return overwrittenFileIds(fileStore.getFiles(), [path]).flatMap((id) => fileStore.getFile(id) ?? []);
}

function showOrLoad(path: string, show: boolean): boolean {
  const loaded = rowsHolding(path).find((row) => row.status !== FILE_STATUS.ERROR);
  if (!loaded) return show ? openUnlessLoaded(path) : ingestUnlessLoaded(path, { keepMode: true });
  if (show) showFile(loaded);
  return true;
}

export async function reloadRewritten(path: string): Promise<void> {
  const spectra = rowsHolding(path).filter((row) => row.status === FILE_STATUS.SPECTRUM);
  await Promise.all([
    refreshOverwrittenFiles({ output_path: path }),
    ...spectra.map((row) => spectrum1dStore.reread(row.path, row.spectrum?.hdu ?? null)),
  ]);
}

const LIVE_DEPS: OpenDeps = {
  ingest: (files, show = true) => files.map((file) => showOrLoad(file.path, show)).some(Boolean),
  setMode: (mode) => appModeStore.set(mode),
  reload: (path) => void reloadRewritten(path),
};

let requestSeq = 0;
const shownRequests = new Set<string>();
let watching: Promise<void> | null = null;
let autoSettle: (() => void) | null = null;

export function nextDownloadRequestId(now: number = Date.now()): string {
  requestSeq += 1;
  return `dl${now.toString(36)}-${requestSeq}`;
}

export function openDownloaded(item: MastDownloadItem, deps: OpenDeps = LIVE_DEPS): boolean {
  if (item.state !== "done" || !openableDownload(item)) return false;
  deps.setMode("workspace");
  return deps.ingest([astroFileFromPath(item.path)]);
}

function settleFinished(store: DownloadsStore, deps: OpenDeps): void {
  const state = store.getState();
  const due = unsettledDownloads(state);
  if (!due.length) return;
  const opens = pendingOpens(state);
  store.dispatch({ type: "settled", ids: due.map((item) => item.id) });
  for (const item of opens) {
    if (shownRequests.has(item.requestId)) {
      deps.ingest([astroFileFromPath(item.path)], false);
      continue;
    }
    shownRequests.add(item.requestId);
    openDownloaded(item, deps);
  }
  for (const item of due) if (!item.reused) deps.reload(item.path);
}

export function watchDownloads(store: DownloadsStore = downloadsStore, deps: OpenDeps = LIVE_DEPS): Promise<void> {
  if (watching) return watching;
  autoSettle ??= store.subscribe(() => settleFinished(store, deps));
  watching = (async () => {
    try {
      await onMastDownload((item) => store.dispatch({ type: "item", item }));
      const current = await mastDownloads();
      store.dispatch({ type: "settled", ids: current.filter((item) => item.state === "done").map((item) => item.id) });
      store.dispatch({ type: "snapshot", items: current });
    } catch {
      watching = null;
    }
  })();
  return watching;
}

export async function startDownloads(
  mission: MastMissionId,
  items: readonly MastDownloadInput[],
  open: boolean,
  store: DownloadsStore = downloadsStore,
): Promise<boolean> {
  if (!items.length) return false;
  void watchDownloads(store);
  try {
    const snapshot = await mastDownload(nextDownloadRequestId(), mission, items, open);
    store.dispatch({ type: "snapshot", items: snapshot });
    store.dispatch({ type: "enqueueFailed", error: null });
    store.dispatch({ type: "drawer", open: true });
    return true;
  } catch (err) {
    store.dispatch({ type: "enqueueFailed", error: mastErrorOf(err) });
    return false;
  }
}

export async function cancelDownload(item: MastDownloadItem, store: DownloadsStore = downloadsStore): Promise<void> {
  try {
    await mastDownloadCancel(item.id);
    store.dispatch({ type: "cancelFailed", error: null });
  } catch (err) {
    store.dispatch({ type: "cancelFailed", error: mastErrorOf(err) });
  }
}

export function recoverDownload(item: MastDownloadItem, store: DownloadsStore = downloadsStore): Promise<boolean> {
  return startDownloads(item.mission, [recoveryInput(item)], item.open, store);
}

export async function revealDownload(item: MastDownloadItem): Promise<void> {
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(item.path);
}

export function setDrawerOpen(open: boolean, store: DownloadsStore = downloadsStore): void {
  store.dispatch({ type: "drawer", open });
}
