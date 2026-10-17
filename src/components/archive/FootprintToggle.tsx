import { useId } from "react";
import { Save, Shapes, X } from "lucide-react";
import type { MastMissionId, MastRow } from "../../shared/types/mast";
import { footprintInput } from "../../utils/mastFootprints";
import { footprintsController } from "./state/footprintsController";
import { availabilityOf, footprintFor, isShown, pixelFootprints, shownCount, shownInput, useFootprints } from "./state/footprintsStore";
import { datasetPeek, usePeeks } from "./state/peeksStore";
import { shownText, toggleDisabledReason, toggleLabel } from "./logic/footprintView";
import { footprintBlockText, peekPolygons } from "./logic/peekView";

interface FootprintToggleViewProps {
  datasetId: string;
  blocked: string | null;
  workspaceName: string | null;
  fileName: string | null;
  checking: boolean;
  shown: boolean;
  count: number;
  pending: boolean;
  note: string | null;
  error: string | null;
  canSave: boolean;
  savesPixels: boolean;
  onToggle: () => void;
  onClear: () => void;
  onSave: () => void;
}

export function FootprintToggleView({
  datasetId,
  blocked,
  workspaceName,
  fileName,
  checking,
  shown,
  count,
  pending,
  note,
  error,
  canSave,
  savesPixels,
  onToggle,
  onClear,
  onSave,
}: FootprintToggleViewProps) {
  const reasonId = useId();
  const reason = toggleDisabledReason({ blocked, workspaceName, fileName, checking });
  const file = fileName ?? "image";
  const toggleTitle = reason ?? (shown ? `Remove the footprint of ${datasetId} from ${file}` : `Draw the footprint of ${datasetId} on ${file}`);
  const saveTitle = savesPixels ? `Save the footprints drawn on ${file} as a DS9 region file in image pixels` : "Save this footprint as a DS9 region file in ICRS";
  return (
    <div data-testid="mast-footprint" data-shown={shown ? "true" : "false"} className="ab-mast-footprint">
      <div className="ab-mast-footprint-row">
        <button
          type="button"
          data-testid="mast-footprint-toggle"
          className="ab-btn-secondary ab-mast-action"
          aria-pressed={shown}
          disabled={reason !== null}
          aria-describedby={reason !== null ? reasonId : undefined}
          title={toggleTitle}
          onClick={onToggle}
        >
          <Shapes size={11} aria-hidden="true" />
          {toggleLabel(fileName)}
        </button>
        <button type="button" data-testid="mast-footprint-save" className="ab-btn-secondary ab-mast-action" disabled={!canSave} title={saveTitle} onClick={onSave}>
          <Save size={11} aria-hidden="true" />
          Save .reg
        </button>
        {count > 0 ? (
          <>
            <span data-testid="mast-footprint-count" className="ab-mast-footprint-count">
              {shownText(count, file)}
            </span>
            <button type="button" className="ab-mast-icon-btn" aria-label="Clear the footprints" title="Clear the footprints" onClick={onClear}>
              <X size={12} aria-hidden="true" />
            </button>
          </>
        ) : null}
        {pending ? <span className="ab-mast-footprint-count">Projecting…</span> : null}
        {reason !== null ? (
          <span id={reasonId} data-testid="mast-footprint-reason" className="ab-mast-footprint-reason">
            {reason}
          </span>
        ) : null}
      </div>
      {note ? <p className="ab-mast-detail-note">{note}</p> : null}
      {error ? (
        <p className="ab-mast-detail-note" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}

export default function FootprintToggle({ row, mission }: { row: MastRow; mission: MastMissionId }) {
  const state = useFootprints();
  const peek = datasetPeek(usePeeks(), mission, row.datasetId);
  const input = shownInput(state, footprintInput(row, peek ? peekPolygons(peek) : null));
  const polygons = input.polygons;
  const { blocked, workspaceName, fileName, checking } = availabilityOf(state, input);
  const shown = isShown(state, input);
  const drawn = footprintFor(state, input);
  const count = shownCount(state);
  const savesPixels = pixelFootprints(state).length > 0;
  const offImage = drawn !== null && drawn.polygons.length > 0 && !drawn.onImage ? `The footprint lies outside ${fileName ?? "the image"}` : null;
  return (
    <FootprintToggleView
      datasetId={row.datasetId}
      blocked={footprintBlockText(blocked, mission)}
      workspaceName={workspaceName}
      fileName={fileName}
      checking={checking}
      shown={shown}
      count={count}
      pending={state.pending}
      note={shown ? (drawn?.note ?? offImage) : null}
      error={state.error?.message ?? state.saveError}
      canSave={savesPixels || blocked === null}
      savesPixels={savesPixels}
      onToggle={() => void footprintsController.toggle(row, polygons)}
      onClear={() => footprintsController.clear()}
      onSave={() => void footprintsController.save(row, polygons)}
    />
  );
}
