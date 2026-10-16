export interface ArchiveKeyEvent {
  key: string;
  code?: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

export type ArchiveKeyCommand =
  | "focus-query"
  | "next-region"
  | "previous-region"
  | "toggle-view"
  | "retry"
  | "toggle-detail"
  | "tab-previews"
  | "tab-metadata"
  | "tab-products";

export interface KeyTarget {
  tagName?: string;
  type?: string;
  isContentEditable?: boolean;
}

const NON_TEXT_INPUTS = new Set(["checkbox", "radio", "button", "submit", "reset", "range", "color", "file", "image"]);
const TAB_KEYS: Record<string, ArchiveKeyCommand> = { "1": "tab-previews", "2": "tab-metadata", "3": "tab-products" };
const LETTERS: Record<string, ArchiveKeyCommand> = { v: "toggle-view", r: "retry", i: "toggle-detail" };

export function isTextEntry(target: KeyTarget | null | undefined): boolean {
  if (!target) return false;
  if (target.isContentEditable) return true;
  const tag = (target.tagName ?? "").toUpperCase();
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag !== "INPUT") return false;
  return !NON_TEXT_INPUTS.has((target.type ?? "text").toLowerCase());
}

function digitOf(e: ArchiveKeyEvent): string | null {
  const m = /^(?:Digit|Numpad)(\d)$/.exec(e.code ?? "");
  if (m) return m[1];
  return /^\d$/.test(e.key) ? e.key : null;
}

export function archiveKeyCommand(e: ArchiveKeyEvent, ctx: { inInput: boolean }): ArchiveKeyCommand | null {
  const mod = e.ctrlKey || e.metaKey;
  if (e.key === "F6" && !mod && !e.altKey) return e.shiftKey ? "previous-region" : "next-region";
  if (mod && !e.altKey && !e.shiftKey && (e.code === "KeyF" || e.key.toLowerCase() === "f")) return "focus-query";
  if (e.altKey && !mod && !e.shiftKey) {
    const digit = digitOf(e);
    return digit ? TAB_KEYS[digit] ?? null : null;
  }
  if (mod || e.altKey || ctx.inInput) return null;
  if (e.key === "/") return "focus-query";
  return LETTERS[e.key.toLowerCase()] ?? null;
}
