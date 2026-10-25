import { useCallback, useEffect, useState } from "react";
import { createTimedNotice } from "../utils/timedNotice";

export function useTimedNotice<T>(durationMs: number): [T | null, (value: T) => void, () => void] {
  const [value, setValue] = useState<T | null>(null);
  const [notice] = useState(() => createTimedNotice<T>(setValue, durationMs));
  useEffect(() => notice.clear, [notice]);
  return [value, notice.show, notice.clear];
}

export function useTimedFlag(durationMs: number): [boolean, () => void, () => void] {
  const [value, show, clear] = useTimedNotice<true>(durationMs);
  const raise = useCallback(() => show(true), [show]);
  return [value === true, raise, clear];
}
