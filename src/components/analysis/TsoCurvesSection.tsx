import { memo, useCallback, useId, useMemo, useState } from "react";
import { DecimalField, ErrorAlert, RunButton, SMALL_BUTTON_CLASS, Toggle, WarningList } from "../ui";
import LightCurveView from "./LightCurveView";
import { spectrumSelectionStore } from "../../hooks/useActiveSpectrumDocument";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import { tsoStore, useTsoCurves, useTsoDraft, type TsoSectionProps } from "../../hooks/useTsoStore";
import type { LightCurveDoc } from "../../shared/types/lightCurve";
import type { TsoLightCurves } from "../../shared/types/tso";
import { APP_VERSION } from "../../utils/constants";
import { decimalFieldState } from "../../utils/decimalText";
import {
  droppedLine,
  equalEdgesText,
  limitsLine,
  rejectedLines,
  requestFromDraft,
  selectedPointLine,
} from "../../utils/tsoCurveInputs";
import { MAX_TSO_BINS, bandText } from "../../utils/tsoLightCurves";
import { measurementLog } from "../../utils/measurementLog";
import { tsoLightCurveEntry } from "../../utils/measurementLogTso";
import { saveCsvDialog } from "../../utils/spectrumExport";
import { tsoCurvesCsv, tsoCurvesCsvFileName } from "../../utils/tsoExports";

const SECTION_CLASS = "text-[9px] text-zinc-500 uppercase tracking-wider";
const LABEL_CLASS = "text-[9px] text-zinc-500";
const CAPTION_CLASS = "text-[9px] text-zinc-500";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const INPUT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-2 py-1 text-xs text-zinc-200 font-mono focus:border-amber-500/50 w-full";
const LINE_CLASS = "text-[10px] font-mono text-zinc-400 break-words";

const NOTICE_CLASS = "text-[9px] text-emerald-400/90 break-all";
const NOTICE_MS = 6000;

const DEFAULT_COUNT_TEXT = "5";
const COUNT_OPTIONS = { integer: true, min: 1, max: MAX_TSO_BINS };

const MODE_NOTES = {
  pipeline: "Sum of FLUX with NaN skipped, as the white-light step of the JWST pipeline does: it matches the archived whtlt.",
  masked: "Only spectral elements that are valid in every integration are summed, so every point adds the same elements.",
};

interface GroupValue<T> {
  group: string;
  value: T;
}

interface ExportNotice {
  group: string;
  kind: "saved" | "logged";
  text: string;
}

