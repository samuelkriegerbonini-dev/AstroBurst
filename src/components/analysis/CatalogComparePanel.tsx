import { memo, useCallback, useEffect, useId, useRef, useState } from "react";
import { Loader2, Table } from "lucide-react";
import { compareWithCatalogue, probeCatalogueTable } from "../../services/catalogCompare";
import { tableRouteStore, useTableRoute } from "../../hooks/useTableRouteStore";
import { useMeasurementProvenance } from "../../hooks/useMeasurementLog";
import { usePhotometryStatus } from "../../hooks/usePhotometryStatus";
import type { CompareProbe } from "../../shared/types/catalogCompare";
import {
  compareRequestOf,
  CUSTOM_PRESET_KEY,
  emptyCompareForm,
  formEdited,
  formFromPreset,
  initialCompareForm,
  presetKeyOf,
  presetLabel,
  presetOfForm,
  type CompareForm,
  type CompareRun,
} from "../../utils/catalogCompare";
import { measurementLog } from "../../utils/measurementLog";
import { catalogCompareEntry } from "../../utils/measurementLogCompare";
import { ErrorAlert, RunButton } from "../ui";
import CompareFormFields from "./CompareFormFields";
import CompareRowsView from "./CompareRowsView";
import CompareSummary from "./CompareSummary";
import StaleResultNote from "./StaleResultNote";

export interface CatalogComparePanelProps {
  filePath: string | null;
  measureKey?: string | null;
}

const INTRO =
  "Measures the sources of a catalogue table you saved (CSV or ECSV) on this image, at the catalogue positions and in the catalogue aperture, and reports measured over catalogue. The app sends no request: save the table from the survey archive first.";
