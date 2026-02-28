import { useState, useRef, useCallback, useEffect } from "react";
import { formatTime } from "../utils/format";

export function useTimer() {
  const [elapsed, setElapsed] = useState(0);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const startTimeRef = useRef<number | null>(null);

  const start = useCallback(() => {
    if (intervalRef.current) return;
    startTimeRef.current = Date.now() - elapsed;
    intervalRef.current = setInterval(() => {
      setElapsed(Date.now() - (startTimeRef.current || 0));
    }, 100);
  }, [elapsed]);

  const stop = useCallback(() => {
    if (intervalRef.current) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }
  }, []);

  const reset = useCallback(() => {
    stop();
    setElapsed(0);
    startTimeRef.current = null;
  }, [stop]);

  useEffect(() => {
    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
      }
    };
  }, []);

  return {
    elapsed,
    formatted: formatTime(elapsed),
    start,
    stop,
    reset,
  };
}
