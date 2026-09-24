import { useCallback, useEffect, useSyncExternalStore } from "react";
import { getRenderRecord, useRenderActions } from "../context/PreviewContext";
import { liveCompositeVersion } from "../context/CompositeContext";
import type { ChainStep, ProcessingChain } from "../shared/types/preview";
import { EMPTY_CHAIN, inputFor, samePath, withVersionParam } from "../utils/processingChain";

export const BUSY_TITLE = "Another step is running on this file";
export const OTHER_FILE_TITLE = "This step is running on another file";
export const INPUT_CHANGED_MESSAGE = "The image this step ran on was reset or replaced while it ran, so the result was not applied.";
export const COMPOSITE_CHANGED_MESSAGE = "The composite changed or another file was selected while this step ran, so the result was not applied.";

interface RunLock {
  panel: string;
  token: number;
}

interface RunEntry {
  key: string;
  token: number;
  result: unknown;
  error: string | null;
  detached: boolean;
}

export interface RunState {
  locks: ReadonlyMap<string, RunLock>;
  entries: ReadonlyMap<string, RunEntry>;
}

export interface RunView {
  running: boolean;
  blocked: boolean;
  busyTitle: string | undefined;
  result: unknown;
  error: string | null;
}

export const EMPTY_RUN_STATE: RunState = { locks: new Map(), entries: new Map() };

const IDLE_VIEW: RunView = { running: false, blocked: false, busyTitle: undefined, result: null, error: null };

function without<K, V>(map: ReadonlyMap<K, V>, key: K): Map<K, V> {
  const next = new Map(map);
  next.delete(key);
  return next;
}

function withValue<K, V>(map: ReadonlyMap<K, V>, key: K, value: V): Map<K, V> {
  const next = new Map(map);
  next.set(key, value);
  return next;
}

function panelRunsElsewhere(state: RunState, panel: string, key: string): boolean {
  for (const [lockKey, lock] of state.locks) {
    if (lockKey !== key && lock.panel === panel) return true;
  }
  return false;
}

export function acquireRun(state: RunState, panel: string, key: string, token: number): RunState | null {
  if (state.locks.has(key) || panelRunsElsewhere(state, panel, key)) return null;
  return {
    locks: withValue(state.locks, key, { panel, token }),
    entries: withValue(state.entries, panel, { key, token, result: null, error: null, detached: false }),
  };
}

export function settleRun(
  state: RunState,
  panel: string,
  key: string,
  token: number,
  result: unknown,
  error: string | null,
): RunState {
  const lock = state.locks.get(key);
  const ownsLock = lock !== undefined && lock.panel === panel && lock.token === token;
  const entry = state.entries.get(panel);
  const ownsEntry = entry !== undefined && entry.token === token;
  if (!ownsLock && !ownsEntry) return state;
  let entries = state.entries;
  if (ownsEntry) entries = entry.detached ? without(state.entries, panel) : withValue(state.entries, panel, { ...entry, result, error });
  return {
    locks: ownsLock ? without(state.locks, key) : state.locks,
    entries,
  };
}

export function forgetOtherKeys(state: RunState, panel: string, key: string | null): RunState {
  const entry = state.entries.get(panel);
  if (!entry) return state;
  const away = entry.key !== key;
  const inFlight = state.locks.get(entry.key)?.token === entry.token;
  if (away && !inFlight) return { locks: state.locks, entries: without(state.entries, panel) };
  if (entry.detached === away) return state;
  return { locks: state.locks, entries: withValue(state.entries, panel, { ...entry, detached: away }) };
}

export function clearRunError(state: RunState, panel: string, key: string | null): RunState {
  const entry = state.entries.get(panel);
  if (!entry || entry.key !== key || entry.error === null) return state;
  return { locks: state.locks, entries: withValue(state.entries, panel, { ...entry, error: null }) };
}

export function runView(state: RunState, panel: string, key: string | null): RunView {
  if (!key) return IDLE_VIEW;
  const lock = state.locks.get(key);
  const entry = state.entries.get(panel);
  const own = entry !== undefined && entry.key === key ? entry : null;
  const running = lock !== undefined && lock.panel === panel;
  const busyTitle = running
    ? undefined
    : lock !== undefined
      ? BUSY_TITLE
      : panelRunsElsewhere(state, panel, key)
        ? OTHER_FILE_TITLE
        : undefined;
  return {
    running,
    blocked: busyTitle !== undefined,
    busyTitle,
    result: own?.result ?? null,
    error: own?.error ?? null,
  };
}

