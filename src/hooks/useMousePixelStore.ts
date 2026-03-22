import { useSyncExternalStore, useCallback, useRef } from "react";

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

export function useMousePixelActions() {
  const previewTargetRef = useRef<HTMLElement | null>(null);
  const rafRef = useRef<number | null>(null);
  const lastRef = useRef<PixelCoord | null>(null);

  const handleMove = useCallback(
    (e: React.MouseEvent<HTMLElement>, dimensions: [number, number] | undefined) => {
      if (!dimensions) return;
      if (
        !previewTargetRef.current ||
        !e.currentTarget.contains(previewTargetRef.current)
      ) {
        previewTargetRef.current = e.currentTarget.querySelector(
          "img, canvas"
        ) as HTMLElement;
      }
      const target = previewTargetRef.current;
      if (!target) return;

      const rect = target.getBoundingClientRect();
      const px = Math.floor(
        ((e.clientX - rect.left) / rect.width) * dimensions[0]
      );
      const py = Math.floor(
        ((e.clientY - rect.top) / rect.height) * dimensions[1]
      );

      if (px < 0 || px >= dimensions[0] || py < 0 || py >= dimensions[1])
        return;

      const prev = lastRef.current;
      if (prev && prev.x === px && prev.y === py) return;
      lastRef.current = { x: px, y: py };

      if (rafRef.current) return;
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        store.set(lastRef.current);
      });
    },
    []
  );

  const handleLeave = useCallback(() => {
    lastRef.current = null;
    previewTargetRef.current = null;
    if (rafRef.current) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    store.clear();
  }, []);

  const reset = useCallback(() => {
    lastRef.current = null;
    previewTargetRef.current = null;
    if (rafRef.current) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    store.clear();
  }, []);

  return { handleMove, handleLeave, reset };
}
