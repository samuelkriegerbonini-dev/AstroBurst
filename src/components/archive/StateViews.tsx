import { Check, CloudOff, Loader2, Lock, RefreshCw, Telescope, X } from "lucide-react";
import type { MastError } from "../../shared/types/mast";
import { currentMission, romanLockOf, unavailableInfo, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { emptyView, notResolvedView } from "./logic/resultsView";
import { errorDetails, errorView } from "./logic/errorCopy";
import { NO_OBJECT_FILTERS } from "./logic/objectFilters";
import { ROMAN_SEARCH_DISABLED, missionEntries, notDeployedView, viewAvailability } from "./logic/missionState";
import { useNow } from "./state/useNow";

interface ViewProps {
  state: ArchiveState;
  ctl: ArchiveController;
}

const CLEAR_ALL = { instrument: undefined, filter: undefined, dateFrom: undefined, dateTo: undefined, publicOnly: undefined, ...NO_OBJECT_FILTERS };

export function EmptyResults({ state, ctl }: ViewProps) {
  const mission = currentMission(state);
  if (!state.results || !mission) return null;
  const v = emptyView({ query: state.resultsQuery, results: state.results, missions: state.missions, mission, presets: state.presets });
  const { widenTo, tryOn } = v;
  return (
    <div className="ab-mast-state">
      <Telescope size={22} aria-hidden="true" className="ab-mast-state-icon" />
      <p className="ab-mast-state-title">{v.title}</p>
      {v.chips.length ? (
        <div className="ab-mast-chips">
          <span className="ab-mast-state-text">Active filters:</span>
          {v.chips.map((c) => (
            <button key={c.id} type="button" className="ab-mast-chip" aria-label={`Remove filter ${c.label}`} onClick={() => void ctl.removeFilter(c.id)}>
              <Check size={10} aria-hidden="true" />
              {c.label}
              <X size={10} aria-hidden="true" />
            </button>
          ))}
        </div>
      ) : null}
      {v.classLine ? <p className="ab-mast-state-text">{v.classLine}</p> : null}
      <div className="ab-mast-actions">
        {widenTo !== null ? (
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => void ctl.widen(widenTo)}>
            {`Widen to ${widenTo}'`}
          </button>
        ) : null}
        {v.clearFilters ? (
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.setFilters(CLEAR_ALL, { rerun: true })}>
            Clear filters
          </button>
        ) : null}
        {v.allLevels ? (
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.setFilters({ level: "all" }, { rerun: true })}>
            All levels
          </button>
        ) : null}
        {tryOn ? (
          <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => void ctl.tryMission(tryOn.mission)}>
            {tryOn.label}
          </button>
        ) : null}
      </div>
    </div>
  );
}

export function NotResolved({ state, ctl, onEditQuery }: ViewProps & { onEditQuery: () => void }) {
  if (!state.notResolved) return null;
  const v = notResolvedView(state.notResolved.name, state.notResolved.elapsedMs);
  return (
    <div className="ab-mast-state">
      <Telescope size={22} aria-hidden="true" className="ab-mast-state-icon" />
      <p className="ab-mast-state-title">{v.title}</p>
      <p className="ab-mast-state-text">Check the spelling, or type coordinates (274.688 -13.792 or 18:18:45 -13:47:31), a programme number or a dataset id.</p>
      <div className="ab-mast-actions">
        <button type="button" className="ab-btn-secondary ab-mast-action" onClick={onEditQuery}>
          Edit query
        </button>
        {v.programmeOffer ? (
          <button
            type="button"
            className="ab-btn-secondary ab-mast-action"
            onClick={() => {
              ctl.setKindOverride("programme");
              void ctl.search();
            }}
          >
            Search by programme instead
          </button>
        ) : null}
      </div>
    </div>
  );
}

