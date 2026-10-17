import { useState, useEffect } from "react";
import { formatTime } from "../utils/format";

export const TIMER_TICK_MS = 100;

export interface Stopwatch {
  start: () => void;
  stop: () => void;
  reset: () => void;
  dispose: () => void;
}

export function createStopwatch(onChange: (elapsedMs: number) => void, now: () => number = Date.now): Stopwatch {
  let interval: ReturnType<typeof setInterval> | null = null;
  let startedAt: number | null = null;
  let carried = 0;

  const halt = (): boolean => {
    if (!interval) return false;
    clearInterval(interval);
    interval = null;
    if (startedAt !== null) carried = now() - startedAt;
    startedAt = null;
    return true;
  };

  return {
    start() {
      if (interval) return;
      startedAt = now() - carried;
      interval = setInterval(() => onChange(now() - (startedAt ?? now())), TIMER_TICK_MS);
    },
    stop() {
      if (halt()) onChange(carried);
    },
    reset() {
      halt();
      carried = 0;
      onChange(0);
    },
    dispose() {
      halt();
    },
  };
}

export function useTimer() {
  const [elapsed, setElapsed] = useState(0);
  const [watch] = useState(() => createStopwatch(setElapsed));

  useEffect(() => () => watch.dispose(), [watch]);

  return {
    elapsed,
    formatted: formatTime(elapsed),
    start: watch.start,
    stop: watch.stop,
    reset: watch.reset,
  };
}
