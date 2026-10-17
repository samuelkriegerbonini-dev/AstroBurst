import type { MastError, MastImageSet, MastMission, MastMissionId, MastPreviewRef, MastProductList, MastProgressEvent } from "../../../shared/types/mast";
import { mastErrorOf } from "../../../services/mast";
import { PREVIEW_DWELL_MS, entriesInGroup, largeNeeded, previewKey, previewsAllowed, stripCover, stripThumbKeys, type PreviewSize } from "../logic/detailView";
import { listId, previewId, productsId } from "../logic/requestIds";
import { detailEntry, detailShown, detailTabOf, imageSetKey, largeEntry, selectedRow, type ArchiveState } from "./archiveReducer";
import type { ArchiveStore } from "./archiveStore";
import type { InflightKind } from "./inflight";

export interface DetailServices {
  imageList(requestId: string, mission: MastMissionId, datasetId: string): Promise<MastImageSet>;
  preview(requestId: string, mission: MastMissionId, datasetId: string, filename: string, size: PreviewSize): Promise<MastPreviewRef>;
  products(requestId: string, mission: MastMissionId, datasetIds: string[], refresh: boolean): Promise<MastProductList>;
  cancel(target: { requestIds?: string[]; prefix?: string }): Promise<number>;
  previewUrl(path: string): Promise<string>;
}

export interface DetailClock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface DetailEffectsDeps {
  store: ArchiveStore;
  services: DetailServices;
  clock: DetailClock;
  call<T>(kind: InflightKind, fn: () => Promise<T>): Promise<T>;
  onStripDemand(keys: string[]): void;
}

export interface DetailEffects {
  sync(): void;
  onProgress(event: MastProgressEvent): void;
  cancelPreview(size: PreviewSize): void;
  retry(): void;
  retryLarge(): void;
  refreshProducts(): void;
  cancelProducts(): void;
  dispose(): void;
}

interface Target {
  mission: MastMissionId;
  datasetId: string;
  filename: string;
  key: string;
}

const CANCELLED_BY_USER: MastError = { code: "mast_cancelled", message: "Cancelled", context: {} };

