import { memo, useCallback, useEffect, useId, useMemo, useState } from "react";
import { ClipboardCopy, FileText, Download, Trash2 } from "lucide-react";
import { useFileContext } from "../../context/PreviewContext";
import { useMeasurementLog } from "../../hooks/useMeasurementLog";
import {
  LOG_SESSION_NOTE,
  MEASUREMENT_KINDS,
  clearConfirmText,
  entrySummary,
  filterEntries,
  measurementLog,
  measurementLogCsv,
  measurementLogCsvFileName,
  type MeasurementKind,
  type MeasurementLogEntry,
} from "../../utils/measurementLog";
import { ErrorAlert, Toggle } from "../ui";

const TABLE_ROW_LIMIT = 200;
const SOURCE_PREVIEW_CHARS = 40;
const SAVED_NOTICE_MS = 6000;
const TIME_START = 11;
const TIME_END = 19;
const EMPTY_TEXT = `No measurements logged yet. Photometry clicks, batch runs, line measurements, time series and cross-matches are logged automatically; region statistics, profiles, sky separation, the pixel table and the statistics panel have a Log button. ${LOG_SESSION_NOTE}`;
const CLEAR_TITLE = `Remove every row of the log, whatever the filters show; asks for confirmation first. ${LOG_SESSION_NOTE}`;
const CLEAR_CONFIRM_MS = 5000;
const NO_MATCH_TEXT = "No rows match the filter.";
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono";
const SMALL_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-zinc-700/60 text-zinc-300 hover:bg-zinc-800/80 disabled:opacity-40 disabled:cursor-not-allowed";
const DANGER_BUTTON_CLASS =
  "flex items-center gap-1 px-2 py-1 rounded text-[10px] border border-red-800/60 text-red-300 hover:bg-red-900/30";
const CELL_CLASS = "px-1.5 py-0.5 text-left whitespace-nowrap border-b border-zinc-800/40";

type KindFilter = MeasurementKind | "all";

function timeOfDay(timestampUtc: string): string {
  return timestampUtc.slice(TIME_START, TIME_END);
}

function sourcePreview(source: string): string {
  return source.length > SOURCE_PREVIEW_CHARS ? `${source.slice(0, SOURCE_PREVIEW_CHARS)}...` : source;
}

function kindsPresent(entries: readonly MeasurementLogEntry[]): MeasurementKind[] {
  const present = new Set(entries.map((e) => e.kind));
  return MEASUREMENT_KINDS.filter((kind) => present.has(kind));
}

