import { useSyncExternalStore } from "react";
import {
  DISPLAY_MEMORY_KEY,
  LEGACY_DISPLAY_KEY,
  fileEntry,
  parseDisplayMemory,
  serializeDisplayMemory,
  type DisplayMemoryDoc,
  type FileDisplayEntry,
} from "./displayMemory";

export type DisplayMemoryStorage = Pick<Storage, "getItem" | "setItem">;

export interface DisplayMemoryTimers {
  set: (fn: () => void, ms: number) => unknown;
  clear: (handle: unknown) => void;
}

export const DISPLAY_MEMORY_SAVE_MS = 250;

const BROWSER_TIMERS: DisplayMemoryTimers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

function readItem(storage: DisplayMemoryStorage | null, key: string): string | null {
  if (!storage) return null;
  try {
    return storage.getItem(key);
  } catch {
    return null;
  }
}

export class DisplayMemoryStore {
  private doc: DisplayMemoryDoc;
  private dirty: boolean;
  private timer: unknown = null;
  private storage: DisplayMemoryStorage | null;
  private readonly timers: DisplayMemoryTimers;
  private readonly listeners = new Set<() => void>();

  constructor(storage: DisplayMemoryStorage | null, timers: DisplayMemoryTimers = BROWSER_TIMERS) {
    this.storage = storage;
    this.timers = timers;
    const text = readItem(storage, DISPLAY_MEMORY_KEY);
    this.doc = parseDisplayMemory(text, readItem(storage, LEGACY_DISPLAY_KEY));
    this.dirty = text === null || serializeDisplayMemory(this.doc) !== text;
  }

  get = (): DisplayMemoryDoc => this.doc;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  update(fn: (doc: DisplayMemoryDoc) => DisplayMemoryDoc): void {
    const next = fn(this.doc);
    if (next === this.doc) return;
    this.doc = next;
    this.dirty = true;
    this.notify();
    if (this.storage && this.timer === null) {
      this.timer = this.timers.set(() => {
        this.timer = null;
        this.flush();
      }, DISPLAY_MEMORY_SAVE_MS);
    }
  }

  flush(): void {
    if (this.timer !== null) {
      this.timers.clear(this.timer);
      this.timer = null;
    }
    if (!this.dirty || !this.storage) return;
    this.dirty = false;
    if (this.write(this.doc)) return;
    const files = this.doc.files;
    const trimmed: DisplayMemoryDoc = { ...this.doc, files: files.slice(Math.ceil(files.length / 2)) };
    if (this.write(trimmed)) {
      this.doc = trimmed;
      this.notify();
      return;
    }
    this.storage = null;
  }

  private write(doc: DisplayMemoryDoc): boolean {
    try {
      this.storage?.setItem(DISPLAY_MEMORY_KEY, serializeDisplayMemory(doc));
      return true;
    } catch {
      return false;
    }
  }

  private notify(): void {
    this.listeners.forEach((listener) => listener());
  }
}

function browserStorage(): DisplayMemoryStorage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export const displayMemory = new DisplayMemoryStore(browserStorage());

if (typeof window !== "undefined") {
  window.addEventListener("pagehide", () => displayMemory.flush());
  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") displayMemory.flush();
    });
  }
}

export function useFileDisplayEntry(path: string | null): FileDisplayEntry | null {
  const read = () => fileEntry(displayMemory.get(), path);
  return useSyncExternalStore(displayMemory.subscribe, read, read);
}
