import { RefreshCw } from "lucide-react";
import { currentMission, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { summaryView } from "./logic/resultsView";
import { missionLabelOf } from "./logic/missionLabels";

interface SummaryLineProps {
  state: ArchiveState;
  ctl: Pick<ArchiveController, "search">;
}

export default function SummaryLine({ state, ctl }: SummaryLineProps) {
  const results = state.results;
  if (!results) return null;
  const mission = currentMission(state);
  const view = summaryView({ results, resolved: state.resolved, query: state.resultsQuery, missionLabel: mission?.label ?? missionLabelOf(results.mission) });
  const notes = [...results.messages, ...results.unresolvedTargets.map((t) => `MAST could not resolve ${t}`)].filter(Boolean);
  return (
    <div className="ab-mast-summary-row">
      <div
        data-testid="mast-summary"
        data-ra={view.raDeg ?? undefined}
        data-dec={view.decDeg ?? undefined}
        data-total={view.total}
        data-applied={view.applied}
        className="ab-mast-summary"
        title={view.text}
      >
        {view.text}
      </div>
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
  );
}