export function sameChainInput(before: ProcessingChain, after: ProcessingChain, step: ChainStep): boolean {
  const a = inputFor(before, step, "");
  const b = inputFor(after, step, "");
  if (a === "" || b === "") return a === b;
  return samePath(a, b);
}

export function sameSource(before: string | null, after: string | null): boolean {
  if (before === null || after === null) return before === after;
  return samePath(before, after);
}

export interface CompositeMark {
  version: number;
  fileKey: string | null;
}

export function sameCompositeRun(start: CompositeMark, live: CompositeMark): boolean {
  return live.version === start.version && live.fileKey === start.fileKey;
}

export function bustPreviewUrl(url: string | null | undefined, stamp: number): string | undefined {
  if (!url) return undefined;
  if (/^(data|blob):/i.test(url)) return url;
  return withVersionParam(url, stamp);
}

export function isCancelMessage(message: string): boolean {
  return /cancel/i.test(message);
}

let runState: RunState = EMPTY_RUN_STATE;
let tokenSeq = 0;
const listeners = new Set<() => void>();

function commit(next: RunState): void {
  if (next === runState) return;
  runState = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot(): RunState {
  return runState;
}

export interface RunContext {
  key: string;
  inputUnchanged: (step: ChainStep) => boolean;
  displayedUnchanged: () => boolean;
}

export interface ProcessingRun<T> {
  running: boolean;
  blocked: boolean;
  busyTitle: string | undefined;
  result: T | null;
  error: string | null;
  run: (task: (ctx: RunContext) => Promise<T | null>, isQuietError?: (message: string) => boolean) => Promise<void>;
  clearError: () => void;
}

export function useProcessingRun<T>(panel: string, fileKey: string | null): ProcessingRun<T> {
  const state = useSyncExternalStore(subscribe, snapshot, snapshot);

  useEffect(() => {
    commit(forgetOtherKeys(runState, panel, fileKey));
  }, [panel, fileKey]);

  const run = useCallback(
    async (task: (ctx: RunContext) => Promise<T | null>, isQuietError?: (message: string) => boolean) => {
      if (!fileKey) return;
      const key = fileKey;
      const token = ++tokenSeq;
      const acquired = acquireRun(runState, panel, key, token);
      if (!acquired) return;
      commit(acquired);
      const start = getRenderRecord(key);
      const ctx: RunContext = {
        key,
        inputUnchanged: (step) => sameChainInput(start?.chain ?? EMPTY_CHAIN, getRenderRecord(key)?.chain ?? EMPTY_CHAIN, step),
        displayedUnchanged: () => sameSource(start?.processed?.fitsPath ?? null, getRenderRecord(key)?.processed?.fitsPath ?? null),
      };
      let result: T | null = null;
      let error: string | null = null;
      try {
        result = await task(ctx);
      } catch (e: unknown) {
        const message = e instanceof Error ? e.message : String(e);
        if (!isQuietError?.(message)) error = message;
      } finally {
        commit(settleRun(runState, panel, key, token, result, error));
      }
    },
    [panel, fileKey],
  );

  const clearError = useCallback(() => {
    commit(clearRunError(runState, panel, fileKey));
  }, [panel, fileKey]);

  const view = runView(state, panel, fileKey);
  return {
    running: view.running,
    blocked: view.blocked,
    busyTitle: view.busyTitle,
    result: view.result as T | null,
    error: view.error,
    run,
    clearError,
  };
}

export function beginCompositeCheck(
  fileKey: () => string | null,
  version: () => number = liveCompositeVersion,
): () => boolean {
  const start: CompositeMark = { version: version(), fileKey: fileKey() };
  return () => sameCompositeRun(start, { version: version(), fileKey: fileKey() });
}

export function useCompositeRunGuard(): () => () => boolean {
  const { currentFileKey } = useRenderActions();
  return useCallback(() => beginCompositeCheck(currentFileKey), [currentFileKey]);
}
