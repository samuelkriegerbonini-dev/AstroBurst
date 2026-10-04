import type { PixelMathSlot } from "../../shared/types/pixelmath";

const MAX_RETAINED_PANELS = 32;

export interface RetainedPanelState {
  expression: string;
  slots: PixelMathSlot[];
  truncate: boolean;
  rescale: boolean;
  outputName: string;
  slotsTouched: boolean;
}

const retained = new Map<string, RetainedPanelState>();

export function retainedFor(key: string | null): RetainedPanelState | undefined {
  return key ? retained.get(key) : undefined;
}

export function retain(key: string | null, state: RetainedPanelState): void {
  if (!key) return;
  retained.delete(key);
  retained.set(key, state);
  while (retained.size > MAX_RETAINED_PANELS) {
    const oldest = retained.keys().next().value;
    if (oldest === undefined) break;
    retained.delete(oldest);
  }
}

export function markSlotsTouched(key: string): void {
  const saved = retained.get(key);
  if (saved) retain(key, { ...saved, slotsTouched: true });
}

export function slotsTouchedFor(key: string | null): boolean {
  return retainedFor(key)?.slotsTouched ?? false;
}
