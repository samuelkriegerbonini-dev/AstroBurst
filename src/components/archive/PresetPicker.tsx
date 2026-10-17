import type { ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { presetById, presetTitle, presetsFor } from "./logic/presetView";

interface PresetPickerProps {
  state: ArchiveState;
  ctl: Pick<ArchiveController, "setPreset">;
}

const NO_PRESET_TITLE = "Narrow the search to one kind of JWST observation";

export default function PresetPicker({ state, ctl }: PresetPickerProps) {
  const options = presetsFor(state.presets, state.form.mission);
  if (options.length === 0) return null;
  const active = presetById(options, state.form.preset);
  return (
    <div className="ab-mast-filter">
      <label className="ab-mast-filter-label" htmlFor="mast-preset">
        Preset
      </label>
      <select
        id="mast-preset"
        data-testid="mast-preset"
        className="ab-select ab-mast-select"
        value={active?.id ?? ""}
        disabled={!!state.run}
        title={active ? presetTitle(active) : NO_PRESET_TITLE}
        onChange={(e) => ctl.setPreset(e.target.value || null, { rerun: true })}
      >
        <option value="">none</option>
        {options.map((p) => (
          <option key={p.id} value={p.id} title={presetTitle(p)}>
            {p.label}
          </option>
        ))}
      </select>
    </div>
  );
}
