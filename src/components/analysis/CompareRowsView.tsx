import { memo, useCallback, useEffect, useMemo, useState } from "react";
import { Download } from "lucide-react";
import { useRegionKey } from "../../hooks/useRegionKey";
import { useTimedNotice } from "../../hooks/useTimedNotice";
import {
  compareCsvFileName,
  compareCsvHeaderLines,
  compareCsvText,
  compareMarkRows,
  fluxCellUjy,
  MAX_SHOWN_ROWS,
  orderedRows,
  ratioCell,
  type CompareRun,
} from "../../utils/catalogCompare";
import { formatWithUncertainty, MISSING_VALUE } from "../../utils/formatUncertainty";
import { overlayStore } from "../../utils/overlayStore";
import { saveCsvDialog } from "../../utils/spectrumExport";
import { CATALOG_LAYER_KIND, createCatalogPainter } from "../viewer/painters/catalogPainter";
import { CopyCsvButton, ErrorAlert, SMALL_BUTTON_CLASS, Toggle } from "../ui";
import { reasonText } from "./compareRowsViewModel";

export const COMPARE_LAYER_ID = "catalog-compare";

export interface CompareRowsViewProps {
  run: CompareRun;
  imagePath: string | null;
}

const SAVED_NOTICE_MS = 6000;
const MARKS_LEGEND =
  "Marks: amber for the rows used in the statistics, cyan for rows measured and left out. Rows cut before the measurement have no mark.";
const COPY_TITLE = "Copy every row of the comparison as CSV, with the header lines that say what was compared";
const SAVE_TITLE = "Save the catalogue comparison";
const TEXT_CELL_CLASS = "text-left px-1.5 py-0.5";
const NUMBER_CELL_CLASS = "text-right px-1.5 py-0.5";

function CompareRowsView({ run, imagePath }: CompareRowsViewProps) {
  const [showMarks, setShowMarks] = useState(true);
  const [hoverId, setHoverId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, showSavedPath] = useTimedNotice<string>(SAVED_NOTICE_MS);
  const overlayKey = useRegionKey();
  const report = run.report;
  const ordered = useMemo(() => orderedRows(report.rows), [report]);
  const shown = ordered.slice(0, MAX_SHOWN_ROWS);
  const marks = useMemo(() => compareMarkRows(report.rows), [report]);
  const calibrated = report.band.jy_per_native_unit !== null;

  useEffect(() => {
    if (!overlayKey || !showMarks || marks.placed.length === 0) return;
    overlayStore.add(overlayKey, {
      id: COMPARE_LAYER_ID,
      kind: CATALOG_LAYER_KIND,
      visible: true,
      paint: createCatalogPainter({ rows: marks.placed, matchedIds: marks.usedIds, labels: false, highlightId: hoverId }),
    });
    return () => overlayStore.remove(overlayKey, COMPARE_LAYER_ID);
  }, [overlayKey, showMarks, marks, hoverId]);

  const csvText = useCallback(
    () =>
      compareCsvText(
        report,
        compareCsvHeaderLines(report, { exportedUtc: new Date().toISOString(), request: run.request, presetEdited: run.presetEdited }),
      ),
    [report, run],
  );

  const saveCsv = async () => {
    setError(null);
    try {
      const path = await saveCsvDialog(csvText(), compareCsvFileName(imagePath), SAVE_TITLE);
      if (path) showSavedPath(path);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="space-y-1.5" data-compare-rows>
      <div className="flex flex-wrap items-center gap-1.5">
        <Toggle
          label="Marks on image"
          checked={showMarks}
          disabled={!overlayKey || marks.placed.length === 0}
          accent="amber"
          onChange={setShowMarks}
        />
        <CopyCsvButton text={csvText} title={COPY_TITLE} onError={setError} />
        <button type="button" data-action="save-compare-csv" className={SMALL_BUTTON_CLASS} onClick={() => void saveCsv()}>
          <Download size={10} />
          Save CSV
        </button>
      </div>
      {showMarks && marks.placed.length > 0 && (
        <div className="text-[9px] text-zinc-500" data-marks-legend>
          {MARKS_LEGEND}
        </div>
      )}
      <ErrorAlert message={error} />
      {savedPath && <div className="text-[9px] text-emerald-400/90 break-all">Saved {savedPath}</div>}
      {shown.length > 0 && (
        <div className="max-h-56 overflow-auto border border-zinc-800/60 rounded">
          <table className="w-full text-[9px] font-mono">
            <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
              <tr>
                <th className={NUMBER_CELL_CLASS}>row</th>
                <th className={TEXT_CELL_CLASS}>id</th>
                <th className={TEXT_CELL_CLASS}>class</th>
                <th className={NUMBER_CELL_CLASS}>catalogue (uJy)</th>
                <th className={NUMBER_CELL_CLASS}>{calibrated ? "measured (uJy)" : "measured (image units)"}</th>
                <th className={NUMBER_CELL_CLASS}>ratio</th>
                <th className={TEXT_CELL_CLASS}>left out because</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <tr
                  key={r.row}
                  data-row={r.row}
                  onMouseEnter={() => setHoverId(r.id)}
                  onMouseLeave={() => setHoverId(null)}
                  className={`${r.used ? "text-amber-200" : "text-zinc-400"} ${hoverId === r.id ? "bg-zinc-800/80" : ""}`}
                >
                  <td className={NUMBER_CELL_CLASS}>{r.row}</td>
                  <td className={`${TEXT_CELL_CLASS} truncate max-w-[120px]`} title={r.id}>
                    {r.id}
                  </td>
                  <td className={TEXT_CELL_CLASS}>{r.class ?? MISSING_VALUE}</td>
                  <td className={NUMBER_CELL_CLASS}>{fluxCellUjy(r.flux_cat_jy, r.flux_err_cat_jy)}</td>
                  <td className={NUMBER_CELL_CLASS}>
                    {calibrated ? fluxCellUjy(r.flux_jy, r.flux_err_jy) : formatWithUncertainty(r.flux_native, r.flux_err_native)}
                  </td>
                  <td className={NUMBER_CELL_CLASS}>{ratioCell(r.ratio)}</td>
                  <td className={TEXT_CELL_CLASS}>{reasonText(r)}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {ordered.length > MAX_SHOWN_ROWS && (
            <div className="text-[9px] text-zinc-500 px-1.5 py-0.5" data-rows-note>
              {`first ${MAX_SHOWN_ROWS} of ${ordered.length} rows shown, used rows first; the CSV holds every row`}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export default memo(CompareRowsView);