function MeasurementLogPanel() {
  const kindId = useId();
  const entries = useMeasurementLog();
  const { file } = useFileContext();
  const [kind, setKind] = useState<KindFilter>("all");
  const [thisFileOnly, setThisFileOnly] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [confirmingClear, setConfirmingClear] = useState(false);

  useEffect(() => {
    if (!confirmingClear) return;
    const timer = setTimeout(() => setConfirmingClear(false), CLEAR_CONFIRM_MS);
    return () => clearTimeout(timer);
  }, [confirmingClear]);

  const fileName = file?.name ?? null;
  const filePath = file?.path ?? null;
  const kinds = useMemo(() => kindsPresent(entries), [entries]);
  const effectiveKind: KindFilter = kind !== "all" && kinds.includes(kind) ? kind : "all";
  const shown = useMemo(
    () => filterEntries(entries, effectiveKind, thisFileOnly ? fileName : null),
    [entries, effectiveKind, thisFileOnly, fileName],
  );
  const newestFirst = useMemo(() => [...shown].reverse().slice(0, TABLE_ROW_LIMIT), [shown]);
  const filtered = effectiveKind !== "all" || thisFileOnly;

  const copyCsv = useCallback(async () => {
    if (shown.length === 0 || busy) return;
    setError(null);
    try {
      await navigator.clipboard.writeText(measurementLogCsv(shown));
    } catch (e: unknown) {
      setError(`Clipboard copy failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }, [shown, busy]);

  const saveCsv = useCallback(async () => {
    if (shown.length === 0 || busy) return;
    setBusy(true);
    setError(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const target = await save({
        defaultPath: measurementLogCsvFileName(thisFileOnly ? filePath : null),
        filters: [{ name: "CSV", extensions: ["csv"] }],
        title: "Save the measurement log as CSV",
      });
      if (!target) return;
      const { writeTextFile } = await import("@tauri-apps/plugin-fs");
      await writeTextFile(target, measurementLogCsv(shown));
      setSavedPath(target);
      setTimeout(() => setSavedPath(null), SAVED_NOTICE_MS);
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [shown, busy, thisFileOnly, filePath]);

  const clear = useCallback(() => {
    setConfirmingClear(false);
    measurementLog.clear();
    setSavedPath(null);
  }, []);

  const showClearConfirm = confirmingClear && entries.length > 0;

  return (
    <div className="ab-panel overflow-hidden">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <FileText size={12} className="text-cyan-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Measurement log</span>
          <span className="font-mono text-[9px] text-zinc-600">{filtered ? `${shown.length}/${entries.length}` : String(entries.length)}</span>
        </div>
      </div>

      <div className="px-3 py-2 space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-1.5">
            <label htmlFor={kindId} className="text-[9px] text-zinc-500 uppercase">
              Kind
            </label>
            <select id={kindId} value={effectiveKind} onChange={(e) => setKind(e.target.value as KindFilter)} className={SELECT_CLASS}>
              <option value="all">all</option>
              {kinds.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </div>
          <div className="min-w-[150px]">
            <Toggle label="This file only" checked={thisFileOnly} disabled={fileName === null} accent="cyan" onChange={setThisFileOnly} />
          </div>
        </div>

        <ErrorAlert message={error} />

        {entries.length === 0 && <div className="text-[10px] text-zinc-600">{EMPTY_TEXT}</div>}
        {entries.length > 0 && shown.length === 0 && <div className="text-[10px] text-zinc-600">{NO_MATCH_TEXT}</div>}

        {newestFirst.length > 0 && (
          <div className="overflow-x-auto rounded-lg border border-zinc-800/50">
            <table className="w-full text-[9px] font-mono">
              <thead>
                <tr className="bg-zinc-900/50 text-zinc-500 uppercase tracking-wider">
                  <th className={CELL_CLASS}>time (UTC)</th>
                  <th className={CELL_CLASS}>kind</th>
                  <th className={CELL_CLASS}>file</th>
                  <th className={CELL_CLASS}>image</th>
                  <th className={CELL_CLASS}>dq</th>
                  <th className={CELL_CLASS}>unit</th>
                  <th className={CELL_CLASS}>summary</th>
                  <th className={CELL_CLASS}>source</th>
                </tr>
              </thead>
              <tbody>
                {newestFirst.map((e) => (
                  <tr key={e.id} className="text-zinc-300">
                    <td className={CELL_CLASS} title={e.timestamp_utc}>
                      {timeOfDay(e.timestamp_utc)}
                    </td>
                    <td className={CELL_CLASS}>{e.kind}</td>
                    <td className={CELL_CLASS}>{e.file ?? "--"}</td>
                    <td className={CELL_CLASS}>{e.image}</td>
                    <td className={CELL_CLASS}>{e.dq}</td>
                    <td className={CELL_CLASS}>{e.unit ?? "--"}</td>
                    <td className={CELL_CLASS}>{entrySummary(e)}</td>
                    <td className={CELL_CLASS} title={e.source}>
                      {sourcePreview(e.source)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
            {shown.length > TABLE_ROW_LIMIT && (
              <div className="text-[9px] text-zinc-600 px-1.5 py-0.5">
                showing {TABLE_ROW_LIMIT} of {shown.length}; the CSV holds every row
              </div>
            )}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-1.5">
          <button type="button" onClick={() => void copyCsv()} disabled={shown.length === 0 || busy} className={SMALL_BUTTON_CLASS} title="Copy the shown rows as CSV">
            <ClipboardCopy size={10} />
            Copy CSV
          </button>
          <button type="button" onClick={() => void saveCsv()} disabled={shown.length === 0 || busy} className={SMALL_BUTTON_CLASS} title="Save the shown rows as a CSV file">
            <Download size={10} />
            Save CSV
          </button>
          {showClearConfirm ? (
            <span role="group" aria-label="Confirm clearing the measurement log" className="ml-auto flex items-center gap-1 text-[10px] text-zinc-300">
              {clearConfirmText(entries.length, shown.length)}
              <button type="button" onClick={clear} className={DANGER_BUTTON_CLASS} title="Remove every row of the log">
                Confirm
              </button>
              <button type="button" onClick={() => setConfirmingClear(false)} className={SMALL_BUTTON_CLASS} title="Keep the log">
                Cancel
              </button>
            </span>
          ) : (
            <button
              type="button"
              onClick={() => setConfirmingClear(true)}
              disabled={entries.length === 0}
              className={`${SMALL_BUTTON_CLASS} ml-auto`}
              title={CLEAR_TITLE}
            >
              <Trash2 size={10} />
              Clear ({entries.length})
            </button>
          )}
        </div>

        {savedPath && <div className="text-[9px] text-emerald-400/90 break-all">Saved {savedPath}</div>}
      </div>
    </div>
  );
}

export default memo(MeasurementLogPanel);
