import { memo, useState, type KeyboardEvent } from "react";
import DecimalField from "../ui/DecimalField";
import { decimalFieldState } from "../../utils/decimalText";
import { documentDetails, wavelengthUnitAssumed } from "../../utils/spectrumDocument";
import { pickKey } from "../../hooks/useSpectrum1dStore";
import {
  selectionKey,
  spectrumSelectionStore,
  type ActiveSpectrumDocument,
  type SpectrumSelection,
} from "../../hooks/useActiveSpectrumDocument";
import type { SpectrumSeriesOption, SpectrumTableOption, WavelengthUnitOverride } from "../../services/spectrum1d";
import type { TsoSeriesPick } from "../../shared/types/spectrum1d";

interface SpectrumSourcePickerProps {
  active: ActiveSpectrumDocument;
}

type IntegrationField = "integration" | "first" | "last";
type FieldDrafts = Partial<Record<IntegrationField, string>>;
type SeriesSelection = Extract<SpectrumSelection, { kind: "series" }>;

const LABEL_CLASS = "text-[9px] text-zinc-500 uppercase";
const SELECT_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] text-zinc-200 focus:border-violet-500/50 min-w-0 max-w-full";
const FIELD_CLASS =
  "bg-zinc-900 border border-zinc-700/50 rounded px-1.5 py-0.5 text-[10px] font-mono text-zinc-200 focus:border-violet-500/50 w-12";
const ROW_CLASS = "flex items-center gap-2 flex-wrap";
const AMBER_NOTE_CLASS = "text-amber-300/80 px-2 py-1 rounded bg-amber-900/15 border border-amber-800/20 break-words";
const NO_WAVELENGTH_SOLUTION = "no wavelength solution";
const UNIT_HINT = "use this when the header gives no unit for the wavelength axis; it does not change files that state their unit";
const AS_READ = "";
const UNIT_CHOICES: readonly WavelengthUnitOverride[] = ["Angstrom", "nm", "um"];
const PICK_KINDS: readonly TsoSeriesPick["kind"][] = ["median", "mean", "integration"];
const PICK_TEXTS: Record<TsoSeriesPick["kind"], string> = {
  median: "median of all integrations",
  mean: "mean of integrations",
  integration: "one integration",
};
const NO_DRAFTS: FieldDrafts = {};

function tableText(table: SpectrumTableOption): string {
  return table.label ? `${table.label} (HDU ${table.hdu})` : `HDU ${table.hdu}`;
}

function seriesOptionText(series: SpectrumSeriesOption): string {
  const count = series.n_integrations;
  return `${series.label || series.key.detector} (${count} ${count === 1 ? "integration" : "integrations"})`;
}

function defaultPick(kind: TsoSeriesPick["kind"], nIntegrations: number): TsoSeriesPick {
  switch (kind) {
    case "median":
      return { kind: "median" };
    case "mean":
      return { kind: "mean", first: 0, last: Math.max(nIntegrations - 1, 0) };
    case "integration":
      return { kind: "integration", index: 0 };
  }
}

function committedPick(field: IntegrationField, position: number, pick: TsoSeriesPick): TsoSeriesPick | null {
  if (field === "integration") return { kind: "integration", index: position };
  if (pick.kind !== "mean") return null;
  return field === "first" ? { ...pick, first: Math.min(position, pick.last) } : { ...pick, last: Math.max(position, pick.first) };
}

function sameSeries(series: SpectrumSeriesOption, selection: SeriesSelection): boolean {
  return series.key.detector === selection.detector && series.key.sporder === selection.sporder;
}

