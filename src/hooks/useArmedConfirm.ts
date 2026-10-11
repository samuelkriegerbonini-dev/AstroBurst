import { useCallback, useEffect, useRef, useState } from "react";

export const ARMED_CONFIRM_WINDOW_MS = 3000;
export const CONFIRM_CLEAR_LABEL = "Confirm clear?";
export const CONFIRM_DELETE_LABEL = "Confirm delete?";

export interface ArmedState {
  armedKey: string | null;
  armedAt: number;
}

export type ArmedEvent =
  | { type: "press"; key: string }
  | { type: "timeout"; key: string; armedAt: number }
  | { type: "blur"; key: string };

export interface ArmedStep {
  state: ArmedState;
  run: boolean;
}

export const DISARMED: ArmedState = { armedKey: null, armedAt: 0 };

export function armedConfirmStep(state: ArmedState, event: ArmedEvent, now: number): ArmedStep {
  const inWindow = state.armedKey !== null && now - state.armedAt < ARMED_CONFIRM_WINDOW_MS;
  switch (event.type) {
    case "press":
      if (inWindow && state.armedKey === event.key) return { state: DISARMED, run: true };
      return { state: { armedKey: event.key, armedAt: now }, run: false };
    case "timeout":
      return {
        state: state.armedKey === event.key && state.armedAt === event.armedAt ? DISARMED : state,
        run: false,
      };
    case "blur":
      return { state: state.armedKey === event.key ? DISARMED : state, run: false };
  }
}

export interface ArmedConfirm {
  armedKey: string | null;
  press: (key: string, action: () => void) => void;
  blur: (key: string) => void;
}

export function useArmedConfirm(): ArmedConfirm {
  const [state, setState] = useState<ArmedState>(DISARMED);
  const stateRef = useRef(state);

  const apply = useCallback((event: ArmedEvent): boolean => {
    const step = armedConfirmStep(stateRef.current, event, Date.now());
    stateRef.current = step.state;
    setState(step.state);
    return step.run;
  }, []);

  useEffect(() => {
    const { armedKey, armedAt } = state;
    if (armedKey === null) return;
    const remaining = Math.max(0, armedAt + ARMED_CONFIRM_WINDOW_MS - Date.now());
    const timer = setTimeout(() => apply({ type: "timeout", key: armedKey, armedAt }), remaining);
    return () => clearTimeout(timer);
  }, [state, apply]);

  const press = useCallback(
    (key: string, action: () => void) => {
      if (apply({ type: "press", key })) action();
    },
    [apply],
  );

  const blur = useCallback(
    (key: string) => {
      apply({ type: "blur", key });
    },
    [apply],
  );

  return { armedKey: state.armedKey, press, blur };
}
