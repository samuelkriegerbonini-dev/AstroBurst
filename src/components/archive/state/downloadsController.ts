import type { MastDownloadInput, MastDownloadItem, MastMissionId } from "../../../shared/types/mast";
import { mastDownload, mastDownloadCancel, mastDownloads, mastErrorOf, onMastDownload } from "../../../services/mast";
import { ingestUnlessLoaded, rowsHolding, showOrOpen } from "../../../hooks/useFileIngest";
import { refreshOverwrittenFiles } from "../../../hooks/refreshOverwrittenFiles";
import { spectrum1dStore } from "../../../hooks/useSpectrum1dStore";
import { appModeStore, type AppMode } from "../../../hooks/useAppMode";
import { FILE_STATUS } from "../../../utils/constants";
import { openableDownload, recoveryInput } from "../logic/downloadView";
import { downloadsStore, pendingOpens, showGroupOf, unsettledDownloads, type DownloadsStore } from "./downloadsStore";

export interface OpenDeps {
  show: (path: string) => boolean;
  load: (path: string) => void;
  setMode: (mode: AppMode) => void;
  reload: (path: string) => void;
}

export async function reloadRewritten(path: string): Promise<void> {
  const spectra = rowsHolding(path).filter((row) => row.status === FILE_STATUS.SPECTRUM);
  await Promise.all([
    refreshOverwrittenFiles({ output_path: path }),
    ...spectra.map((row) => spectrum1dStore.reread(row.path, row.spectrum?.hdu ?? null)),
  ]);
}

const LIVE_DEPS: OpenDeps = {
  show: showOrOpen,
  load: (path) => void ingestUnlessLoaded(path, { keepMode: true }),
  setMode: (mode) => appModeStore.set(mode),
  reload: (path) => void reloadRewritten(path),
};

let requestSeq = 0;
let watching: Promise<void> | null = null;
let autoSettle: (() => void) | null = null;

export function nextDownloadRequestId(now: number = Date.now()): string {
  requestSeq += 1;
  return `dl${now.toString(36)}-${requestSeq}`;
}

export function openDownloaded(item: MastDownloadItem, deps: OpenDeps = LIVE_DEPS): boolean {
  if (item.state !== "done" || !openableDownload(item)) return false;
  deps.setMode("workspace");
  return deps.show(item.path);
}

function settleFinished(store: DownloadsStore, deps: OpenDeps): void {
  const state = store.getState();
  const due = unsettledDownloads(state);
  if (!due.length) return;
  const opens = pendingOpens(state);
  store.dispatch({ type: "settled", ids: due.map((item) => item.id) });
  for (const item of opens) {
    const group = showGroupOf(store.getState(), item);
    if (store.getState().shownGroups.includes(group)) {
      deps.load(item.path);
      continue;
    }
    store.dispatch({ type: "shown", group });
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
  const requestId = nextDownloadRequestId();
  try {
    const snapshot = await mastDownload(requestId, mission, items, open);
    if (open) store.dispatch({ type: "showGroup", group: requestId, ids: requestedItems(snapshot, requestId, items).map((item) => item.id) });
    store.dispatch({ type: "snapshot", items: snapshot });
    store.dispatch({ type: "enqueueFailed", error: null });
    store.dispatch({ type: "drawer", open: true });
    return true;
  } catch (err) {
    store.dispatch({ type: "enqueueFailed", error: mastErrorOf(err) });
    return false;
  }
}

function requestedItems(snapshot: readonly MastDownloadItem[], requestId: string, inputs: readonly MastDownloadInput[]): MastDownloadItem[] {
  const asked = (item: MastDownloadItem) => inputs.some((input) => item.datasetId === input.datasetId && item.filename === input.filename);
  const joined = (item: MastDownloadItem) => item.state === "queued" || item.state === "downloading";
  return snapshot.filter((item) => item.requestId === requestId || (joined(item) && asked(item)));
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
