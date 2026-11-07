import { useMemo, useSyncExternalStore, type CSSProperties, type KeyboardEvent } from "react";
import { useRenderContext } from "../../context/PreviewContext";
import { cubeFrameRequesters, frameFromKey, viewerFrameStep, type ViewerFrameStep } from "../../utils/cubeNavigation";

export interface FrameKeyProps {
  tabIndex: 0;
  role: "group";
  "aria-label": string;
  "aria-keyshortcuts": string;
  style: CSSProperties;
  onKeyDown: (event: KeyboardEvent<HTMLElement>) => void;
}

const FRAME_KEY_SHORTCUTS = "ArrowLeft ArrowRight Shift+ArrowLeft Shift+ArrowRight";
const FRAME_KEYS_TARGET_STYLE: CSSProperties = { outline: "none" };

export function useViewerFrameStep(): ViewerFrameStep | null {
  const { processed } = useRenderContext();
  const requesters = useSyncExternalStore(cubeFrameRequesters.subscribe, cubeFrameRequesters.get, cubeFrameRequesters.get);
  return useMemo(() => viewerFrameStep(processed, requesters), [processed, requesters]);
}

export function useViewerFrameKeys(step: ViewerFrameStep | null): FrameKeyProps | undefined {
  return useMemo(() => {
    if (step === null) return undefined;
    return {
      tabIndex: 0,
      role: "group",
      "aria-label": "Cube frame viewer",
      "aria-keyshortcuts": FRAME_KEY_SHORTCUTS,
      style: FRAME_KEYS_TARGET_STYLE,
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
        if (event.target !== event.currentTarget) return;
        const from = step.origin();
        const target = frameFromKey(event.key, event.shiftKey, from, step.total);
        if (target === null) return;
        event.preventDefault();
        if (!step.playing && target !== from) step.request(target);
      },
    };
  }, [step]);
}
