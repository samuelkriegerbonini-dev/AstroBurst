import { memo, useId, useMemo, useState } from "react";
import { Table } from "lucide-react";
import LightCurveView from "./LightCurveView";
import {
  EMPTY_TABLE_STATE,
  lightCurveTableLoader,
  tablePathOf,
  useLightCurveTableState,
  type TableColumnsChoice,
} from "../../hooks/useLightCurveTable";
import { useActiveLightCurveTable, useLightCurves } from "../../hooks/useLightCurveStore";
import type { LightCurveQuantity, LightCurveTimeScale } from "../../shared/types/lightCurve";
import { TIME_SCALE_NAMES } from "../../utils/lightCurveDoc";
import type { LightCurveTablePlan } from "../../utils/lightCurveTable";
import { fileBaseName } from "../../utils/values";
import { ErrorAlert, SMALL_BUTTON_CLASS } from "../ui";

const SELECT_CLASS = "bg-zinc-900 border border-zinc-800 rounded px-1 py-0.5 text-[10px] text-zinc-300 font-mono w-full";
const LABEL_CLASS = "text-[9px] text-zinc-500";
const NO_ERROR_COLUMN = "";
const TIME_SCALES = Object.keys(TIME_SCALE_NAMES) as LightCurveTimeScale[];
const QUANTITY_OPTIONS: readonly { value: LightCurveQuantity; label: string }[] = [
  { value: "diff_mag", label: "magnitude" },
  { value: "flux", label: "flux" },
  { value: "rel_flux", label: "relative flux" },
];

function seededChoice(manual: TableColumnsChoice | null, plan: LightCurveTablePlan | null): TableColumnsChoice | null {
  if (manual) return manual;
  if (!plan) return null;
  const first = plan.values[0];
  if (first) {
    return {
      timeColumn: first.time.column,
      timeScale: first.time.scale,
      valueColumn: first.column,
      quantity: first.quantity,
      errColumn: first.errColumn,
    };
  }
  const timeColumn = plan.time?.column ?? plan.numericColumns[0] ?? "";
  return {
    timeColumn,
    timeScale: plan.time?.scale ?? "index",
    valueColumn: plan.numericColumns.find((name) => name !== timeColumn) ?? timeColumn,
    quantity: "flux",
    errColumn: null,
  };
}

interface ChoiceEdits {
  from: TableColumnsChoice;
  choice: TableColumnsChoice;
}

