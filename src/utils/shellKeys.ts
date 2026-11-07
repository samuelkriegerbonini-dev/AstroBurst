export type ShellKeyEvent = Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey">;

export interface ShellKeyContext {
  inInput: boolean;
}

export type ShellOverlayAction = "toggle-palette" | "toggle-shortcuts" | "close-overlays";

export type ShellKeyAction = "open-files" | ShellOverlayAction;

export interface ShellOverlays {
  paletteOpen: boolean;
  shortcutsOpen: boolean;
}

export const NO_OVERLAYS: ShellOverlays = { paletteOpen: false, shortcutsOpen: false };

export const DOUBLE_SHIFT_MS = 350;

export function shellKeyAction(e: ShellKeyEvent, ctx: ShellKeyContext): ShellKeyAction | null {
  const primary = e.ctrlKey || e.metaKey;
  const letter = e.key.toLowerCase();
  if (primary && letter === "o") return "open-files";
  if (primary && letter === "k") return "toggle-palette";
  if (ctx.inInput) return null;
  if (e.key === "?") return "toggle-shortcuts";
  if (e.key === "Escape") return "close-overlays";
  return null;
}

export function shiftReleaseAction(now: number, lastShiftUp: number): ShellOverlayAction | null {
  return lastShiftUp > 0 && now - lastShiftUp < DOUBLE_SHIFT_MS ? "toggle-palette" : null;
}

export function overlaysAfter(overlays: ShellOverlays, action: ShellOverlayAction): ShellOverlays {
  if (action === "toggle-palette") return { paletteOpen: !overlays.paletteOpen, shortcutsOpen: false };
  if (action === "toggle-shortcuts") return { paletteOpen: false, shortcutsOpen: !overlays.shortcutsOpen };
  return overlays.paletteOpen || overlays.shortcutsOpen ? NO_OVERLAYS : overlays;
}

export const NEW_BATCH_CONFIRM_DELAY_MS = 450;

export type NewBatchPaletteStep = "discard" | "arm" | "confirm";

export interface NewBatchPaletteEntry {
  label: string;
  hint?: string;
  keywords?: string[];
  prompt?: string;
  step: NewBatchPaletteStep;
}

export function newBatchPaletteEntry(processed: number, armed: boolean): NewBatchPaletteEntry {
  if (processed === 0) return { label: "New Batch", step: "discard" };
  if (!armed) return { label: "New Batch (discard processed files)", hint: "asks to confirm", step: "arm" };
  const files = `${processed} processed file${processed === 1 ? "" : "s"}`;
  return {
    label: `Discard ${files}`,
    hint: "confirm",
    keywords: ["New Batch", "Reset"],
    prompt: `Discard ${files}? Press Enter again to confirm, or Escape to cancel.`,
    step: "confirm",
  };
}

export function newBatchConfirmAccepted(sinceArmedMs: number, keyRepeat: boolean): boolean {
  return !keyRepeat && sinceArmedMs >= NEW_BATCH_CONFIRM_DELAY_MS;
}