const EDITED_NOTE = "Edited: the values below differ from the preset.";
const NO_IMAGE_REASON = "Select an image first: the comparison measures its pixels at the catalogue positions.";
const NO_TABLE_REASON = "Choose a catalogue table saved as CSV or ECSV.";
const TABLE_DIALOG_NAME = "Catalogue table";
const TABLE_EXTENSIONS = ["csv", "ecsv"];
const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function CatalogComparePanel({ filePath, measureKey = null }: CatalogComparePanelProps) {
  const [tablePath, setTablePath] = useState<string | null>(null);
  const [probe, setProbe] = useState<CompareProbe | null>(null);
  const [probing, setProbing] = useState(false);
  const [form, setForm] = useState<CompareForm>(() => emptyCompareForm(null));
  const [formChosen, setFormChosen] = useState(false);
  const [run, setRun] = useState<CompareRun | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const probeSeqRef = useRef(0);
  const runSeqRef = useRef(0);
  const route = useTableRoute("catalogue");
  const provenance = useMeasurementProvenance();
  const reasonId = useId();

  const openTable = useCallback(async (path: string) => {
    const seq = ++probeSeqRef.current;
    runSeqRef.current++;
    setTablePath(path);
    setProbe(null);
    setRun(null);
    setError(null);
    setRunning(false);
    setProbing(true);
    try {
      const result = await probeCatalogueTable(path);
      if (probeSeqRef.current !== seq) return;
      setProbe(result);
      setForm(initialCompareForm(result));
      setFormChosen(false);
    } catch (e: unknown) {
      if (probeSeqRef.current === seq) setError(messageOf(e));
    } finally {
      if (probeSeqRef.current === seq) setProbing(false);
    }
  }, []);

  const chooseTable = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        multiple: false,
        directory: false,
        title: TABLE_DIALOG_NAME,
        filters: [{ name: TABLE_DIALOG_NAME, extensions: TABLE_EXTENSIONS }],
      });
      const path = Array.isArray(picked) ? picked[0] : picked;
      if (path) void openTable(path);
    } catch (e: unknown) {
      setError(messageOf(e));
    }
  };

  useEffect(() => {
    if (route === null) return;
    void openTable(route.path);
    tableRouteStore.clear("catalogue");
  }, [route, openTable]);

  useEffect(() => {
    runSeqRef.current++;
    setRun(null);
    setRunning(false);
    setError(null);
  }, [filePath, measureKey]);

  const imageBand = usePhotometryStatus(filePath, form.excludeDq, measureKey).status?.filter?.filter ?? null;
  const shownForm = probe !== null && !formChosen ? initialCompareForm(probe, imageBand) : form;
  const built = filePath && probe ? compareRequestOf(shownForm, filePath, probe) : null;
  const blocked = !filePath ? NO_IMAGE_REASON : built === null ? NO_TABLE_REASON : built.error;
  const edited = formEdited(shownForm, probe);
  const stale =
    run !== null && (built === null || built.request === null || JSON.stringify(built.request) !== JSON.stringify(run.request));

  const runCompare = async () => {
    const request = built?.request;
    if (!request) return;
    const seq = ++runSeqRef.current;
    setRunning(true);
    setError(null);
    try {
      const report = await compareWithCatalogue(request);
      if (runSeqRef.current !== seq) return;
      const next: CompareRun = { report, request, presetEdited: edited };
      setRun(next);
      measurementLog.append(catalogCompareEntry(provenance, report, { request: next.request, presetEdited: next.presetEdited }));
    } catch (e: unknown) {
      if (runSeqRef.current === seq) setError(messageOf(e));
    } finally {
      if (runSeqRef.current === seq) setRunning(false);
    }
  };

  const editForm = (next: CompareForm) => {
    setForm(next);
    setFormChosen(true);
  };

  const choosePreset = (key: string) => {
    if (key === CUSTOM_PRESET_KEY) {
      editForm({ ...shownForm, presetKey: CUSTOM_PRESET_KEY });
      return;
    }
    const preset = probe?.presets.find((p) => presetKeyOf(p) === key);
    if (preset) editForm({ ...formFromPreset(preset), excludeDq: shownForm.excludeDq });
  };

  return (
    <div className="ab-panel overflow-hidden" data-compare-panel>
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2">
          <Table size={12} className="text-cyan-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider">Catalogue comparison</span>
        </div>
        {(probing || running) && <Loader2 size={12} className="animate-spin text-cyan-400/70" />}
      </div>

      <div className="px-3 py-2 space-y-2">
        {tablePath === null && (
          <div className="text-[10px] text-zinc-500" data-compare-intro>
            {INTRO}
          </div>
        )}
        <RunButton
          label="Choose table"
          runningLabel="Reading the table..."
          running={probing}
          disabled={running}
          accent="cyan"
          small
          onClick={() => void chooseTable()}
        />
        <ErrorAlert message={error} />

        {probe && (
          <>
            <div className="text-[10px] text-zinc-400 font-mono break-all" data-compare-table>
              {`${probe.file_name}: ${probe.n_rows} rows, ${probe.columns.length} columns (${probe.format.toUpperCase()})`}
            </div>
            {probe.notes.map((note) => (
              <div key={note} className="text-[9px] text-zinc-500" data-table-note>
                {note}
              </div>
            ))}
            <label className="flex flex-col gap-0.5">
              <span className="text-[9px] text-zinc-500 uppercase">Preset</span>
              <select
                data-field="preset"
                value={shownForm.presetKey}
                disabled={running}
                onChange={(e) => choosePreset(e.target.value)}
                className={SELECT_CLASS}
              >
                {probe.presets.map((preset) => (
                  <option key={presetKeyOf(preset)} value={presetKeyOf(preset)}>
                    {presetLabel(preset)}
                  </option>
                ))}
                <option value={CUSTOM_PRESET_KEY}>Custom columns</option>
              </select>
            </label>
            {presetOfForm(shownForm, probe)?.notes.map((note) => (
              <div key={note} className="text-[9px] text-zinc-500" data-preset-note>
                {note}
              </div>
            ))}
            {edited && (
              <div className="text-[9px] text-amber-400/90" data-preset-edited>
                {EDITED_NOTE}
              </div>
            )}
            <CompareFormFields form={shownForm} columns={probe.columns} disabled={running} onChange={editForm} />
          </>
        )}

        <div title={blocked ?? undefined}>
          <RunButton
            label="Compare with the catalogue"
            runningLabel="Measuring the catalogue positions..."
            running={running}
            disabled={blocked !== null || probing}
            accent="amber"
            describedBy={blocked ? reasonId : undefined}
            onClick={() => void runCompare()}
          />
        </div>
        {blocked && (
          <div id={reasonId} data-run-blocked className="text-[9px] text-zinc-500">
            {blocked}
          </div>
        )}

        {run && (
          <>
            {stale && <StaleResultNote />}
            <CompareSummary run={run} />
            <CompareRowsView run={run} imagePath={filePath} />
          </>
        )}
      </div>
    </div>
  );
}

export default memo(CatalogComparePanel);
