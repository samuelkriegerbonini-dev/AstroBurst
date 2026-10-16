export interface DetailKeyEvent {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

export type LargeKeyCommand =
  | "close"
  | "prev-image"
  | "next-image"
  | "prev-dataset"
  | "next-dataset"
  | "next-group"
  | "zoom-in"
  | "zoom-out"
  | "zoom-fit"
  | "zoom-actual"
  | "toggle-rail"
  | "retry"
  | "focus-query";

export type PreviewsKeyCommand = "prev-image" | "next-image" | "next-group" | "open-large";
export type ResultsKeyCommand = "open-detail" | "open-large";

const LARGE_KEYS: Record<string, LargeKeyCommand> = {
  Escape: "close",
  ArrowLeft: "prev-image",
  ArrowRight: "next-image",
  ArrowUp: "prev-dataset",
  ArrowDown: "next-dataset",
  g: "next-group",
  "+": "zoom-in",
  "=": "zoom-in",
  "-": "zoom-out",
  "0": "zoom-fit",
  "1": "zoom-actual",
  i: "toggle-rail",
  r: "retry",
};

const PREVIEW_KEYS: Record<string, PreviewsKeyCommand> = {
  ArrowLeft: "prev-image",
  ArrowRight: "next-image",
  g: "next-group",
  " ": "open-large",
};

function normal(key: string): string {
  return key.length === 1 ? key.toLowerCase() : key;
}

export function largeKeyCommand(e: DetailKeyEvent): LargeKeyCommand | null {
  const mod = e.ctrlKey || e.metaKey;
  if (mod && !e.altKey && !e.shiftKey && normal(e.key) === "f") return "focus-query";
  if (mod || e.altKey) return null;
  const key = normal(e.key);
  if (e.shiftKey && key !== "+") return null;
  return LARGE_KEYS[key] ?? null;
}

export function previewsKeyCommand(e: DetailKeyEvent, ctx: { inInput: boolean }): PreviewsKeyCommand | null {
  if (ctx.inInput || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return null;
  return PREVIEW_KEYS[normal(e.key)] ?? null;
}

export function resultsKeyCommand(e: DetailKeyEvent): ResultsKeyCommand | null {
  if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return null;
  if (e.key === "Enter") return "open-detail";
  if (e.key === " ") return "open-large";
  return null;
}
