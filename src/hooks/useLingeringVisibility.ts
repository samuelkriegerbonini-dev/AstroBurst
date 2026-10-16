import { useEffect, useState } from "react";

export const PROGRESS_STRIP_LINGER_MS = 2500;

export function scheduleIdleExpiry(busy: boolean, lingerMs: number, onExpire: () => void): () => void {
  if (busy) return () => {};
  const timer = setTimeout(onExpire, lingerMs);
  return () => clearTimeout(timer);
}

export function useLingeringVisibility(busy: boolean, lingerMs: number = PROGRESS_STRIP_LINGER_MS): boolean {
  const [expired, setExpired] = useState(false);

  useEffect(() => {
    if (busy) setExpired(false);
    return scheduleIdleExpiry(busy, lingerMs, () => setExpired(true));
  }, [busy, lingerMs]);

  return busy || !expired;
}
