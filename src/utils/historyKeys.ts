import type { AppMode } from "../hooks/useAppMode";

export type HistoryKeyEvent = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">;

export interface HistoryKeyContext {
  inInput: boolean;
  menuOpen: boolean;
  mode: AppMode;
  compositeMode: boolean;
  mac: boolean;
}

export type HistoryKeyAction = "undo" | "redo";

const PHYSICAL_LETTERS: Record<string, string> = { KeyZ: "z", KeyY: "y" };

function letterOf(e: HistoryKeyEvent): string {
  if (e.key.length === 1) return e.key.toLowerCase();
  return PHYSICAL_LETTERS[e.code] ?? "";
}

export function historyKeyAction(e: HistoryKeyEvent, ctx: HistoryKeyContext): HistoryKeyAction | null {
  if (ctx.inInput || ctx.menuOpen || ctx.compositeMode || ctx.mode !== "workspace") return null;
  const primary = ctx.mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
  if (!primary || e.altKey) return null;
  const letter = letterOf(e);
  if (letter === "z") return e.shiftKey ? "redo" : "undo";
  if (letter === "y" && !ctx.mac && !e.shiftKey) return "redo";
  return null;
}

export function isMacPlatform(platform: string): boolean {
  return /^mac/i.test(platform);
}
