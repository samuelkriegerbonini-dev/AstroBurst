import { useSyncExternalStore, useCallback } from "react";

export interface PixelCoord {
  x: number;
  y: number;
}

type Listener = () => void;

class MousePixelStore {
  private value: PixelCoord | null = null;
  private listeners = new Set<Listener>();

  getSnapshot = (): PixelCoord | null => this.value;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  set(coord: PixelCoord | null) {
    if (
      this.value?.x === coord?.x &&
      this.value?.y === coord?.y
    ) {
      return;
    }
    this.value = coord;
    this.listeners.forEach((l) => l());
  }

  clear() {
    if (this.value === null) return;
    this.value = null;
    this.listeners.forEach((l) => l());
  }
}

const store = new MousePixelStore();

export function setMousePixel(coord: PixelCoord | null) {
  if (coord) store.set(coord);
  else store.clear();
}

export function useMousePixel(): PixelCoord | null {
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}

export interface PixelClick {
  x: number;
  y: number;
  seq: number;
}

class PixelClickStore {
  private value: PixelClick | null = null;
  private seq = 0;
  private listeners = new Set<Listener>();

  getSnapshot = (): PixelClick | null => this.value;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  emit(x: number, y: number) {
    this.seq += 1;
    this.value = { x, y, seq: this.seq };
    this.listeners.forEach((l) => l());
  }
}

const clickStore = new PixelClickStore();

export function emitPixelClick(x: number, y: number) {
  clickStore.emit(x, y);
}

export function usePixelClick(): PixelClick | null {
  return useSyncExternalStore(clickStore.subscribe, clickStore.getSnapshot);
}

export interface ElementRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export function pixelFromRect(
  clientX: number,
  clientY: number,
  rect: ElementRect,
  dimensions: readonly [number, number] | null | undefined,
): PixelCoord | null {
  if (!dimensions || !(rect.width > 0) || !(rect.height > 0)) return null;
  const px = Math.floor(((clientX - rect.left) / rect.width) * dimensions[0]);
  const py = Math.floor(((clientY - rect.top) / rect.height) * dimensions[1]);
  if (!Number.isFinite(px) || !Number.isFinite(py)) return null;
  if (px < 0 || px >= dimensions[0] || py < 0 || py >= dimensions[1]) return null;
  return { x: px, y: py };
}

export function useMousePixelActions() {
  const clear = useCallback(() => {
    store.clear();
  }, []);

  return { handleLeave: clear, reset: clear };
}