function SpectrumSourcePicker({ active }: SpectrumSourcePickerProps) {
  const { fileKey, path, kind, selection, options, doc } = active;
  const draftKey = `${fileKey ?? ""}|${selection === null ? "" : selectionKey(selection)}`;
  const [drafts, setDrafts] = useState<{ key: string; values: FieldDrafts }>({ key: "", values: NO_DRAFTS });
  if (path === null) return null;

  const draftValues = drafts.key === draftKey ? drafts.values : NO_DRAFTS;
  const setDraft = (field: IntegrationField, text: string | undefined) =>
    setDrafts({ key: draftKey, values: { ...draftValues, [field]: text } });
  const store = (next: SpectrumSelection) => {
    if (fileKey !== null) spectrumSelectionStore.set(fileKey, next);
  };

  const tableSelection = selection?.kind === "table" ? selection : null;
  const tables = kind?.kind === "x1d" && options.tables !== null && options.tables.length > 1 ? options.tables : null;
  const tablePosition = tables === null ? -1 : tables.findIndex((table) => table.hdu === tableSelection?.hdu);

  const seriesSet = kind?.kind === "x1dints" ? options.series : null;
  const seriesSelection = selection?.kind === "series" ? selection : null;
  const seriesIndex = seriesSet === null || seriesSelection === null ? -1 : seriesSet.series.findIndex((s) => sameSeries(s, seriesSelection));
  const currentSeries = seriesSet === null || seriesIndex < 0 ? null : seriesSet.series[seriesIndex];

  const unitShown =
    kind?.kind === "wcs1d" &&
    tableSelection !== null &&
    (tableSelection.wavelengthUnit !== null ||
      (doc.spectrum !== null && wavelengthUnitAssumed(doc.spectrum)) ||
      (doc.spectrum === null && (doc.error ?? "").includes(NO_WAVELENGTH_SOLUTION)));

  const commitField = (field: IntegrationField) => {
    const text = draftValues[field];
    if (text === undefined || seriesSelection === null || currentSeries === null) return;
    const { value, error } = decimalFieldState(text, { integer: true, min: 1, max: currentSeries.n_integrations });
    if (error !== null || value === null) return;
    setDraft(field, undefined);
    const pick = committedPick(field, value - 1, seriesSelection.pick);
    if (pick !== null && pickKey(pick) !== pickKey(seriesSelection.pick)) store({ ...seriesSelection, pick });
  };
  const handleFieldKey = (field: IntegrationField, e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") commitField(field);
    else if (e.key === "Escape") setDraft(field, undefined);
  };
  const integrationField = (field: IntegrationField, label: string, position: number, nIntegrations: number) => (
    <DecimalField
      integer
      min={1}
      max={nIntegrations}
      value={draftValues[field] ?? String(position + 1)}
      onChange={(text) => setDraft(field, text)}
      onBlur={() => commitField(field)}
      onKeyDown={(e) => handleFieldKey(field, e)}
      aria-label={label}
      className={FIELD_CLASS}
    />
  );

  return (
    <div data-spectrum-source className="px-1 flex flex-col gap-1.5 text-[10px] font-mono">
      {doc.spectrum !== null && (
        <span data-spectrum-details className="text-zinc-500 break-words">
          {documentDetails(doc.spectrum)}
        </span>
      )}
      {options.loading && <span className="text-zinc-500">listing the spectra of this file</span>}
      {options.error !== null && <p className={AMBER_NOTE_CLASS}>{`could not list the spectra of this file: ${options.error}`}</p>}
      {tables !== null && (
        <div data-spectrum-tables className={ROW_CLASS}>
          <span className={LABEL_CLASS}>Table</span>
          <select
            aria-label="Spectrum table"
            value={String(tableSelection?.hdu ?? "")}
            onChange={(e) => store({ kind: "table", hdu: Number(e.target.value), wavelengthUnit: null })}
            className={SELECT_CLASS}
          >
            {tables.map((table) => (
              <option key={table.hdu} value={String(table.hdu)}>
                {tableText(table)}
              </option>
            ))}
          </select>
          {tablePosition >= 0 && <span className="text-zinc-500">{`table ${tablePosition + 1} of ${tables.length}`}</span>}
        </div>
      )}
      {seriesSet !== null && (
        <div data-spectrum-series className="flex flex-col gap-1.5">
          {seriesSelection !== null && currentSeries !== null && (
            <>
              <div className={ROW_CLASS}>
                <span className={LABEL_CLASS}>Series</span>
                <select
                  aria-label="Spectrum series"
                  value={String(seriesIndex)}
                  onChange={(e) => {
                    const next = seriesSet.series[Number(e.target.value)];
                    if (next === undefined) return;
                    store({
                      kind: "series",
                      detector: next.key.detector,
                      sporder: next.key.sporder,
                      pick: defaultPick(seriesSelection.pick.kind, next.n_integrations),
                    });
                  }}
                  className={SELECT_CLASS}
                >
                  {seriesSet.series.map((series, index) => (
                    <option key={`${series.key.detector}:${series.key.sporder ?? ""}`} value={String(index)}>
                      {seriesOptionText(series)}
                    </option>
                  ))}
                </select>
              </div>
              <div className={ROW_CLASS}>
                <span className={LABEL_CLASS}>Spectrum</span>
                <select
                  aria-label="Spectrum of the series"
                  value={seriesSelection.pick.kind}
                  onChange={(e) => {
                    const pickKind = PICK_KINDS.find((k) => k === e.target.value);
                    if (pickKind !== undefined) store({ ...seriesSelection, pick: defaultPick(pickKind, currentSeries.n_integrations) });
                  }}
                  className={SELECT_CLASS}
                >
                  {PICK_KINDS.map((pickKind) => (
                    <option key={pickKind} value={pickKind}>
                      {PICK_TEXTS[pickKind]}
                    </option>
                  ))}
                </select>
                {seriesSelection.pick.kind === "integration" &&
                  integrationField("integration", "Integration", seriesSelection.pick.index, currentSeries.n_integrations)}
                {seriesSelection.pick.kind === "mean" && (
                  <>
                    {integrationField("first", "First integration", seriesSelection.pick.first, currentSeries.n_integrations)}
                    <span className="text-zinc-500">to</span>
                    {integrationField("last", "Last integration", seriesSelection.pick.last, currentSeries.n_integrations)}
                  </>
                )}
              </div>
            </>
          )}
          {seriesSet.warnings.map((warning, i) => (
            <p key={i} data-series-warning className={AMBER_NOTE_CLASS}>
              {warning}
            </p>
          ))}
          {seriesSet.notes.length > 0 && (
            <details className="text-zinc-600">
              <summary className="cursor-pointer select-none">{`notes (${seriesSet.notes.length})`}</summary>
              <ul className="mt-0.5 flex flex-col gap-0.5">
                {seriesSet.notes.map((note, i) => (
                  <li key={i} className="break-words">
                    {note}
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}
      {unitShown && tableSelection !== null && (
        <div data-wavelength-unit className="flex flex-col gap-0.5">
          <div className={ROW_CLASS}>
            <span className={LABEL_CLASS}>Wavelength unit of the file</span>
            <select
              aria-label="Wavelength unit of the file"
              value={tableSelection.wavelengthUnit ?? AS_READ}
              onChange={(e) =>
                store({ kind: "table", hdu: tableSelection.hdu, wavelengthUnit: UNIT_CHOICES.find((unit) => unit === e.target.value) ?? null })
              }
              className={SELECT_CLASS}
            >
              <option value={AS_READ}>as read</option>
              {UNIT_CHOICES.map((unit) => (
                <option key={unit} value={unit}>
                  {unit}
                </option>
              ))}
            </select>
          </div>
          <span className="text-zinc-600">{UNIT_HINT}</span>
        </div>
      )}
    </div>
  );
}

export default memo(SpectrumSourcePicker);
