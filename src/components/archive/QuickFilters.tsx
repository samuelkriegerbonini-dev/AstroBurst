import type { KeyboardEvent } from "react";
import { LayoutGrid, Table } from "lucide-react";
import { currentMission, effectiveView, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { viewAvailability } from "./logic/missionState";

interface FiltersProps {
  state: ArchiveState;
  ctl: Pick<ArchiveController, "setFilters" | "search" | "setView">;
}

const LEVELS: { value: "combined" | "all"; label: string }[] = [
  { value: "combined", label: "Combined L3" },
  { value: "all", label: "All levels" },
];

export default function QuickFilters({ state, ctl }: FiltersProps) {
  const mission = currentMission(state);
  const quick = mission?.quickFilters ?? [];
  if (quick.length === 0) return <span className="ab-mast-filters-none">No quick filters for this mission</span>;
  const disabled = !!state.run;
  const f = state.form.filters;
  const runOnEnter = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    void ctl.search();
  };

  return (
    <div className="ab-mast-filters" role="group" aria-label="Quick filters">
      {quick.map((q) => {
        switch (q.id) {
          case "level":
            return (
              <div key={q.id} className="ab-mast-filter">
                <span className="ab-mast-filter-label" id="mast-filter-level-label">
                  Level
                </span>
                <div data-testid="mast-filter-level" role="group" aria-labelledby="mast-filter-level-label" className="ab-mast-segment">
                  {LEVELS.map((l) => {
                    const active = (f.level ?? "all") === l.value;
                    return (
                      <button
                        key={l.value}
                        type="button"
                        className="ab-pill ab-mast-seg"
                        data-active={active ? "true" : "false"}
                        aria-pressed={active}
                        disabled={disabled}
                        onClick={() => {
                          if (!active) ctl.setFilters({ level: l.value }, { rerun: true });
                        }}
                      >
                        {l.label}
                      </button>
                    );
                  })}
                </div>
              </div>
            );
          case "instrument":
            return (
              <div key={q.id} className="ab-mast-filter">
                <label className="ab-mast-filter-label" htmlFor="mast-filter-instrument">
                  Instrument
                </label>
                <select
                  id="mast-filter-instrument"
                  data-testid="mast-filter-instrument"
                  className="ab-select ab-mast-select"
                  value={f.instrument ?? ""}
                  disabled={disabled}
                  onChange={(e) => ctl.setFilters({ instrument: e.target.value || undefined }, { rerun: true })}
                >
                  <option value="">any</option>
                  {(q.options ?? []).map((o) => (
                    <option key={o} value={o}>
                      {o}
                    </option>
                  ))}
                </select>
              </div>
            );
          case "filter":
            return (
              <div key={q.id} className="ab-mast-filter">
                <label className="ab-mast-filter-label" htmlFor="mast-filter-filter">
                  Filter
                </label>
                <input
                  id="mast-filter-filter"
                  data-testid="mast-filter-filter"
                  className="ab-mast-text ab-mast-text-filter"
                  value={f.filter ?? ""}
                  placeholder="any"
                  spellCheck={false}
                  autoComplete="off"
                  disabled={disabled}
                  title="Filter name, for example F444W; press Enter to search"
                  onChange={(e) => ctl.setFilters({ filter: e.target.value })}
                  onKeyDown={runOnEnter}
                />
              </div>
            );
          case "date":
            return (
              <div key={q.id} className="ab-mast-filter">
                <span className="ab-mast-filter-label" id="mast-filter-date-label">
                  Date
                </span>
                <div data-testid="mast-filter-date" role="group" aria-labelledby="mast-filter-date-label" className="ab-mast-date">
                  <input
                    type="date"
                    aria-label="Observed from"
                    className="ab-mast-text ab-mast-text-date"
                    value={f.dateFrom ?? ""}
                    disabled={disabled}
                    onChange={(e) => ctl.setFilters({ dateFrom: e.target.value })}
                    onKeyDown={runOnEnter}
                  />
                  <span aria-hidden="true">-</span>
                  <input
                    type="date"
                    aria-label="Observed until"
                    className="ab-mast-text ab-mast-text-date"
                    value={f.dateTo ?? ""}
                    disabled={disabled}
                    onChange={(e) => ctl.setFilters({ dateTo: e.target.value })}
                    onKeyDown={runOnEnter}
                  />
                </div>
              </div>
            );
          case "public":
            return (
              <label key={q.id} className="ab-mast-filter ab-mast-check">
                <input
                  type="checkbox"
                  data-testid="mast-filter-public"
                  checked={!!f.publicOnly}
                  disabled={disabled}
                  onChange={(e) => ctl.setFilters({ publicOnly: e.target.checked || undefined }, { rerun: true })}
                />
                Public only
              </label>
            );
          default:
            return null;
        }
      })}
    </div>
  );
}

export function ViewToggle({ state, ctl }: FiltersProps) {
  const mission = currentMission(state);
  const avail = mission ? viewAvailability(mission) : null;
  const galleryOk = avail ? avail.gallery : true;
  const view = effectiveView(state);
  return (
    <div data-testid="mast-view-toggle" role="group" aria-label="Results view" className="ab-mast-segment">
      <button
        type="button"
        className="ab-pill ab-mast-seg"
        data-view="gallery"
        aria-pressed={view === "gallery"}
        aria-disabled={!galleryOk}
        data-active={view === "gallery" ? "true" : "false"}
        title={galleryOk ? "Gallery (V)" : avail?.galleryReason ?? undefined}
        onClick={() => {
          if (galleryOk) ctl.setView("gallery");
        }}
      >
        <LayoutGrid size={11} aria-hidden="true" />
        Gallery
      </button>
      <button
        type="button"
        className="ab-pill ab-mast-seg"
        data-view="table"
        aria-pressed={view === "table"}
        aria-disabled={false}
        data-active={view === "table" ? "true" : "false"}
        title="Table (V)"
        onClick={() => ctl.setView("table")}
      >
        <Table size={11} aria-hidden="true" />
        Table
      </button>
    </div>
  );
}
