export type WcsOverlayKind = "grid" | "compass";

export type WcsOverlayErrors = Readonly<Record<WcsOverlayKind, string | null>>;

export interface WcsOverlayNote {
  text: string;
  title: string;
}

const NO_ERRORS: WcsOverlayErrors = Object.freeze({ grid: null, compass: null });

export class WcsOverlayStatusStore {
  private errors: WcsOverlayErrors = NO_ERRORS;
  private listeners = new Set<() => void>();

  get = (): WcsOverlayErrors => this.errors;

  set(kind: WcsOverlayKind, error: string | null): void {
    if (this.errors[kind] === error) return;
    this.errors = { ...this.errors, [kind]: error };
    for (const listener of this.listeners) listener();
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
}

export const wcsOverlayStatus = new WcsOverlayStatusStore();

export function wcsOverlayNote(ticked: Readonly<Record<WcsOverlayKind, boolean>>, errors: WcsOverlayErrors): WcsOverlayNote | null {
  const reasons = (["grid", "compass"] as const)
    .filter((kind) => ticked[kind] && errors[kind] !== null)
    .map((kind) => `${kind}: ${errors[kind]}`);
  return reasons.length > 0 ? { text: "no WCS", title: reasons.join("; ") } : null;
}
