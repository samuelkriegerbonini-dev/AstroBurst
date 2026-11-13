import { memo, useEffect, useId, useMemo, useState } from "react";
import { X } from "lucide-react";
import { ErrorAlert, RunButton, SMALL_BUTTON_CLASS, Toggle, WarningList } from "../ui";
import ForcedPhotometryInputs from "./ForcedPhotometryInputs";
import ForcedBandCards from "./ForcedBandCards";
import ForcedPhotometryTable from "./ForcedPhotometryTable";
import ForcedSourceDetail from "./ForcedSourceDetail";
import { forcedPhotometryStore, useForcedPhotometry } from "../../hooks/useForcedPhotometryStore";
import { useDoneFiles } from "../../hooks/useFileStore";
import { useProgress } from "../../hooks/useProgress";
import { useRegionKey } from "../../hooks/useRegionKey";
import {
  FORCED_PHOTOMETRY_PROGRESS_EVENT,
  type ForcedPhotometryRequest,
  type ForcedPhotometryResult,
} from "../../shared/types/forcedPhotometry";
import { bandOfPath, forcedRequest, isStale } from "../../utils/forcedPhotometryRequest";
import { FORCED_LAYER_ID, forcedApertureLayer } from "../../utils/forcedOverlay";
import { overlayStore } from "../../utils/overlayStore";
import { measurementLog } from "../../utils/measurementLog";
import { forcedPhotometryEntry } from "../../utils/measurementLogForced";
import { APERTURE_LAYER_KIND, createAperturePainter } from "../viewer/painters/aperturePainter";
import { progressText, runLabel, runSummary } from "./forcedPhotometryPanelModel";

export interface ForcedPhotometryPanelProps {
  filePath: string | null;
  measureKey?: string | null;
}

export function ForcedPhotometryPanelInner({ filePath }: ForcedPhotometryPanelProps) {
  const state = useForcedPhotometry();
  const plan = useMemo(() => forcedRequest(state.fields), [state.fields]);
  const doneFiles = useDoneFiles();
  const files = useMemo(() => doneFiles.map((f) => ({ name: f.name, path: f.path })), [doneFiles]);
  const progress = useProgress(FORCED_PHOTOMETRY_PROGRESS_EVENT);
  const blockedId = useId();
  const blocker = plan.blockers.length > 0 ? plan.blockers[0] : null;
  const result = state.result;
  const resultKey = state.resultKey;
  const overlayKey = useRegionKey();
  const [showApertures, setShowApertures] = useState(true);
  const [logged, setLogged] = useState<{ result: ForcedPhotometryResult; selected: number | null } | null>(null);
  const isLogged = logged !== null && logged.result === result && logged.selected === state.selected;
  const shownBand = result ? bandOfPath(result, filePath) : null;

  useEffect(() => {
    const layer = result && shownBand !== null ? forcedApertureLayer(result, shownBand, state.selected) : null;
    if (!overlayKey || !showApertures || !layer) return;
    overlayStore.add(overlayKey, { id: FORCED_LAYER_ID, kind: APERTURE_LAYER_KIND, visible: true, paint: createAperturePainter(layer) });
    return () => overlayStore.remove(overlayKey, FORCED_LAYER_ID);
  }, [overlayKey, result, shownBand, showApertures, state.selected]);

  return (
    <div className="ab-panel overflow-hidden" data-forced-panel>
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Multi-band</span>
      </div>
      <div className="px-3 py-2 space-y-2">
        <div className="text-[10px] text-zinc-500">
          Measure the same sky positions in several band images with one aperture. Each band is measured on its own pixels through its own WCS; positions are not recentred.
        </div>
        <ForcedPhotometryInputs fields={state.fields} positions={plan.positions} files={files} disabled={state.running} />
        <RunButton
          label={runLabel(plan.positions.positions.length, state.fields.bands.length)}
          runningLabel="Measuring..."
          running={state.running}
          disabled={plan.request === null}
          small
          describedBy={blocker !== null ? blockedId : undefined}
          onClick={() => void forcedPhotometryStore.run()}
        />
        {blocker !== null && (
          <div id={blockedId} data-run-blocked className="text-[9px] text-zinc-500">
            {blocker}
          </div>
        )}
        {state.running && (
          <div data-forced-progress className="flex items-center justify-between gap-2 text-[10px] text-zinc-400 font-mono">
            <span>{progressText(progress)}</span>
            <button
              type="button"
              onClick={() => void forcedPhotometryStore.cancel()}
              title="Cancel"
              aria-label="Cancel the multi-band measurement"
              className={SMALL_BUTTON_CLASS}
            >
              <X size={10} />
              Cancel
            </button>
          </div>
        )}
        <ErrorAlert message={state.error} />
        {result && (
          <>
            <div className="flex items-center justify-between gap-2">
              <div data-forced-summary className="text-[10px] text-zinc-400 font-mono">
                {runSummary(result)}
              </div>
              {resultKey !== null && (
                <button
                  type="button"
                  data-action="log-forced"
                  title="Write this run to the measurement log: the aperture, the bands and, when a row is selected, the fluxes of that source"
                  onClick={() => {
                    measurementLog.append(forcedPhotometryEntry(JSON.parse(resultKey) as ForcedPhotometryRequest, result, state.selected));
                    setLogged({ result, selected: state.selected });
                  }}
                  className={SMALL_BUTTON_CLASS}
                >
                  {isLogged ? "Logged" : "Log"}
                </button>
              )}
            </div>
            {isStale(state.resultKey, plan) && (
              <div data-forced-stale className="text-[10px] text-amber-300/90">
                The fields changed after this run: the results below belong to the earlier request.
              </div>
            )}
            <WarningList warnings={result.warnings} />
            <ForcedBandCards result={result} currentPath={filePath} />
            <ForcedPhotometryTable result={result} selected={state.selected} currentPath={filePath} />
            {shownBand !== null ? (
              <Toggle label="Show apertures" checked={showApertures} disabled={!overlayKey} accent="amber" onChange={setShowApertures} />
            ) : (
              <div data-forced-overlay-hint className="text-[10px] text-zinc-500">
                Select one of the band images in Files to see its apertures on the image.
              </div>
            )}
            {state.selected !== null ? (
              <ForcedSourceDetail result={result} source={state.selected} currentPath={filePath} />
            ) : (
              <div data-forced-select-hint className="text-[10px] text-zinc-500">
                Select a row to see the SED of that source.
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}

export default memo(ForcedPhotometryPanelInner);
