import { useSyncExternalStore } from "react";
import type { MastDownloadItem, MastDownloadState, MastError } from "../../../shared/types/mast";
import { downloadCountsOf, openableDownload, type DownloadCounts } from "../logic/downloadView";

export interface DownloadsState {
  items: MastDownloadItem[];
  settled: readonly string[];
  showGroups: Readonly<Record<string, string>>;
  shownGroups: readonly string[];
  drawerOpen: boolean;
  enqueueError: MastError | null;
  cancelError: MastError | null;
}

export type DownloadsAction =
  | { type: "snapshot"; items: readonly MastDownloadItem[] }
  | { type: "item"; item: MastDownloadItem }
  | { type: "settled"; ids: readonly string[] }
  | { type: "showGroup"; group: string; ids: readonly string[] }
  | { type: "shown"; group: string }
  | { type: "drawer"; open: boolean }
  | { type: "enqueueFailed"; error: MastError | null }
  | { type: "cancelFailed"; error: MastError | null };

const STAGE: Record<MastDownloadState, number> = { queued: 0, downloading: 1, done: 2, error: 2, cancelled: 2 };

export function createDownloadsState(): DownloadsState {
  return { items: [], settled: [], showGroups: {}, shownGroups: [], drawerOpen: false, enqueueError: null, cancelError: null };
}

function later(known: MastDownloadItem | undefined, incoming: MastDownloadItem): MastDownloadItem {
  if (!known) return incoming;
  const rank = STAGE[incoming.state] - STAGE[known.state];
  if (rank !== 0) return rank > 0 ? incoming : known;
  if (STAGE[incoming.state] === STAGE.done) return known;
  return incoming.received >= known.received ? incoming : known;
}

function merged(current: readonly MastDownloadItem[], incoming: readonly MastDownloadItem[]): MastDownloadItem[] {
  const known = new Map(current.map((item) => [item.id, item]));
  return incoming.map((item) => later(known.get(item.id), item));
}

function upserted(current: readonly MastDownloadItem[], item: MastDownloadItem): MastDownloadItem[] {
  const at = current.findIndex((known) => known.id === item.id);
  if (at < 0) return [...current, item];
  const next = later(current[at], item);
  if (next === current[at]) return current as MastDownloadItem[];
  const items = [...current];
  items[at] = next;
  return items;
}

export function downloadsReducer(state: DownloadsState, action: DownloadsAction): DownloadsState {
  switch (action.type) {
    case "snapshot":
      return { ...state, items: merged(state.items, action.items) };
    case "item": {
      const items = upserted(state.items, action.item);
      return items === state.items ? state : { ...state, items };
    }
    case "settled": {
      const fresh = action.ids.filter((id) => !state.settled.includes(id));
      return fresh.length ? { ...state, settled: [...state.settled, ...fresh] } : state;
    }
    case "showGroup":
      return action.ids.length ? { ...state, showGroups: { ...state.showGroups, ...Object.fromEntries(action.ids.map((id) => [id, action.group])) } } : state;
    case "shown":
      return state.shownGroups.includes(action.group) ? state : { ...state, shownGroups: [...state.shownGroups, action.group] };
    case "drawer":
      return state.drawerOpen === action.open ? state : { ...state, drawerOpen: action.open };
    case "enqueueFailed":
      return state.enqueueError === action.error ? state : { ...state, enqueueError: action.error };
    case "cancelFailed":
      return state.cancelError === action.error ? state : { ...state, cancelError: action.error };
  }
}

export function downloadCounts(state: DownloadsState): DownloadCounts {
  return downloadCountsOf(state.items);
}

export function unsettledDownloads(state: DownloadsState): MastDownloadItem[] {
  return state.items.filter((item) => item.state === "done" && !state.settled.includes(item.id));
}

export function pendingOpens(state: DownloadsState): MastDownloadItem[] {
  return unsettledDownloads(state).filter((item) => item.open && openableDownload(item));
}

export function showGroupOf(state: DownloadsState, item: MastDownloadItem): string {
  return Object.hasOwn(state.showGroups, item.id) ? state.showGroups[item.id] : item.requestId;
}

export interface DownloadsStore {
  getState(): DownloadsState;
  dispatch(action: DownloadsAction): void;
  subscribe(listener: () => void): () => void;
}

export function createDownloadsStore(initial: DownloadsState = createDownloadsState()): DownloadsStore {
  let state = initial;
  const listeners = new Set<() => void>();
  return {
    getState: () => state,
    dispatch(action) {
      const next = downloadsReducer(state, action);
      if (next === state) return;
      state = next;
      [...listeners].forEach((listener) => listener());
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

export const downloadsStore = createDownloadsStore();

export function useDownloads(store: DownloadsStore = downloadsStore): DownloadsState {
  return useSyncExternalStore(store.subscribe, store.getState, store.getState);
}

export function enqueueErrorOf(state: DownloadsState): MastError | null {
  return state.enqueueError;
}

export function useDownloadsError(store: DownloadsStore = downloadsStore): MastError | null {
  const read = () => enqueueErrorOf(store.getState());
  return useSyncExternalStore(store.subscribe, read, read);
}
