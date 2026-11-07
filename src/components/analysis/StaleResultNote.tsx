import type { RefObject } from "react";

const NOTE_CLASS = "text-[10px] leading-snug text-amber-300/80 px-2 py-1 rounded bg-amber-900/15 border border-amber-800/20 break-words";
const SHOW_CUBE_CLASS =
  "shrink-0 px-2 py-0.5 rounded text-[9px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80";

export default function StaleResultNote() {
  return (
    <p role="status" data-stale-result className={NOTE_CLASS}>
      Stale: inputs changed since this result. Run again to update.
    </p>
  );
}

interface ReopenedProductNoteProps {
  label: string;
  onShowCube?: () => void;
  focusAfterShowCube?: RefObject<HTMLElement | null>;
}

export function ReopenedProductNote({ label, onShowCube, focusAfterShowCube }: ReopenedProductNoteProps) {
  return (
    <div role="status" data-reopened-product className={`${NOTE_CLASS} flex items-start gap-2`}>
      <span className="flex-1">{`${label} on screen is from an earlier run. Its parameters and manifest are not shown here; run again to replace it.`}</span>
      {onShowCube && (
        <button
          type="button"
          onClick={() => {
            focusAfterShowCube?.current?.focus();
            onShowCube();
          }}
          className={SHOW_CUBE_CLASS}
        >
          Show cube
        </button>
      )}
    </div>
  );
}