function LightCurveTablePanel() {
  const timeColumnId = useId();
  const timeScaleId = useId();
  const valueColumnId = useId();
  const errColumnId = useId();
  const quantityId = useId();
  const state = useLightCurveTableState();
  const activeTable = useActiveLightCurveTable();
  const path = activeTable === null ? state.path : tablePathOf(activeTable);
  const current = state.path === path ? state : EMPTY_TABLE_STATE;
  const docs = useLightCurves(activeTable);
  const seed = useMemo(() => seededChoice(current.manual, current.plan), [current.manual, current.plan]);
  const [edits, setEdits] = useState<ChoiceEdits | null>(null);
  const choice = edits && edits.from === seed ? edits.choice : seed;

  const notes = useMemo(
    () => [...new Set([...docs.flatMap((doc) => doc.notes), ...(current.table?.notes ?? [])])],
    [docs, current.table],
  );

  const edit = (patch: Partial<TableColumnsChoice>) => {
    if (seed === null) return;
    setEdits((prev) => ({ from: seed, choice: { ...(prev && prev.from === seed ? prev.choice : seed), ...patch } }));
  };

  const name = path === null ? "" : fileBaseName(path);
  const columns = current.plan?.numericColumns ?? [];

  return (
    <div className="ab-panel overflow-hidden" data-light-curve-table>
      <div className="flex items-center justify-between gap-2 px-3 py-2 border-b border-zinc-800/50">
        <div className="flex items-center gap-2 min-w-0">
          <Table size={12} className="text-amber-400" />
          <span className="text-[11px] font-semibold text-zinc-300 uppercase tracking-wider whitespace-nowrap">Light curve table</span>
          <span className="text-[10px] text-zinc-500 font-mono truncate" title={path ?? undefined}>
            {name}
          </span>
        </div>
        <button type="button" onClick={() => lightCurveTableLoader.close()} className={SMALL_BUTTON_CLASS}>
          Close table
        </button>
      </div>

      <div className="px-3 py-2 space-y-2">
        {current.loading && <div className="text-[10px] text-zinc-500">Reading the table...</div>}
        <ErrorAlert message={current.error} />

        {current.table && choice && (
          <>
            <div className="grid grid-cols-2 gap-1.5">
              <div className="flex flex-col gap-0.5">
                <label htmlFor={timeColumnId} className={LABEL_CLASS}>
                  time column
                </label>
                <select
                  id={timeColumnId}
                  value={choice.timeColumn}
                  onChange={(e) => edit({ timeColumn: e.target.value })}
                  className={SELECT_CLASS}
                >
                  {columns.map((column) => (
                    <option key={column} value={column}>
                      {column}
                    </option>
                  ))}
                </select>
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={timeScaleId} className={LABEL_CLASS}>
                  time scale
                </label>
                <select
                  id={timeScaleId}
                  value={choice.timeScale}
                  onChange={(e) => edit({ timeScale: e.target.value as LightCurveTimeScale })}
                  className={SELECT_CLASS}
                >
                  {TIME_SCALES.map((scale) => (
                    <option key={scale} value={scale}>
                      {TIME_SCALE_NAMES[scale]}
                    </option>
                  ))}
                </select>
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={valueColumnId} className={LABEL_CLASS}>
                  value column
                </label>
                <select
                  id={valueColumnId}
                  value={choice.valueColumn}
                  onChange={(e) => edit({ valueColumn: e.target.value })}
                  className={SELECT_CLASS}
                >
                  {columns.map((column) => (
                    <option key={column} value={column}>
                      {column}
                    </option>
                  ))}
                </select>
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={errColumnId} className={LABEL_CLASS}>
                  error column
                </label>
                <select
                  id={errColumnId}
                  value={choice.errColumn ?? NO_ERROR_COLUMN}
                  onChange={(e) => edit({ errColumn: e.target.value === NO_ERROR_COLUMN ? null : e.target.value })}
                  className={SELECT_CLASS}
                >
                  <option value={NO_ERROR_COLUMN}>none</option>
                  {columns.map((column) => (
                    <option key={column} value={column}>
                      {column}
                    </option>
                  ))}
                </select>
              </div>
              <div className="flex flex-col gap-0.5">
                <label htmlFor={quantityId} className={LABEL_CLASS}>
                  quantity
                </label>
                <select
                  id={quantityId}
                  value={choice.quantity}
                  onChange={(e) => edit({ quantity: e.target.value as LightCurveQuantity })}
                  className={SELECT_CLASS}
                >
                  {QUANTITY_OPTIONS.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              <button type="button" onClick={() => lightCurveTableLoader.applyColumns(choice)} className={SMALL_BUTTON_CLASS}>
                Use these columns
              </button>
              {current.manual && (
                <button type="button" onClick={() => lightCurveTableLoader.automatic()} className={SMALL_BUTTON_CLASS}>
                  Automatic columns
                </button>
              )}
            </div>
          </>
        )}

        {activeTable !== null && docs.length > 0 && <LightCurveView group={activeTable} csvName={name.replace(/\.[^.]+$/, "")} />}

        {notes.length > 0 && (
          <details className="text-[9px] text-zinc-500">
            <summary className="cursor-pointer select-none">Table notes</summary>
            <ul className="mt-1 space-y-0.5 list-disc pl-4">
              {notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </div>
  );
}

export default memo(LightCurveTablePanel);
