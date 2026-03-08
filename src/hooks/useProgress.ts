import {useCallback, useEffect, useRef, useState} from "react";

const isTauri = (): boolean => !!window.__TAURI_INTERNALS__;

interface ProgressPayload {
  current: number;
  total: number;
  percent: number;
  stage: string;
}

interface ProgressState {
  current: number;
  total: number;
  percent: number;
  stage: string;
  active: boolean;
}

const INITIAL: ProgressState = {
  current: 0,
  total: 0,
  percent: 0,
  stage: "",
  active: false,
};

export function useProgress(eventName: string) {
  const [state, setState] = useState<ProgressState>(INITIAL);
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (!isTauri()) return;

    let mounted = true;

    (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unlistenRef.current = await listen<ProgressPayload>(eventName, (event) => {
        if (!mounted) return;
        const p = event.payload;
        setState({
          current: p.current,
          total: p.total,
          percent: p.percent,
          stage: p.stage,
          active: p.stage !== "complete",
        });
      });
    })();

    return () => {
      mounted = false;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    };
  }, [eventName]);

  const reset = useCallback(() => {
    setState(INITIAL);
  }, []);

  return { ...state, reset };
}