function TsoCurvesSection({ fileKey, paths, set, series, group }: TsoSectionProps) {
  const modeId = useId();
  const minId = useId();
  const maxId = useId();
  const binsId = useId();
  const excludeId = useId();
  const sigmaId = useId();
  const draft = useTsoDraft(group);
  const curves = useTsoCurves(group);
  const [count, setCount] = useState<GroupValue<string> | null>(null);
  const [picked, setPicked] = useState<GroupValue<number> | null>(null);
  const [saveError, setSaveError] = useState<GroupValue<string> | null>(null);
  const [notice, showNotice] = useTimedNotice<ExportNotice>(NOTICE_MS);
  const countText = count?.group === group ? count.value : DEFAULT_COUNT_TEXT;
  const selected = picked?.group === group ? picked.value : null;

  const input = useMemo(() => requestFromDraft(draft), [draft]);
  const result = curves.result;
  const computedRequest = tsoStore.getRequest(group);
  const stale = result !== null && computedRequest !== null && JSON.stringify(input.request) !== JSON.stringify(computedRequest);

  const countValue = decimalFieldState(countText, COUNT_OPTIONS).value;
  const canFill = countValue !== null && series.wavelength_min_um !== null && series.wavelength_max_um !== null;

  const run = () => {
    if (input.request !== null) void tsoStore.compute(group, paths, series.key, input.request);
  };

  const fill = () => {
    if (countValue !== null) {
      tsoStore.patchDraft(group, { binsText: equalEdgesText(series.wavelength_min_um, series.wavelength_max_um, countValue) });
    }
  };

  const select = useCallback(
    (_doc: LightCurveDoc, pointRef: number) => {
      setPicked({ group, value: pointRef });
      const k = result?.int_num.indexOf(pointRef) ?? -1;
      if (result === null || k < 0) return;
      spectrumSelectionStore.set(fileKey, {
        kind: "series",
        detector: series.key.detector,
        sporder: series.key.sporder,
        pick: { kind: "integration", index: result.row_index[k] },
      });
    },
    [group, result, fileKey, series.key.detector, series.key.sporder],
  );

  const saveCurves = async (shown: TsoLightCurves) => {
    setSaveError(null);
    const context = { software: `AstroBurst ${APP_VERSION}`, exportedUtc: new Date().toISOString(), files: set.files };
    let path: string | null;
    try {
      path = await saveCsvDialog(tsoCurvesCsv(shown, context), tsoCurvesCsvFileName(shown, set.files[0]?.file_name ?? ""), "Save light curves");
    } catch (e) {
      setSaveError({ group, value: `Could not save the file: ${e instanceof Error ? e.message : String(e)}` });
      return;
    }
    if (path === null) return;
    measurementLog.append(tsoLightCurveEntry(shown, set.files, path));
    showNotice({ group, kind: "saved", text: `Saved ${path}` });
  };

  const logCurves = (shown: TsoLightCurves) => {
    setSaveError(null);
    measurementLog.append(tsoLightCurveEntry(shown, set.files, null));
    showNotice({ group, kind: "logged", text: "Added to the measurement log" });
  };

  const shownNotice = notice?.group === group ? notice : null;
  const pointLine = result !== null && selected !== null ? selectedPointLine(result, selected) : null;
  const dropped = result === null ? null : droppedLine(result.dropped);

  return (
    <div className="px-3 py-2 space-y-2" data-tso-curves>
      <div className={SECTION_CLASS}>Light curves</div>

      <div className="flex flex-col gap-0.5">
        <label htmlFor={modeId} className={LABEL_CLASS}>
          Mode
        </label>
        <select
          id={modeId}
          aria-label="Light-curve mode"
          value={draft.mode}
          onChange={(e) => tsoStore.patchDraft(group, { mode: e.target.value === "masked" ? "masked" : "pipeline" })}
          className={SELECT_CLASS}
        >
          <option value="pipeline">as the pipeline (pipeline mode)</option>
          <option value="masked">valid elements only (masked mode)</option>
        </select>
        <div className={CAPTION_CLASS} data-tso-mode-note>
          {MODE_NOTES[draft.mode]}
        </div>
      </div>

      <div className="space-y-0.5">
        <div className="flex gap-2">
          <div className="flex-1 flex flex-col gap-0.5">
            <label htmlFor={minId} className={LABEL_CLASS}>
              from (um)
            </label>
            <DecimalField
              id={minId}
              optional
              aria-label="White-light lower limit in micrometres"
              placeholder="default"
              value={draft.minText}
              onChange={(text) => tsoStore.patchDraft(group, { minText: text })}
              className={INPUT_CLASS}
            />
          </div>
          <div className="flex-1 flex flex-col gap-0.5">
            <label htmlFor={maxId} className={LABEL_CLASS}>
              to (um)
            </label>
            <DecimalField
              id={maxId}
              optional
              aria-label="White-light upper limit in micrometres"
              placeholder="default"
              value={draft.maxText}
              onChange={(text) => tsoStore.patchDraft(group, { maxText: text })}
              className={INPUT_CLASS}
            />
          </div>
        </div>
        <div className={CAPTION_CLASS}>Blank uses the pipeline's limits for this instrument mode.</div>
      </div>

      <div className="space-y-1">
        <label htmlFor={binsId} className={LABEL_CLASS}>
          Bin edges (um)
        </label>
        <div className="flex items-center gap-1">
          <input
            id={binsId}
            type="text"
            aria-label="Bin edges in micrometres"
            placeholder="0.6, 1.0, 2.0, 3.0"
            value={draft.binsText}
            onChange={(e) => tsoStore.patchDraft(group, { binsText: e.target.value })}
            className={INPUT_CLASS}
          />
          <div className="w-12 shrink-0">
            <DecimalField
              {...COUNT_OPTIONS}
              aria-label="Number of equal bins"
              value={countText}
              onChange={(text) => setCount({ group, value: text })}
              className={INPUT_CLASS}
            />
          </div>
          <button
            type="button"
            className={`${SMALL_BUTTON_CLASS} shrink-0`}
            disabled={!canFill}
            onClick={fill}
          >
            Fill equal bins
          </button>
        </div>
        {draft.extraBins.map((pair, index) => (
          <div key={`${pair[0]}-${pair[1]}-${index}`} className="flex items-center justify-between gap-2" data-tso-extra-bin>
            <span className={LINE_CLASS}>{`band ${bandText(pair)}`}</span>
            <button
              type="button"
              className={SMALL_BUTTON_CLASS}
              aria-label={`Remove band ${bandText(pair)}`}
              onClick={() => tsoStore.patchDraft(group, { extraBins: draft.extraBins.filter((_, i) => i !== index) })}
            >
              Remove
            </button>
          </div>
        ))}
      </div>

      {draft.mode === "masked" && (
        <div className="space-y-1" data-tso-masked-options>
          <div className="flex flex-col gap-0.5">
            <label htmlFor={excludeId} className={LABEL_CLASS}>
              Leave out (um)
            </label>
            <input
              id={excludeId}
              type="text"
              aria-label="Wavelength ranges to leave out"
              placeholder="3.72-3.82; 4.2-4.4"
              value={draft.excludeText}
              onChange={(e) => tsoStore.patchDraft(group, { excludeText: e.target.value })}
              className={INPUT_CLASS}
            />
          </div>
          <Toggle
            label="Reject isolated elements"
            checked={draft.rejectOn}
            onChange={(on) => tsoStore.patchDraft(group, { rejectOn: on })}
          />
          <div className="flex items-center gap-2">
            <label htmlFor={sigmaId} className={LABEL_CLASS}>
              robust sigma
            </label>
            <div className="w-20">
              <DecimalField
                id={sigmaId}
                value={draft.rejectSigmaText}
                disabled={!draft.rejectOn}
                onChange={(text) => tsoStore.patchDraft(group, { rejectSigmaText: text })}
                className={INPUT_CLASS}
              />
            </div>
          </div>
          <div className={CAPTION_CLASS}>
            An element is rejected when its time-median lies that many robust sigma from the median of its neighbours.
          </div>
        </div>
      )}

      <RunButton
        label="Compute light curves"
        runningLabel="Computing light curves"
        running={curves.loading}
        disabled={input.request === null}
        onClick={run}
      />
      {input.error !== null && (
        <div className="text-[9px] text-amber-400/90" data-field-error>
          {input.error}
        </div>
      )}

      <ErrorAlert message={curves.error} />

      {result !== null && (
        <div className="space-y-1">
          <div className={LINE_CLASS} data-tso-limits>
            {limitsLine(result.white)}
          </div>
          {dropped !== null && (
            <div className={LINE_CLASS} data-tso-dropped>
              {dropped}
            </div>
          )}
          {rejectedLines(result.rejected_elements).map((line) => (
            <div key={line} className={LINE_CLASS} data-tso-rejected>
              {line}
            </div>
          ))}
          <WarningList warnings={result.warnings} />
          {result.notes.length > 0 && (
            <details className="text-[10px] font-mono text-zinc-500" data-tso-curve-notes>
              <summary className="cursor-pointer select-none">{`notes (${result.notes.length})`}</summary>
              <ul className="mt-0.5 flex flex-col gap-0.5">
                {result.notes.map((note, i) => (
                  <li key={i} className="break-words">
                    {note}
                  </li>
                ))}
              </ul>
            </details>
          )}
          {stale && (
            <div className="text-[10px] text-amber-300" data-tso-stale>
              These curves were computed with other settings: compute again to apply the changes.
            </div>
          )}
          <LightCurveView group={group} csvName="tso_lightcurve" onSelectPoint={select} selectedPointRef={selected} />
          {pointLine !== null && (
            <div className={LINE_CLASS} data-tso-point>
              {pointLine}
            </div>
          )}
          <div className="space-y-1" data-tso-curves-actions>
            <div className="flex items-center gap-1">
              <button type="button" className={SMALL_BUTTON_CLASS} disabled={curves.loading} onClick={() => void saveCurves(result)}>
                Save CSV
              </button>
              <button type="button" className={SMALL_BUTTON_CLASS} disabled={curves.loading} onClick={() => logCurves(result)}>
                Log
              </button>
            </div>
            <ErrorAlert message={saveError?.group === group ? saveError.value : null} />
            {shownNotice?.kind === "saved" && (
              <div role="status" className={NOTICE_CLASS} data-tso-saved>
                {shownNotice.text}
              </div>
            )}
            {shownNotice?.kind === "logged" && (
              <div role="status" className={NOTICE_CLASS} data-tso-logged>
                {shownNotice.text}
              </div>
            )}
          </div>
        </div>
      )}

      {result === null && !curves.loading && (
        <div className="text-[10px] text-zinc-600">Compute the light curves of this series to see the white light and the bins.</div>
      )}
    </div>
  );
}

export default memo(TsoCurvesSection);