function sameKeys(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

function inFlight(stage: string | undefined): boolean {
  return stage === "download" || stage === "decode";
}

export function createDetailEffects(deps: DetailEffectsDeps): DetailEffects {
  const { store, services, clock, call, onStripDemand } = deps;
  const get = () => store.getState();
  const dispatch = store.dispatch;
  const attempts = new Map<string, number>();
  let attemptSeq = 0;
  let strip: string[] = [];
  let paneSel: string | null = null;
  let paneSelAt = 0;
  let prodWant: string | null = null;
  let prodDue = 0;
  let timer: { at: number; handle: unknown } | null = null;
  let syncing = false;
  let again = false;

  function begin(key: string): number {
    attemptSeq += 1;
    attempts.set(key, attemptSeq);
    return attemptSeq;
  }

  function current(key: string, attempt: number): boolean {
    return attempts.get(key) === attempt;
  }

  function cancelRequest(requestId: string): void {
    services.cancel({ requestIds: [requestId] }).catch(() => 0);
  }

  function missionOf(s: ArchiveState): MastMission | undefined {
    const id = s.results?.mission;
    return id ? s.missions.find((m) => m.id === id) : undefined;
  }

  function rowAllowed(s: ArchiveState, mission: MastMission, datasetId: string): boolean {
    const row = s.results?.rows.find((r) => r.datasetId === datasetId);
    return !!row && previewsAllowed(row, mission);
  }

  function startList(s: ArchiveState, mission: MastMissionId, datasetId: string, key: string): void {
    const attempt = begin(`list:${key}`);
    const requestId = listId(s.generation, datasetId, attempt);
    dispatch({ type: "imageListStarted", key, requestId, at: clock.now() });
    call("other", () => services.imageList(requestId, mission, datasetId)).then(
      (set) => {
        if (current(`list:${key}`, attempt) && get().lists[key]?.requestId === requestId) dispatch({ type: "imageListLoaded", key, set });
      },
      (err) => {
        if (!current(`list:${key}`, attempt)) return;
        const e = mastErrorOf(err);
        if (e.code === "mast_cancelled") dispatch({ type: "imageListCancelled", key, requestId });
        else dispatch({ type: "imageListFailed", key, requestId, error: e, at: clock.now() });
      },
    );
  }

  function startPreview(s: ArchiveState, target: Target, size: PreviewSize): void {
    const key = target.key;
    const attempt = begin(`preview:${key}`);
    const requestId = previewId(s.generation, target.datasetId, target.filename, size, attempt);
    dispatch({ type: "previewStarted", key, requestId, at: clock.now() });
    call("other", () => services.preview(requestId, target.mission, target.datasetId, target.filename, size))
      .then(async (ref) => {
        const url = await services.previewUrl(ref.path);
        if (current(`preview:${key}`, attempt)) dispatch({ type: "previewLoaded", key, requestId, ref, url });
      })
      .catch((err) => {
        if (!current(`preview:${key}`, attempt)) return;
        const e = mastErrorOf(err);
        if (e.code === "mast_cancelled") {
          if (inFlight(get().previews[key]?.stage)) dispatch({ type: "previewCancelled", key, requestId });
          return;
        }
        dispatch({ type: "previewFailed", key, requestId, error: e, at: clock.now() });
      });
  }

  function startProducts(s: ArchiveState, mission: MastMissionId, datasetId: string, refresh: boolean): void {
    const key = imageSetKey(mission, datasetId);
    const requestId = productsId(s.generation, datasetId);
    const attempt = begin(`products:${key}`);
    dispatch({ type: "productsStarted", key, requestId, at: clock.now() });
    call("other", () => services.products(requestId, mission, [datasetId], refresh)).then(
      (list) => {
        if (current(`products:${key}`, attempt)) dispatch({ type: "productsLoaded", key, requestId, list });
      },
      (err) => {
        if (!current(`products:${key}`, attempt)) return;
        const e = mastErrorOf(err);
        if (e.code === "mast_cancelled") dispatch({ type: "productsCancelled", key, requestId, byUser: false });
        else dispatch({ type: "productsFailed", key, requestId, error: e });
      },
    );
  }

  function target(mission: MastMission, datasetId: string, filename: string | null, size: PreviewSize): Target | null {
    if (!filename) return null;
    return { mission: mission.id, datasetId, filename, key: previewKey(mission.id, datasetId, filename, size) };
  }

  function wake(at: number | null): void {
    if (timer && timer.at === at) return;
    if (timer) clock.clearTimeout(timer.handle);
    timer = null;
    if (at === null) return;
    const handle = clock.setTimeout(() => {
      timer = null;
      sync();
    }, Math.max(0, at - clock.now()));
    timer = { at, handle };
  }

  function step(): void {
    const s = get();
    if (s.run) {
      wake(null);
      return;
    }
    const now = clock.now();
    const mission = missionOf(s);
    const idle = !!s.results && !!mission;
    const shown = detailShown(s);
    const tab = detailTabOf(s);
    const sel = s.selectedId;
    let nextWake: number | null = null;
    const dueAt = (at: number) => {
      nextWake = nextWake === null ? at : Math.min(nextWake, at);
    };

    const selKey = `${sel}|${s.detail.imageKey}|${s.detail.group}`;
    if (selKey !== paneSel) {
      paneSel = selKey;
      paneSelAt = now;
    }

    const wantedLists = new Set<string>();
    const listFor = (datasetId: string | null) => {
      if (!idle || !mission || !datasetId || !rowAllowed(s, mission, datasetId)) return;
      const key = imageSetKey(mission.id, datasetId);
      wantedLists.add(key);
      if (!s.imageSets[key] && !s.lists[key]) startList(s, mission.id, datasetId, key);
    };
    if (s.large.open) listFor(s.large.datasetId);
    if (shown) listFor(sel);
    for (const [key, list] of Object.entries(s.lists)) {
      if (wantedLists.has(key)) continue;
      if (list.error) {
        dispatch({ type: "imageListRetried", key });
        continue;
      }
      attempts.delete(`list:${key}`);
      cancelRequest(list.requestId);
      dispatch({ type: "imageListCancelled", key, requestId: list.requestId });
    }

    const previewsTab = idle && shown && tab === "previews" && !!mission && !!sel && rowAllowed(s, mission, sel);
    const set = previewsTab && mission && sel ? s.imageSets[imageSetKey(mission.id, sel)] : undefined;
    const entry = previewsTab ? detailEntry(s) : null;
    const nextStrip = set && entry && sel ? stripThumbKeys(sel, entriesInGroup(set, entry.group), stripCover(set, s.thumbRefs[sel], s.thumbs.inflight.includes(sel))) : [];
    if (!sameKeys(nextStrip, strip)) {
      strip = nextStrip;
      onStripDemand(nextStrip);
    }

    const wanted = new Set<string>();
    const pane = previewsTab && mission && sel && entry ? target(mission, sel, entry.previewFilename, "pane") : null;
    if (pane) wanted.add(pane.key);
    let largePane: Target | null = null;
    let largeCopy: Target | null = null;
    if (idle && mission && s.large.open && s.large.datasetId && rowAllowed(s, mission, s.large.datasetId)) {
      const le = largeEntry(s);
      largePane = le ? target(mission, s.large.datasetId, le.previewFilename, "pane") : null;
      largeCopy = le ? target(mission, s.large.datasetId, le.previewFilename, "large") : null;
      if (largePane) wanted.add(largePane.key);
      if (largeCopy) wanted.add(largeCopy.key);
    }
    for (const [key, p] of Object.entries(s.previews)) {
      if (!inFlight(p.stage) || wanted.has(key)) continue;
      attempts.delete(`preview:${key}`);
      cancelRequest(p.requestId);
      dispatch({ type: "previewCancelled", key, requestId: p.requestId });
    }
    if (pane && !s.previews[pane.key]) {
      const due = paneSelAt + PREVIEW_DWELL_MS;
      if (now >= due) startPreview(s, pane, "pane");
      else dueAt(due);
    }
    if (largePane && largeCopy) {
      const paneState = s.previews[largePane.key];
      if (!paneState) startPreview(s, largePane, "pane");
      else if (paneState.stage === "ready" && paneState.ref && largeNeeded(paneState.ref) && !s.previews[largeCopy.key]) startPreview(s, largeCopy, "large");
    }

    const selectedKey = mission && sel ? imageSetKey(mission.id, sel) : null;
    for (const [key, p] of Object.entries(s.products)) {
      if (p.stage !== "loading" || key === selectedKey) continue;
      attempts.delete(`products:${key}`);
      cancelRequest(p.requestId);
      dispatch({ type: "productsCancelled", key, requestId: p.requestId, byUser: false });
    }
    const want = idle && shown && tab === "products" && mission?.capabilities.products && selectedKey ? selectedKey : null;
    if (want !== prodWant) {
      prodDue = prodWant === null ? now : now + PREVIEW_DWELL_MS;
      prodWant = want;
    }
    if (want && mission && sel && !s.products[want]) {
      if (now >= prodDue) startProducts(s, mission.id, sel, false);
      else dueAt(prodDue);
    }

    wake(nextWake);
  }

  function sync(): void {
    if (syncing) {
      again = true;
      return;
    }
    syncing = true;
    try {
      do {
        again = false;
        step();
      } while (again);
    } finally {
      syncing = false;
    }
  }

  function selectedPaneKey(s: ArchiveState, size: PreviewSize): string | null {
    const mission = missionOf(s);
    if (!mission) return null;
    if (size === "large") {
      const le = largeEntry(s);
      return le?.previewFilename && s.large.datasetId ? previewKey(mission.id, s.large.datasetId, le.previewFilename, "large") : null;
    }
    const entry = detailEntry(s);
    return entry?.previewFilename && s.selectedId ? previewKey(mission.id, s.selectedId, entry.previewFilename, "pane") : null;
  }

  function stopPreview(key: string | null): void {
    if (!key) return;
    const p = get().previews[key];
    if (!p || !inFlight(p.stage)) return;
    attempts.delete(`preview:${key}`);
    cancelRequest(p.requestId);
    dispatch({ type: "previewFailed", key, requestId: p.requestId, error: CANCELLED_BY_USER, at: clock.now() });
  }

  function retryFailedPreview(key: string | null): void {
    if (key && get().previews[key]?.stage === "failed") dispatch({ type: "previewRetried", key });
  }

  return {
    sync,
    onProgress(event) {
      dispatch({ type: "previewProgress", requestId: event.requestId, received: event.received, total: event.total, stage: event.stage });
    },
    cancelPreview(size) {
      const s = get();
      if (size === "large") {
        const le = largeEntry(s);
        const mission = missionOf(s);
        const paneKey = le?.previewFilename && mission && s.large.datasetId ? previewKey(mission.id, s.large.datasetId, le.previewFilename, "pane") : null;
        stopPreview(paneKey);
      }
      stopPreview(selectedPaneKey(s, size));
    },
    retry() {
      const s = get();
      const mission = missionOf(s);
      const row = selectedRow(s);
      if (!mission || !row) return;
      const key = imageSetKey(mission.id, row.datasetId);
      if (s.lists[key]?.error) dispatch({ type: "imageListRetried", key });
      const tab = detailTabOf(s);
      if (tab === "previews") {
        retryFailedPreview(selectedPaneKey(s, "pane"));
        const failedStrip = strip.filter((k) => s.thumbs.done[k] === "failed");
        if (failedStrip.length) dispatch({ type: "thumbsRetried", keys: failedStrip });
      }
      if (tab === "products") dispatch({ type: "productsRetried", key });
    },
    retryLarge() {
      const s = get();
      const mission = missionOf(s);
      const id = s.large.datasetId;
      if (!mission || !id) return;
      const key = imageSetKey(mission.id, id);
      if (s.lists[key]?.error) dispatch({ type: "imageListRetried", key });
      const le = largeEntry(s);
      if (!le?.previewFilename) return;
      retryFailedPreview(previewKey(mission.id, id, le.previewFilename, "pane"));
      retryFailedPreview(previewKey(mission.id, id, le.previewFilename, "large"));
    },
    refreshProducts() {
      const s = get();
      const mission = missionOf(s);
      if (!mission?.capabilities.products || !s.selectedId || s.run) return;
      startProducts(s, mission.id, s.selectedId, true);
    },
    cancelProducts() {
      const s = get();
      const mission = missionOf(s);
      if (!mission || !s.selectedId) return;
      const key = imageSetKey(mission.id, s.selectedId);
      const p = s.products[key];
      if (!p || p.stage !== "loading") return;
      attempts.delete(`products:${key}`);
      cancelRequest(p.requestId);
      dispatch({ type: "productsCancelled", key, requestId: p.requestId, byUser: true });
    },
    dispose() {
      if (timer) clock.clearTimeout(timer.handle);
      timer = null;
    },
  };
}
