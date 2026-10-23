import { RefreshCw } from "lucide-react";
import { currentMission, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { UNKNOWN_CLEAR_TEXT, summaryView } from "./logic/resultsView";
import { missionLabelOf } from "./logic/missionLabels";

interface SummaryLineProps {
  state: ArchiveState;
  ctl: Pick<ArchiveController, "search" | "clearObjectFilters">;
}

export default function SummaryLine({ state, ctl }: SummaryLineProps) {
  const results = state.results;
  if (!results) return null;
  const mission = currentMission(state);
  const view = summaryView({
    results,
    resolved: state.resolved,
    query: state.resultsQuery,
    missionLabel: mission?.label ?? missionLabelOf(results.mission),
    presets: state.presets,
    groups: mission?.filterGroups ?? [],
  });
  const notes = [...results.messages, ...results.unresolvedTargets.map((t) => `MAST could not resolve ${t}`)].filter(Boolean);
  return (
    <>
      <div className="ab-mast-summary-row">
        <div
          data-testid="mast-summary"
          data-ra={view.raDeg ?? undefined}
          data-dec={view.decDeg ?? undefined}
          data-total={view.total}
          data-applied={view.applied}
          data-preset={view.preset ?? undefined}
          className="ab-mast-summary"
          title={view.text}
        >
          {view.text}
        </div>
        {view.ignored ? (
          <span data-testid="mast-summary-ignored" className="ab-mast-summary-note" title={view.ignored}>
            {view.ignored}
          </span>
        ) : null}
        {notes.length ? (
          <span className="ab-mast-summary-note" title={notes.join("\n")}>
            {notes[0]}
          </span>
        ) : null}
        <button
          type="button"
          className="ab-mast-icon-btn"
          title="Search again without MAST answers cached in AstroBurst"
          aria-label="Refresh"
          disabled={!!state.run}
          onClick={() => void ctl.search({ refresh: true })}
        >
          <RefreshCw size={12} aria-hidden="true" />
        </button>
      </div>
      {view.unknownNote ? (
        <div data-testid="mast-summary-unknown" className="ab-mast-summary-unknown">
          <span className="ab-mast-summary-unknown-text" title={view.unknownNote}>
            {view.unknownNote}
          </span>
          <button type="button" data-testid="mast-summary-unknown-clear" className="ab-mast-link-btn" disabled={!!state.run} onClick={() => void ctl.clearObjectFilters()}>
            {UNKNOWN_CLEAR_TEXT}
          </button>
        </div>
      ) : null}
    </>
  );
}
