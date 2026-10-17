import type { MastError, MastErrorCode } from "../../../shared/types/mast";

export const MAX_THUMBS_IN_FLIGHT = 4;
export const BREAKER_LIMIT = 3;

export type ThumbStatus = "pending" | "loading" | "ready" | "failed" | "paused";

export interface ThumbFailure {
  error: MastError;
  waitedMs: number | null;
}

export interface ThumbQueue {
  selected: string[];
  visible: string[];
  ahead: string[];
  inflight: string[];
  done: Record<string, "ready" | "failed">;
  failures: Record<string, ThumbFailure>;
  startedAt: Record<string, number>;
  pausedUntil: number | null;
  retryAfterSec: number | null;
  status: number | null;
  consecutive: number;
  gaveUp: boolean;
  active: boolean;
}

export function createThumbQueue(): ThumbQueue {
  return {
    selected: [],
    visible: [],
    ahead: [],
    inflight: [],
    done: {},
    failures: {},
    startedAt: {},
    pausedUntil: null,
    retryAfterSec: null,
    status: null,
    consecutive: 0,
    gaveUp: false,
    active: true,
  };
}

function sameList(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

function omit<T>(map: Record<string, T>, keys: readonly string[]): Record<string, T> {
  if (!keys.some((k) => k in map)) return map;
  const next = { ...map };
  for (const k of keys) delete next[k];
  return next;
}

export function setDemand(q: ThumbQueue, demand: { selected: string[]; visible: string[]; ahead: string[] }): ThumbQueue {
  if (sameList(q.selected, demand.selected) && sameList(q.visible, demand.visible) && sameList(q.ahead, demand.ahead)) return q;
  return { ...q, selected: [...demand.selected], visible: [...demand.visible], ahead: [...demand.ahead] };
}

export function setActive(q: ThumbQueue, active: boolean): ThumbQueue {
  return q.active === active ? q : { ...q, active };
}

function isPaused(q: ThumbQueue, now: number): boolean {
  if (q.pausedUntil === null) return false;
  return q.gaveUp || now < q.pausedUntil;
}

function wanted(q: ThumbQueue): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const key of [...q.selected, ...q.visible, ...q.ahead]) {
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(key);
  }
  return out;
}

export function nextStarts(q: ThumbQueue, now: number): string[] {
  if (!q.active || isPaused(q, now)) return [];
  const free = MAX_THUMBS_IN_FLIGHT - q.inflight.length;
  if (free <= 0) return [];
  const busy = new Set(q.inflight);
  return wanted(q).filter((k) => !busy.has(k) && !(k in q.done)).slice(0, free);
}

export function markStarted(q: ThumbQueue, keys: readonly string[], now: number): ThumbQueue {
  const add = keys.filter((k) => !q.inflight.includes(k));
  if (add.length === 0) return q;
  const startedAt = { ...q.startedAt };
  for (const k of add) startedAt[k] ??= now;
  return { ...q, inflight: [...q.inflight, ...add], startedAt };
}

function finish(q: ThumbQueue, key: string): Pick<ThumbQueue, "inflight" | "startedAt"> {
  return { inflight: q.inflight.filter((k) => k !== key), startedAt: omit(q.startedAt, [key]) };
}

export function markReady(q: ThumbQueue, key: string): ThumbQueue {
  return { ...q, ...finish(q, key), done: { ...q.done, [key]: "ready" }, failures: omit(q.failures, [key]), consecutive: 0 };
}

export function markFailed(q: ThumbQueue, key: string, error: MastError, now: number | null): ThumbQueue {
  const started = q.startedAt[key];
  const waitedMs = now !== null && started !== undefined ? Math.max(0, now - started) : null;
  return { ...q, ...finish(q, key), done: { ...q.done, [key]: "failed" }, failures: { ...q.failures, [key]: { error, waitedMs } } };
}

export function markDropped(q: ThumbQueue, key: string): ThumbQueue {
  return q.inflight.includes(key) || key in q.startedAt ? { ...q, ...finish(q, key) } : q;
}

export function markRateLimited(q: ThumbQueue, key: string, now: number, retryAfterSec: number, consecutive: number, status: number | null = null): ThumbQueue {
  const seconds = Math.max(1, retryAfterSec);
  const stillPaused = q.gaveUp || (q.pausedUntil !== null && now < q.pausedUntil);
  return {
    ...q,
    ...finish(q, key),
    pausedUntil: Math.max(q.pausedUntil ?? 0, now + seconds * 1000),
    retryAfterSec: seconds,
    status: status ?? (stillPaused ? q.status : null),
    consecutive,
    gaveUp: consecutive >= BREAKER_LIMIT,
  };
}

export function tickQueue(q: ThumbQueue, now: number): ThumbQueue {
  if (q.pausedUntil === null || q.gaveUp || now < q.pausedUntil) return q;
  return { ...q, pausedUntil: null, retryAfterSec: null };
}

export function resumeNow(q: ThumbQueue): ThumbQueue {
  if (q.pausedUntil === null && !q.gaveUp) return q;
  return { ...q, pausedUntil: null, retryAfterSec: null, gaveUp: false };
}

export function retryFailed(q: ThumbQueue, keys?: readonly string[]): ThumbQueue {
  const targets = keys ?? Object.keys(q.done).filter((k) => q.done[k] === "failed");
  const done = { ...q.done };
  for (const k of targets) {
    if (done[k] === "failed") delete done[k];
  }
  return { ...q, done, failures: omit(q.failures, targets), pausedUntil: null, retryAfterSec: null, gaveUp: false, consecutive: keys ? q.consecutive : 0 };
}

export function forget(q: ThumbQueue, keys: readonly string[]): ThumbQueue {
  if (keys.length === 0) return q;
  return { ...q, done: omit(q.done, keys), failures: omit(q.failures, keys) };
}

export function thumbStatus(q: ThumbQueue, key: string, now: number): ThumbStatus {
  const done = q.done[key];
  if (done) return done;
  if (q.inflight.includes(key)) return "loading";
  return isPaused(q, now) ? "paused" : "pending";
}

export function failedCount(q: ThumbQueue, code?: MastErrorCode): number {
  return Object.keys(q.done).filter((k) => q.done[k] === "failed" && (code === undefined || q.failures[k]?.error.code === code)).length;
}
