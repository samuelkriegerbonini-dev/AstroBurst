import { useSyncExternalStore } from "react";
import { imageDepth } from "../services/depth";
import type { ImageDepth, ImageDepthOptions } from "../shared/types/depth";

export interface DepthState {
  depth: ImageDepth | null;
  loading: boolean;
  error: string | null;
}

export const EMPTY_DEPTH: DepthState = Object.freeze({ depth: null, loading: false, error: null });

export const MAX_DEPTH_DOCS = 8;

type Listener = () => void;

export function depthKey(path: string, measureKey: string | null | undefined): string {
  return `${path}|${measureKey ?? ""}`;
}

export class DepthStore {
  private readonly states = new Map<string, DepthState>();
  private readonly runs = new Map<string, number>();
  private readonly listeners = new Set<Listener>();
  private readonly run: (path: string, options: ImageDepthOptions) => Promise<ImageDepth>;
  private seq = 0;

  constructor(run: (path: string, options: ImageDepthOptions) => Promise<ImageDepth> = imageDepth) {
    this.run = run;
  }

  getSnapshot = (key: string | null): DepthState => (key === null ? EMPTY_DEPTH : (this.states.get(key) ?? EMPTY_DEPTH));

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  async measure(key: string, path: string, options: ImageDepthOptions): Promise<void> {
    const run = ++this.seq;
    this.runs.set(key, run);
    this.set(key, { depth: this.getSnapshot(key).depth, loading: true, error: null });
    try {
      const depth = await this.run(path, options);
      if (this.runs.get(key) === run) this.set(key, { depth, loading: false, error: null });
    } catch (error) {
      if (this.runs.get(key) === run) {
        this.set(key, { depth: null, loading: false, error: error instanceof Error ? error.message : String(error) });
      }
    }
  }

  forget(key: string): void {
    this.runs.delete(key);
    if (this.states.delete(key)) this.emit();
  }

  latest(path: string): ImageDepth | null {
    const prefix = `${path}|`;
    const newestFirst = [...this.states.entries()].reverse();
    return newestFirst.find(([key, state]) => key.startsWith(prefix) && state.depth !== null)?.[1].depth ?? null;
  }

  private set(key: string, state: DepthState): void {
    this.states.delete(key);
    this.states.set(key, state);
    for (const oldest of this.states.keys()) {
      if (this.states.size <= MAX_DEPTH_DOCS) break;
      this.states.delete(oldest);
      this.runs.delete(oldest);
    }
    this.emit();
  }

  private emit(): void {
    this.listeners.forEach((listener) => listener());
  }
}

export const depthStore = new DepthStore();

export function useDepth(key: string | null): DepthState {
  const snapshot = () => depthStore.getSnapshot(key);
  return useSyncExternalStore(depthStore.subscribe, snapshot, snapshot);
}
