import { useMemo } from "react";
import { Loader2 } from "lucide-react";
import Banner from "./Banner";
import FirstRun from "./FirstRun";
import RomanPanel from "./RomanPanel";
import SummaryLine from "./SummaryLine";
import ResultsGallery from "./ResultsGallery";
import ResultsTable from "./ResultsTable";
import { BlockingError, EmptyResults, LockedView, NoImagesIntro, NotResolved, UnavailableView } from "./StateViews";
import { currentMission, effectiveView, resultsState, rowsOnScreen, visibleBanner, type ArchiveState } from "./state/archiveReducer";
import type { ArchiveController } from "./state/archiveController";
import { displayRows, isLevelSearch, searchProgress } from "./logic/resultsView";
import type { AroundTarget } from "./logic/aroundQuery";
import { missionLabelOf } from "./logic/missionLabels";
import { useNow } from "./state/useNow";

interface ResultsRegionProps {
  state: ArchiveState;
  ctl: ArchiveController;
  around: AroundTarget | null;
  onEditQuery?: () => void;
  onSwitchToWorkspace?: () => void;
  onOpenDetail?: () => void;
}

function hasCountdown(state: ArchiveState): boolean {
  return state.thumbs.pausedUntil !== null || Object.values(state.banners).some((b) => b?.until !== null && b?.until !== undefined);
}

function ResultsBody({ state, ctl, onOpenDetail }: { state: ArchiveState; ctl: ArchiveController; onOpenDetail?: () => void }) {
  const rows = useMemo(() => displayRows(state.results, state.sort), [state.results, state.sort]);
  const mission = currentMission(state);
  const label = mission?.label ?? missionLabelOf(state.results?.mission ?? state.form.mission);
  if (effectiveView(state) === "table") return <ResultsTable state={state} ctl={ctl} rows={rows} mission={mission} missionLabel={label} onOpenDetail={onOpenDetail} />;
  return <ResultsGallery state={state} ctl={ctl} rows={rows} missionLabel={label} onOpenDetail={onOpenDetail} />;
}

function SearchProgress({ state, ctl }: { state: ArchiveState; ctl: ArchiveController }) {
  const now = useNow(250);
  const run = state.run;
  if (!run) return null;
  const mission = currentMission(state);
  const view = searchProgress(run, {
    missionLabel: mission?.label ?? missionLabelOf(state.form.mission),
    radiusText: state.form.radiusText,
    filters: run.query?.filters ?? state.form.filters,
    now,
    groups: mission?.filterGroups ?? [],
  });
  return (
    <div className="ab-mast-progress" role="status">
      <ul className="ab-mast-progress-lines">
        {view.lines.map((line) => (
          <li key={line.key} className="ab-mast-progress-line" data-line={line.key}>
            {line.key === "search" || (line.key === "resolve" && run.phase === "resolving") ? <Loader2 size={11} aria-hidden="true" className="ab-mast-spin" /> : null}
            <span className="ab-mast-progress-text">{line.text}</span>
            <span className="ab-mast-progress-dots" aria-hidden="true" />
            {line.elapsed ? <span className="ab-mast-mono">{line.elapsed}</span> : null}
          </li>
        ))}
      </ul>
      {view.slowNote ? <p className="ab-mast-progress-note">{view.slowNote}</p> : null}
      <button type="button" className="ab-btn-secondary ab-mast-action" onClick={() => ctl.cancelSearch()}>
        Cancel
        <kbd className="ab-cmdp-kbd">Esc</kbd>
      </button>
    </div>
  );
}

function Skeleton() {
  return (
    <div className="ab-mast-skeleton-row" aria-hidden="true">
      {[0, 1, 2, 3].map((i) => (
        <span key={i} className="ab-mast-skeleton ab-mast-shimmer" />
      ))}
    </div>
  );
}

export default function ResultsRegion({ state, ctl, around, onEditQuery, onSwitchToWorkspace, onOpenDetail }: ResultsRegionProps) {
  const now = useNow(hasCountdown(state) ? 1000 : null);
  const rs = resultsState(state);
  const view = effectiveView(state);
  const banner = visibleBanner(state, now);
  const mission = currentMission(state);
  const editQuery = onEditQuery ?? (() => {});

  const running = rs === "resolving" || rs === "searching";
  const hasRows = !!state.results && state.results.rows.length > 0;
  const showBody = rowsOnScreen(state);

  let content = null;
  switch (rs) {
    case "idle":
      content = <FirstRun ctl={ctl} around={around} onSwitchToWorkspace={onSwitchToWorkspace} />;
      break;
    case "resolving":
    case "searching":
      content = hasRows ? null : <Skeleton />;
      break;
    case "empty":
      content = <EmptyResults state={state} ctl={ctl} />;
      break;
    case "not-resolved":
      content = <NotResolved state={state} ctl={ctl} onEditQuery={editQuery} />;
      break;
    case "error":
      content = state.error ? (
        <BlockingError error={state.error} missionLabel={mission?.label ?? missionLabelOf(state.form.mission)} ctl={ctl} levelSearch={isLevelSearch(state.form, state.presets)} />
      ) : null;
      break;
    case "locked":
      content = mission?.id === "roman" ? <RomanPanel state={state} ctl={ctl} /> : <LockedView state={state} />;
      break;
    case "unavailable":
      content = <UnavailableView state={state} ctl={ctl} />;
      break;
    case "no-images":
      content = <NoImagesIntro state={state} />;
      break;
    default:
      break;
  }

  return (
    <section
      id="mast-results-region"
      role="main"
      aria-label="MAST results"
      data-testid="mast-results"
      data-view={view}
      data-state={rs}
      data-mast-region="results"
      aria-busy={rs === "resolving" || rs === "searching"}
      tabIndex={-1}
      className="ab-mast-results"
    >
      {mission?.id === "roman" && rs !== "locked" ? <RomanPanel state={state} ctl={ctl} /> : null}
      {banner ? <Banner banner={banner} ctl={ctl} /> : null}
      {running ? <SearchProgress state={state} ctl={ctl} /> : null}
      {rs === "ready" || rs === "empty" ? <SummaryLine state={state} ctl={ctl} /> : null}
      {showBody ? (
        <div className={running ? "ab-mast-body ab-mast-dim" : "ab-mast-body"} inert={running}>
          <ResultsBody state={state} ctl={ctl} onOpenDetail={onOpenDetail} />
        </div>
      ) : null}
      {content}
    </section>
  );
}
