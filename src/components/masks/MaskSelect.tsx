import type { ChangeEvent } from "react";
import type { RerunSection } from "../../utils/historyRerunStore";
import type { MaskEntry } from "../../utils/maskStore";
import { NO_MASK_LABEL, NO_RUN_MASKS_HINT, maskLabel } from "./maskText";

const RECORDED_OPTION = "recorded";

export interface MaskSelectProps {
  section: RerunSection;
  masks: readonly Pick<MaskEntry, "id" | "recipe">[];
  selectedId: string | null;
  recorded: string | null;
  disabledReason: string | null;
  note: string | null;
  noteId: string;
  busy?: boolean;
  onSelect: (id: string | null) => void;
}

export default function MaskSelect({ section, masks, selectedId, recorded, disabledReason, note, noteId, busy = false, onSelect }: MaskSelectProps) {
  const value = recorded !== null ? RECORDED_OPTION : (selectedId ?? "");
  const hint = masks.length === 0 && recorded === null ? NO_RUN_MASKS_HINT : undefined;
  const handleChange = (event: ChangeEvent<HTMLSelectElement>) => {
    const next = event.target.value;
    if (next !== RECORDED_OPTION) onSelect(next === "" ? null : next);
  };
  return (
    <div className="flex flex-col gap-1">
      <label className="flex items-center gap-2 text-[10px] text-zinc-400">
        <span className="shrink-0">Mask</span>
        <select
          aria-label="Mask"
          data-mask-select={section}
          value={value}
          disabled={busy || disabledReason !== null}
          title={disabledReason ?? hint}
          aria-describedby={note ? noteId : undefined}
          onChange={handleChange}
          className="flex-1 min-w-0 bg-zinc-900 border border-zinc-700/60 rounded px-1.5 py-1 text-[10px] text-zinc-200 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <option value="">{NO_MASK_LABEL}</option>
          {recorded !== null && <option value={RECORDED_OPTION}>{recorded}</option>}
          {masks.map((mask) => (
            <option key={mask.id} value={mask.id}>
              {maskLabel(mask)}
            </option>
          ))}
        </select>
      </label>
      {note && (
        <div id={noteId} role="note" data-mask-blocked="" className="text-[10px] text-amber-300 bg-amber-900/20 border border-amber-800/30 rounded-lg px-3 py-1.5">
          {note}
        </div>
      )}
    </div>
  );
}