export function BlockingError({
  error,
  missionLabel,
  ctl,
  levelSearch = false,
}: {
  error: MastError;
  missionLabel: string;
  ctl: Pick<ArchiveController, "retry" | "setFilters">;
  levelSearch?: boolean;
}) {
  const view = errorView(error, { surface: "search", hasResults: false, missionLabel, levelSearch });
  const title = view.title ?? view.text;
  return (
    <div data-testid="mast-error" data-code={error.code} data-kind={view.bannerKind ?? undefined} className="ab-mast-state">
      <CloudOff size={22} aria-hidden="true" className="ab-mast-state-icon" />
      <p className="ab-mast-state-title">{title}</p>
      {view.title ? <p className="ab-mast-state-text">{view.text}</p> : null}
      <div className="ab-mast-actions">
        <button type="button" data-testid="mast-try-again" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.retry()}>
          Try again
          <kbd className="ab-cmdp-kbd">R</kbd>
        </button>
        {levelSearch && error.code === "mast_timeout" ? (
          <button type="button" data-testid="mast-timeout-all-levels" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.setFilters({ level: "all" }, { rerun: true })}>
            Search all levels
          </button>
        ) : null}
      </div>
      <details className="ab-mast-details">
        <summary>Details</summary>
        <ul>
          {errorDetails(error).map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      </details>
    </div>
  );
}

export function LockedView({ state }: { state: ArchiveState }) {
  const mission = currentMission(state);
  return (
    <div className="ab-mast-state">
      <Lock size={22} aria-hidden="true" className="ab-mast-state-icon" />
      <p className="ab-mast-state-title">{mission?.id === "roman" ? ROMAN_SEARCH_DISABLED : `${mission?.label ?? "This mission"} needs authorization`}</p>
      <p className="ab-mast-state-text">MAST answers Roman searches only for authorized users. A MAST API token and Roman access granted by STScI are both needed.</p>
    </div>
  );
}

function BackToJwst({ state, ctl }: ViewProps) {
  if (state.form.mission === "jwst") return null;
  return (
    <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.switchMission("jwst")}>
      Back to JWST
    </button>
  );
}

export function UnavailableView({ state, ctl }: ViewProps) {
  const now = useNow(60_000);
  const mission = currentMission(state);
  const info = unavailableInfo(state);
  if (!mission || !info) return null;
  const notDeployed = mission.availability === "not_deployed" || state.unavailable?.mission === mission.id;
  if (!notDeployed) {
    const entry = missionEntries(state.missions, romanLockOf(state)).find((e) => e.id === mission.id);
    return (
      <div data-testid="mast-unavailable" data-mission={mission.id} className="ab-mast-state">
        <CloudOff size={22} aria-hidden="true" className="ab-mast-state-icon" />
        <p className="ab-mast-state-title">{`${mission.label} is not available`}</p>
        <p className="ab-mast-state-text">{entry?.reason ?? mission.note ?? "MAST does not serve this mission right now."}</p>
        <div className="ab-mast-actions">
          <BackToJwst state={state} ctl={ctl} />
        </div>
      </div>
    );
  }
  const view = notDeployedView({ label: info.label, operation: info.operation, at: info.at, status: info.status, checkedOn: info.checkedOn, now });
  const probe = state.probes[mission.id];
  const checking = probe?.stage === "checking";
  const failure = probe?.stage === "failed" && probe.error ? errorView(probe.error, { surface: "search", hasResults: true, missionLabel: mission.label }).text : null;
  return (
    <div data-testid="mast-unavailable" data-mission={mission.id} className="ab-mast-state">
      <CloudOff size={22} aria-hidden="true" className="ab-mast-state-icon" />
      <p className="ab-mast-state-title">{view.title}</p>
      <p className="ab-mast-state-text">{view.text}</p>
      {failure ? <p className="ab-mast-state-text ab-mast-state-warn">{`Could not check again: ${failure}`}</p> : null}
      <div className="ab-mast-actions">
        <button
          type="button"
          data-testid="mast-check-again"
          data-checking={checking}
          aria-disabled={checking}
          className="ab-btn-secondary ab-mast-action"
          onClick={() => {
            if (!checking) void ctl.checkMission(mission.id);
          }}
        >
          {checking ? <Loader2 size={11} aria-hidden="true" className="ab-mast-spin" /> : <RefreshCw size={11} aria-hidden="true" />}
          {checking ? "Checking…" : "Check again"}
        </button>
        <BackToJwst state={state} ctl={ctl} />
      </div>
    </div>
  );
}

export function NoImagesIntro({ state }: { state: ArchiveState }) {
  const mission = currentMission(state);
  if (!mission) return null;
  const reason = viewAvailability(mission).galleryReason;
  return (
    <div className="ab-mast-state">
      <Telescope size={22} aria-hidden="true" className="ab-mast-state-icon" />
      <p className="ab-mast-state-title">{`Search ${mission.label} by target, coordinates or dataset id`}</p>
      {reason ? <p className="ab-mast-state-text">{`${reason} Results open in the table.`}</p> : null}
    </div>
  );
}
