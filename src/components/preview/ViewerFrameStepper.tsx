import { memo, type CSSProperties } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { stepFrame, type ViewerFrameStep } from "../../utils/cubeNavigation";

export const FRAME_KEYS_GROUP_CLASS = "group/frame-keys";

const VIEWER_BAR_Z = 10;
const FOCUS_RING_STYLE: CSSProperties = {
  position: "absolute",
  inset: 0,
  zIndex: VIEWER_BAR_Z + 1,
  pointerEvents: "none",
  outline: "2px solid var(--ab-accent)",
  outlineOffset: -2,
};
const PLAYING_TITLE = "Playing in the Cube tool: pause playback to step frames";
const FRAME_KEYS_TIP = "Click the image, then ArrowLeft and ArrowRight step frames and Shift steps by 10";
const STEP_BUTTON =
  "flex items-center justify-center p-1 rounded text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-colors disabled:opacity-30 disabled:cursor-not-allowed disabled:hover:bg-transparent";

export function FrameKeysFocusRing() {
  return <span aria-hidden="true" className="hidden group-focus-visible/frame-keys:block" style={FOCUS_RING_STYLE} />;
}

function readoutTitle({ playing, label, failure }: ViewerFrameStep): string {
  if (playing) return PLAYING_TITLE;
  if (failure !== null) return `Could not load ${label}: ${failure}. Previous and Next step from this frame; Retry it in the Cube tool`;
  return `${label}. ${FRAME_KEYS_TIP}`;
}

function ViewerFrameStepper({ step }: { step: ViewerFrameStep }) {
  const { frame, total, playing, label, failure, origin, request } = step;
  const stepBy = (direction: -1 | 1) => {
    const from = origin();
    const target = stepFrame(from, direction, total, false);
    if (target !== from) request(target);
  };
  const atFirst = stepFrame(frame, -1, total, false) === frame;
  const atLast = stepFrame(frame, 1, total, false) === frame;
  return (
    <div
      role="group"
      aria-label="Cube frame"
      className="flex min-w-0 max-w-full items-center gap-0.5 rounded px-0.5 text-[10px]"
      style={{ border: "1px solid var(--ab-border)" }}
    >
      <button
        type="button"
        onClick={() => stepBy(-1)}
        disabled={playing || atFirst}
        title="Previous frame"
        aria-label="Previous frame"
        className={STEP_BUTTON}
      >
        <ChevronLeft size={12} />
      </button>
      <span
        className={`min-w-0 truncate font-mono tabular-nums ${failure === null ? "text-zinc-400" : "text-red-400"}`}
        title={readoutTitle(step)}
      >
        {failure === null ? label : `Failed: ${label}`}
      </span>
      <button
        type="button"
        onClick={() => stepBy(1)}
        disabled={playing || atLast}
        title="Next frame"
        aria-label="Next frame"
        className={STEP_BUTTON}
      >
        <ChevronRight size={12} />
      </button>
    </div>
  );
}

export default memo(ViewerFrameStepper);
