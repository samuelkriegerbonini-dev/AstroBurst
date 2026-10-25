import { useEffect, useSyncExternalStore } from "react";
import {
  cancelCatalogInstall,
  catalogStatus,
  chooseCatalogFile,
  chooseSirilCatalogDir,
  installCatalog,
  onCatalogInstallProgress,
  removeCatalog,
  zenodoConsent,
  zenodoOffer,
  type CatalogChoice,
  type CatalogDatasetReport,
  type CatalogInstallOutcome,
  type CatalogInstallProgress,
  type CatalogRemoved,
  type ZenodoConsent,
  type ZenodoOffer,
} from "../services/catalogStore";
import { onDataFolderChanged } from "../services/config";
import {
  INSTALL_COMPLETE_STAGE,
  USED_DOWNLOAD_NOTE,
  installReducer,
  installedNote,
  offersZenodo,
  registersZenodoDownload,
  removedNote,
  type InstallAction,
  type InstallMap,
} from "../utils/catalogRows";

export interface CatalogStoreApi {
  status(): Promise<CatalogDatasetReport[]>;
  install(id: string, consent?: ZenodoConsent): Promise<CatalogInstallOutcome>;
  cancel(id: string): Promise<boolean>;
  remove(id: string): Promise<CatalogRemoved>;
  offer(): Promise<ZenodoOffer>;
  useSirilDir(dir: string): Promise<CatalogChoice>;
  useFile(id: string, path: string): Promise<CatalogChoice>;
  onProgress(cb: (progress: CatalogInstallProgress) => void): Promise<() => void>;
}

export interface CatalogState {
  reports: CatalogDatasetReport[] | null;
  installs: InstallMap;
  loading: boolean;
  error: string | null;
}

export interface CatalogStore {
  get(): CatalogState;
  subscribe(cb: () => void): () => void;
  start(): void;
  refresh(): Promise<void>;
  ask(id: string): Promise<void>;
  decline(id: string): void;
  confirm(id: string): Promise<void>;
  cancel(id: string): Promise<void>;
  remove(id: string): Promise<void>;
  useSirilDir(dir: string): Promise<void>;
  useFile(id: string, path: string): Promise<void>;
}

const SERVICE_API: CatalogStoreApi = {
  status: catalogStatus,
  install: installCatalog,
  cancel: cancelCatalogInstall,
  remove: removeCatalog,
  offer: zenodoOffer,
  useSirilDir: chooseSirilCatalogDir,
  useFile: chooseCatalogFile,
  onProgress: onCatalogInstallProgress,
};

const INITIAL: CatalogState = { reports: null, installs: {}, loading: true, error: null };
const PROGRESS_UNAVAILABLE = "Install progress is not available: ";

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function createCatalogStore(api: CatalogStoreApi = SERVICE_API): CatalogStore {
  let state = INITIAL;
  let started = false;
  let statusSeq = 0;
  const listeners = new Set<() => void>();

  const set = (change: Partial<CatalogState>) => {
    state = { ...state, ...change };
    listeners.forEach((l) => l());
  };
  const dispatch = (action: InstallAction) => set({ installs: installReducer(state.installs, action) });
  const reportOf = (id: string) => state.reports?.find((row) => row.id === id);

  async function refresh(): Promise<void> {
    const seq = ++statusSeq;
    set({ loading: true });
    try {
      const reports = await api.status();
      if (seq === statusSeq) set({ reports, error: null, loading: false });
    } catch (e: unknown) {
      if (seq === statusSeq) set({ error: errorText(e), loading: false });
    }
  }

  function progress(update: CatalogInstallProgress): void {
    dispatch({ type: "progress", progress: update });
    if (update.stage === INSTALL_COMPLETE_STAGE) void refresh();
  }

  function start(): void {
    void refresh();
    if (started) return;
    started = true;
    onDataFolderChanged(() => void refresh());
    api.onProgress(progress).catch((e: unknown) => {
      console.error("[AstroBurst] Catalogue install progress:", e);
      set({ error: `${PROGRESS_UNAVAILABLE}${errorText(e)}` });
    });
  }

  async function ask(id: string): Promise<void> {
    const report = reportOf(id);
    if (report && registersZenodoDownload(report)) {
      try {
        await api.useSirilDir(report.folder);
        dispatch({ type: "finished", id, note: USED_DOWNLOAD_NOTE });
      } catch (e: unknown) {
        set({ error: errorText(e) });
      }
      await refresh();
      return;
    }
    dispatch({ type: "ask", id });
    if (!report || !offersZenodo(report)) return;
    try {
      dispatch({ type: "offer", id, offer: await api.offer() });
    } catch (e: unknown) {
      dispatch({ type: "offerFailed", id, message: errorText(e) });
    }
  }

  async function confirm(id: string): Promise<void> {
    const install = state.installs[id];
    if (install?.phase !== "consent") return;
    const report = reportOf(id);
    const zenodo = report !== undefined && offersZenodo(report);
    if (zenodo && !install.offer) return;
    dispatch({ type: "start", id });
    try {
      const outcome = zenodo && install.offer ? await api.install(id, zenodoConsent(install.offer)) : await api.install(id);
      dispatch({ type: "finished", id, note: installedNote(outcome) });
    } catch (e: unknown) {
      dispatch({ type: "failed", id, message: errorText(e) });
    }
    await refresh();
  }

  async function cancel(id: string): Promise<void> {
    const install = state.installs[id];
    const adopted = install?.phase === "running" && install.adopted === true;
    dispatch({ type: "cancel", id });
    try {
      await api.cancel(id);
    } catch (e: unknown) {
      set({ error: errorText(e) });
    }
    if (!adopted) return;
    if (state.installs[id]?.phase === "running") dispatch({ type: "failed", id, message: "cancelled" });
    await refresh();
  }

  async function remove(id: string): Promise<void> {
    try {
      dispatch({ type: "finished", id, note: removedNote(await api.remove(id)) });
    } catch (e: unknown) {
      set({ error: errorText(e) });
    }
    await refresh();
  }

  async function choose(run: () => Promise<CatalogChoice>): Promise<void> {
    try {
      await run();
      await refresh();
    } catch (e: unknown) {
      set({ error: errorText(e) });
    }
  }

  return {
    get: () => state,
    subscribe: (cb) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    start,
    refresh,
    ask,
    decline: (id) => dispatch({ type: "decline", id }),
    confirm,
    cancel,
    remove,
    useSirilDir: (dir) => choose(() => api.useSirilDir(dir)),
    useFile: (id, path) => choose(() => api.useFile(id, path)),
  };
}

export const catalogStore = createCatalogStore();

export function useCatalogStore(store: CatalogStore = catalogStore): CatalogState {
  useEffect(() => store.start(), [store]);
  return useSyncExternalStore(store.subscribe, store.get, store.get);
}
