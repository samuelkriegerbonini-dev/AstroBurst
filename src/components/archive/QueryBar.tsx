import { useState, type KeyboardEvent, type RefObject } from "react";
import { ChevronDown, Search } from "lucide-react";
import MissionTabs from "./MissionTabs";
import MoreMenu from "./MoreMenu";
import QuickFilters, { ViewToggle } from "./QuickFilters";
import PresetPicker from "./PresetPicker";
import { ActiveFilters, FilterPanel, FiltersToggle } from "./ObjectFilters";
import { currentMission, resultsState, romanLockOf, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { interpretQuery, QUERY_KINDS, type QueryKind } from "./logic/interpretQuery";
import { EMPTY_QUERY_HINT, browserStorage, loadFiltersOpen, saveFiltersOpen } from "./logic/buildQuery";
import { ROMAN_SEARCH_DISABLED, resultsGate } from "./logic/missionState";
import { ROMAN_SEARCH_HINT } from "./logic/romanPanel";

interface QueryBarProps {
  state: ArchiveState;
  ctl: ArchiveController;
  queryRef?: RefObject<HTMLInputElement | null>;
  onLeaveQuery?: () => void;
}

const ERROR_ID = "mast-query-error";
const SEARCH_HINT_ID = "mast-search-hint";
const KIND_TITLES: Record<QueryKind, string> = {
  name: "Resolved with MAST name lookup, then searched by position",
  coordinates: "Searched by position (RA and Dec in degrees or sexagesimal)",
  programme: "Searched by programme number",
  dataset: "Searched by dataset id",
};

function searchBlockReason(state: ArchiveState): string | null {
  const mission = currentMission(state);
  if (!mission) return "MAST missions are loading";
  const gate = resultsGate(mission, romanLockOf(state));
  if (gate === "locked") return ROMAN_SEARCH_DISABLED;
  if (gate === "unavailable") return mission.note ?? `MAST does not serve ${mission.label} searches right now`;
  if (!state.form.text.trim()) return EMPTY_QUERY_HINT;
  return null;
}

export default function QueryBar({ state, ctl, queryRef, onLeaveQuery }: QueryBarProps) {
  const [kindMenu, setKindMenu] = useState(false);
  const [recentOpen, setRecentOpen] = useState(false);
  const [recentAt, setRecentAt] = useState(0);
  const [filtersOpen, setFiltersOpen] = useState(() => loadFiltersOpen(browserStorage()));
  const form = state.form;
  const running = !!state.run;
  const meaning = interpretQuery(form.text, form.kindOverride);
  const kind = meaning.kind === "empty" ? null : meaning.kind;
  const blocked = running ? null : searchBlockReason(state);
  const queryError = state.queryError;
  const fieldInvalid = queryError?.field === "query";
  const radiusInvalid = queryError?.field === "radius";
  const radiusUnused = kind === "programme" || kind === "dataset";
  const recent = state.recent;
  const showRecent = recentOpen && recent.length > 0 && !form.text.trim();
  const state0 = resultsState(state);

  const pickRecent = (text: string) => {
    setRecentOpen(false);
    ctl.setText(text);
    void ctl.search();
  };

  const onQueryKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (showRecent) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setRecentAt((i) => (i + (e.key === "ArrowDown" ? 1 : recent.length - 1)) % recent.length);
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        pickRecent(recent[Math.min(recentAt, recent.length - 1)]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setRecentOpen(false);
        return;
      }
    }
    if (e.key === "ArrowDown" && !form.text.trim() && recent.length) {
      e.preventDefault();
      setRecentAt(0);
      setRecentOpen(true);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      if (!running) void ctl.search();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      if (kindMenu) setKindMenu(false);
      else if (running) ctl.cancelSearch();
      else onLeaveQuery?.();
    }
  };

  const toggleFilters = () => {
    const next = !filtersOpen;
    saveFiltersOpen(browserStorage(), next);
    setFiltersOpen(next);
  };

  const chooseKind = (next: QueryKind | null) => {
    setKindMenu(false);
    ctl.setKindOverride(next);
    queryRef?.current?.focus();
  };

  return (
    <div role="search" aria-label="Search MAST" className="ab-mast-querybar" data-mast-region="query" data-filters-open={filtersOpen ? "true" : "false"}>
      <div className="ab-mast-query-row">
        <MissionTabs state={state} ctl={ctl} />
        <MoreMenu state={state} ctl={ctl} onPicked={() => queryRef?.current?.focus()} />
        <div className="ab-mast-field" data-invalid={fieldInvalid ? "true" : "false"}>
          <Search size={13} aria-hidden="true" className="ab-mast-field-icon" />
          <input
            ref={queryRef}
            data-testid="mast-query"
            data-mast-focus="true"
            className="ab-mast-input"
            type="text"
            value={form.text}
            placeholder="Target, coordinates, programme or dataset id"
            aria-label="Target, coordinates, programme or dataset id"
            aria-invalid={fieldInvalid}
            aria-describedby={fieldInvalid ? ERROR_ID : undefined}
            aria-expanded={showRecent}
            aria-controls={showRecent ? "mast-recent" : undefined}
            aria-autocomplete="list"
            role="combobox"
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => {
              setRecentOpen(false);
              ctl.setText(e.target.value);
            }}
            onKeyDown={onQueryKey}
            onBlur={() => setRecentOpen(false)}
          />
          {kind ? (
            <div
              className="ab-mast-kind"
              onBlur={(e) => {
                if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setKindMenu(false);
              }}
              onKeyDown={(e) => {
                if (e.key !== "Escape" || !kindMenu) return;
                e.preventDefault();
                e.stopPropagation();
                setKindMenu(false);
                queryRef?.current?.focus();
              }}
            >
              <span className="ab-mast-kind-as">as</span>
              <button
                type="button"
                data-testid="mast-query-kind"
                data-override={form.kindOverride ? "true" : "false"}
                className="ab-mast-kind-btn"
                aria-haspopup="menu"
                aria-expanded={kindMenu}
                title={`${KIND_TITLES[kind]}. Click to change.`}
                onClick={() => setKindMenu((v) => !v)}
              >
                {kind}
                <ChevronDown size={10} aria-hidden="true" />
              </button>
              {kindMenu ? (
                <div role="menu" aria-label="Interpret the query as" className="ab-mast-menu">
                  <button type="button" role="menuitemradio" aria-checked={!form.kindOverride} className="ab-mast-menu-item" onClick={() => chooseKind(null)}>
                    automatic
                  </button>
                  {QUERY_KINDS.map((k) => (
                    <button key={k} type="button" role="menuitemradio" aria-checked={form.kindOverride === k} className="ab-mast-menu-item" onClick={() => chooseKind(k)}>
                      {k}
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          ) : null}
          {showRecent ? (
            <ul id="mast-recent" role="listbox" aria-label="Recent searches" className="ab-mast-menu ab-mast-recent">
              {recent.map((r, i) => (
                <li
                  key={r}
                  role="option"
                  aria-selected={i === recentAt}
                  className="ab-mast-menu-item"
                  onMouseDown={(e) => {
                    e.preventDefault();
                    pickRecent(r);
                  }}
                >
                  {r}
                </li>
              ))}
            </ul>
          ) : null}
        </div>
        <label className="ab-mast-radius" title={radiusUnused ? "The radius applies to names and coordinates" : "Search radius in arcminutes, 0.1 to 30"}>
          <span className="sr-only">Radius in arcminutes</span>
          <input
            data-testid="mast-radius"
            className="ab-mast-text ab-mast-text-radius"
            type="text"
            inputMode="decimal"
            value={form.radiusText}
            disabled={radiusUnused}
            aria-invalid={radiusInvalid}
            aria-describedby={radiusInvalid ? ERROR_ID : undefined}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => ctl.setRadius(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (!running) void ctl.search();
              }
            }}
          />
          <span aria-hidden="true" className="ab-mast-radius-unit">
            &apos;
          </span>
        </label>
        {running ? (
          <button type="button" data-testid="mast-search" data-running="true" className="ab-btn-secondary ab-mast-search-btn" onClick={() => ctl.cancelSearch()}>
            Cancel
            <kbd className="ab-cmdp-kbd">Esc</kbd>
          </button>
        ) : (
          <button
            type="button"
            data-testid="mast-search"
            data-running="false"
            className="ab-btn-primary ab-mast-search-btn"
            disabled={!!blocked}
            aria-disabled={!!blocked}
            aria-describedby={blocked === ROMAN_SEARCH_DISABLED ? SEARCH_HINT_ID : undefined}
            title={blocked ?? "Search (Enter)"}
            onClick={() => void ctl.search()}
          >
            Search
          </button>
        )}
      </div>
      {blocked === ROMAN_SEARCH_DISABLED ? (
        <p id={SEARCH_HINT_ID} data-testid="mast-search-hint" className="ab-mast-query-hint">
          {ROMAN_SEARCH_HINT}
        </p>
      ) : null}
      {queryError ? (
        <p id={ERROR_ID} data-testid="mast-query-error" className="ab-mast-query-error">
          {queryError.text}
        </p>
      ) : null}
      {state0 !== "locked" && state0 !== "unavailable" ? (
        <>
          <div className="ab-mast-filter-row">
            <QuickFilters state={state} ctl={ctl} />
            <PresetPicker state={state} ctl={ctl} />
            <FiltersToggle state={state} open={filtersOpen} onToggle={toggleFilters} />
            <ViewToggle state={state} ctl={ctl} />
          </div>
          <ActiveFilters state={state} ctl={ctl} />
          <FilterPanel state={state} ctl={ctl} open={filtersOpen} />
        </>
      ) : null}
    </div>
  );
}
