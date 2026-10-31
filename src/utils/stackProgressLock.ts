import { useSyncExternalStore } from "react";

export type StackProgressRun = "stack" | "drizzle" | "compose";

const RUN_NAMES: Record<StackProgressRun, string> = {
  stack: "Stacking › Stack",
  drizzle: "Stacking › Drizzle RGB (mono)",
  compose: "Compose › Stack",
};

let holder: StackProgressRun | null = null;
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function stackProgressHolder(): StackProgressRun | null {
  return holder;
}

export function claimStackProgress(run: StackProgressRun): boolean {
  if (holder !== null) return false;
  holder = run;
  notify();
  return true;
}

export function releaseStackProgress(run: StackProgressRun): void {
  if (holder !== run) return;
  holder = null;
  notify();
}

export function useStackProgressHolder(): StackProgressRun | null {
  return useSyncExternalStore(subscribe, stackProgressHolder, stackProgressHolder);
}

export function stackProgressBusyLine(current: StackProgressRun | null, runningHere: StackProgressRun | null): string | null {
  if (current === null || current === runningHere) return null;
  return `${RUN_NAMES[current]} is running; wait until it ends, as both use the same progress bar and Cancel.`;
}
