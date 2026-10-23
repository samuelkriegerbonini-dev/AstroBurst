import { useState } from "react";
import { ChevronDown, Info, SlidersHorizontal, X } from "lucide-react";
import type { MastFilterGroup } from "../../shared/types/mast";
import { currentMission, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { OBJECT_KEY, chipAfterRemoval, objectChips, toggledOption, withoutOption, type ObjectChip } from "./logic/objectFilters";
import { PURE_PARALLEL_NOTE, PURE_PARALLEL_PRESET } from "./logic/presetView";

export const FILTER_PANEL_ID = "mast-filter-panel";
export const FILTERS_TOGGLE_ID = "mast-filters-toggle";

interface ToggleProps {
  state: ArchiveState;
  open: boolean;
  onToggle: () => void;
}

interface FiltersProps {
  state: ArchiveState;
  ctl: Pick<ArchiveController, "setObjectFilters" | "clearObjectFilters">;
}

function groupsOf(state: ArchiveState): MastFilterGroup[] {
  return currentMission(state)?.filterGroups ?? [];
}

function chipTestId(chip: Pick<ObjectChip, "group" | "option">): string {
  return `mast-filter-chip-${chip.group}-${chip.option}`;
}

function focusTestId(testId: string): void {
  document.querySelector<HTMLElement>(`[data-testid="${testId}"]`)?.focus();
}

export function FiltersToggle({ state, open, onToggle }: ToggleProps) {
  const groups = groupsOf(state);
  if (groups.length === 0) return null;
  const active = objectChips(state.form.filters, groups).length;
  return (
    <button
      type="button"
      id={FILTERS_TOGGLE_ID}
      data-testid={FILTERS_TOGGLE_ID}
      className="ab-pill ab-mast-filters-toggle"
      data-active={active > 0 ? "true" : "false"}
      aria-expanded={open}
      aria-controls={FILTER_PANEL_ID}
      aria-label={active ? `Filters, ${active} active` : "Filters"}
      title="Category and object type, from what each proposal declared"
      onClick={onToggle}
    >
      <SlidersHorizontal size={11} aria-hidden="true" />
      Filters
      {active ? (
        <span className="ab-mast-filters-count" aria-hidden="true">
          {active}
        </span>
      ) : null}
      <ChevronDown size={10} aria-hidden="true" className="ab-mast-filters-chevron" data-open={open ? "true" : "false"} />
    </button>
  );
}

export function ActiveFilters({ state, ctl }: FiltersProps) {
  const chips = objectChips(state.form.filters, groupsOf(state));
  if (chips.length === 0) return null;
  const remove = (index: number) => {
    const chip = chips[index];
    const next = chipAfterRemoval(chips, index);
    focusTestId(next ? chipTestId(next) : FILTERS_TOGGLE_ID);
    ctl.setObjectFilters(withoutOption(state.form.filters, OBJECT_KEY[chip.group], chip.option));
  };
  const clear = () => {
    focusTestId(FILTERS_TOGGLE_ID);
    void ctl.clearObjectFilters();
  };
  return (
    <div className="ab-mast-active-row">
      <span className="ab-mast-filter-label" id="mast-active-filters-label">
        Active filters
      </span>
      <ul role="list" data-testid="mast-filter-chips" aria-labelledby="mast-active-filters-label" className="ab-mast-active-chips">
        {chips.map((chip, i) => (
          <li key={chip.id} className="ab-mast-active-item">
            <button type="button" data-testid={chipTestId(chip)} className="ab-mast-chip ab-mast-filter-chip" aria-label={`Remove filter ${chip.label}`} onClick={() => remove(i)}>
              {chip.label}
              <X size={10} aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>
      <button type="button" data-testid="mast-filters-reset" className="ab-btn-secondary ab-mast-filters-reset" aria-label="Clear object filters" onClick={clear}>
        Clear
      </button>
    </div>
  );
}

function FilterGroupView({ group, state, ctl }: FiltersProps & { group: MastFilterGroup }) {
  const [infoOpen, setInfoOpen] = useState(false);
  const infoId = `mast-filter-info-${group.id}`;
  const labelId = `mast-filter-label-${group.id}`;
  const filters = state.form.filters;
  const selected: readonly string[] = filters[OBJECT_KEY[group.id]] ?? [];
  return (
    <fieldset data-testid={`mast-filter-group-${group.id}`} className="ab-mast-filter-group" aria-labelledby={labelId} aria-describedby={infoId}>
      <legend className="ab-mast-filter-legend">
        <span className="ab-mast-filter-label" id={labelId}>
          {group.label}
        </span>
        <button
          type="button"
          className="ab-mast-icon-btn ab-mast-filter-info-btn"
          aria-label={`About ${group.label}`}
          aria-expanded={infoOpen}
          aria-controls={infoId}
          title={group.info}
          onClick={() => setInfoOpen((v) => !v)}
        >
          <Info size={11} aria-hidden="true" />
        </button>
      </legend>
      <div className="ab-mast-filter-options">
        {group.options.map((option) => {
          const on = selected.includes(option.id);
          return (
            <button
              key={option.id}
              type="button"
              data-testid={`mast-filter-option-${group.id}-${option.id}`}
              className="ab-mast-option"
              data-active={on ? "true" : "false"}
              aria-pressed={on}
              title={option.title}
              onClick={() => ctl.setObjectFilters(toggledOption(filters, group, option.id))}
            >
              {option.label}
            </button>
          );
        })}
      </div>
      <p id={infoId} className={infoOpen ? "ab-mast-filter-info" : "sr-only"}>
        {group.info}
      </p>
    </fieldset>
  );
}

export function FilterPanel({ state, ctl, open }: FiltersProps & { open: boolean }) {
  const groups = groupsOf(state);
  if (groups.length === 0) return null;
  return (
    <div id={FILTER_PANEL_ID} data-testid={FILTER_PANEL_ID} role="group" aria-label="Category and object type" className="ab-mast-filter-panel" hidden={!open}>
      {open ? (
        <>
          {state.form.preset === PURE_PARALLEL_PRESET ? (
            <p data-testid="mast-preset-object-note" className="ab-mast-filter-note">
              {PURE_PARALLEL_NOTE}
            </p>
          ) : null}
          {groups.map((group) => (
            <FilterGroupView key={group.id} group={group} state={state} ctl={ctl} />
          ))}
        </>
      ) : null}
    </div>
  );
}
