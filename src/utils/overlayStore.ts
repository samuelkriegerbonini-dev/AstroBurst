import type { Pt } from "./regionGeometry";

export interface OverlayPaintContext {
  ctx: CanvasRenderingContext2D;
  toScreen: (p: Pt) => Pt;
  width: number;
  height: number;
  screenPxPerImagePx: number;
}

export type OverlayPainter = (paint: OverlayPaintContext) => void;

export interface OverlayLayer {
  id: string;
  kind: string;
  visible: boolean;
  paint: OverlayPainter;
}

export interface OverlayDoc {
  layers: OverlayLayer[];
  version: number;
}

export const EMPTY_OVERLAY_DOC: OverlayDoc = Object.freeze({ layers: [], version: 0 }) as OverlayDoc;

type Listener = () => void;

export class OverlayStoreCore {
  private docs = new Map<string, OverlayDoc>();
  private listeners = new Set<Listener>();

  subscribe = (l: Listener): (() => void) => {
    this.listeners.add(l);
    return () => {
      this.listeners.delete(l);
    };
  };

  private notify(): void {
    this.listeners.forEach((l) => l());
  }

  getDoc = (fileKey: string): OverlayDoc => this.docs.get(fileKey) ?? EMPTY_OVERLAY_DOC;

  has(fileKey: string, id: string): boolean {
    return this.getDoc(fileKey).layers.some((l) => l.id === id);
  }

  visibleLayers(fileKey: string): OverlayLayer[] {
    return this.getDoc(fileKey).layers.filter((l) => l.visible);
  }

  private commit(fileKey: string, layers: OverlayLayer[]): void {
    const prev = this.getDoc(fileKey);
    this.docs.set(fileKey, { layers, version: prev.version + 1 });
    this.notify();
  }

  add(fileKey: string, layer: OverlayLayer): void {
    const layers = this.getDoc(fileKey).layers.slice();
    const idx = layers.findIndex((l) => l.id === layer.id);
    if (idx >= 0) layers[idx] = layer;
    else layers.push(layer);
    this.commit(fileKey, layers);
  }

  remove(fileKey: string, id: string): void {
    const doc = this.getDoc(fileKey);
    if (!doc.layers.some((l) => l.id === id)) return;
    this.commit(
      fileKey,
      doc.layers.filter((l) => l.id !== id),
    );
  }

  setVisible(fileKey: string, id: string, visible: boolean): void {
    const doc = this.getDoc(fileKey);
    const idx = doc.layers.findIndex((l) => l.id === id);
    if (idx < 0 || doc.layers[idx].visible === visible) return;
    const layers = doc.layers.slice();
    layers[idx] = { ...layers[idx], visible };
    this.commit(fileKey, layers);
  }

  clear(fileKey: string): void {
    if (this.getDoc(fileKey).layers.length === 0) return;
    this.commit(fileKey, []);
  }
}

export const overlayStore = new OverlayStoreCore();
