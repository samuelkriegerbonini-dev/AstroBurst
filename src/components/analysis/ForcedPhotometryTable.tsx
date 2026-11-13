import { memo, useState, type KeyboardEvent } from "react";
import { SMALL_BUTTON_CLASS } from "../ui";
import { forcedPhotometryStore } from "../../hooks/useForcedPhotometryStore";
import { forcedSizeStore, useForcedSizes } from "../../hooks/useForcedSizes";
import { imagePsfStore, useHeaderSeeing, useImagePsf, useResolutionSettings } from "../../hooks/useResolutionSettings";
import type { ForcedPhotometryResult } from "../../shared/types/forcedPhotometry";
import type { ResolutionVerdict } from "../../shared/types/resolution";
import { APP_VERSION } from "../../utils/constants";
import { sizeBand } from "../../utils/forcedPhotometryRequest";
import { VERDICT_LABEL, psfChoice } from "../../utils/resolutionText";
import {
  colourBasis,
  colourCellText,
  fluxCellText,
  forcedColour,
  forcedCsvFileName,
  forcedLongCsv,
  forcedWideCsv,
  magCellText,
} from "../../utils/forcedPhotometry";
import { saveCsvDialog } from "../../utils/spectrumExport";
import { flagsText, fwhmMissing, sizePsfText, sizeRequests, tableColours, tableColumns, type ForcedTableColour } from "./forcedPhotometryTableModel";

export const FORCED_TABLE_ROW_LIMIT = 200;

export type ForcedTableView = "flux" | "mag";
export type ForcedCsvKind = "long" | "wide";

export interface ForcedPhotometryTableProps {
  result: ForcedPhotometryResult;
  selected: number | null;
  currentPath?: string | null;
}

const VIEW_ON_CLASS = "font-semibold text-sky-300 bg-sky-500/10";
const VIEW_OFF_CLASS = "text-zinc-400";
const HEAD_CLASS = "text-left px-1.5 py-0.5 font-normal whitespace-nowrap";
const CELL_CLASS = "px-1.5 py-0.5 whitespace-nowrap";

function colourSentence(result: ForcedPhotometryResult, colours: ForcedTableColour[]): string | null {
  if (colours.length === 0) return null;
  const totals = colours.filter((c) => colourBasis(result.bands[c.a], result.bands[c.b]) === "total").length;
  if (totals === colours.length) return "Colours are between total magnitudes.";
  if (totals === 0) return "Colours are between aperture magnitudes.";
  return "Colours are between total magnitudes where both bands have an aperture correction, aperture magnitudes otherwise.";
}

function sizeAlert(text: string) {
  return (
    <div role="alert" className="ab-error-alert ab-error-alert-compact">
      {text}
    </div>
  );
}

function sizeSentence(label: string, psfText: string): string {
  return `Sizes: FWHM at the forced position in ${label} against a PSF FWHM of ${psfText}; thresholds as set in the Resolution section. A quick look for the first ${FORCED_TABLE_ROW_LIMIT} rows: select a row for the inputs of its verdict.`;
}

function legendText(result: ForcedPhotometryResult, colours: ForcedTableColour[], sizeLegend: string | null): string {
  const units = `Flux in uJy with its one-sigma error (multiplied by the noise factor where a band has one). "< x" is the ${String(result.limit_sigma)} sigma upper limit and "> m" the magnitude it corresponds to; the measured flux stays in the CSV.`;
  return [units, colourSentence(result, colours), sizeLegend].filter((s) => s !== null).join(" ");
}

const NO_FWHM_CELL = "no FWHM";
const NO_FWHM_SENTENCE =
  '"no FWHM": the fit of the width found none at the forced position, which happens on a source too faint for it or far wider than the PSF (a large galaxy, a blend); it is not a verdict. Size such a source with an ellipse in the Image tool, section Profiles (R50 against the PSF).';

function sizeCellText(verdict: ResolutionVerdict | null | undefined, noFwhm: boolean): string {
  if (verdict !== null && verdict !== undefined) return VERDICT_LABEL[verdict];
  return noFwhm ? NO_FWHM_CELL : "--";
}

export function ForcedPhotometryTableInner({ result, selected, currentPath }: ForcedPhotometryTableProps) {
  const [view, setView] = useState<ForcedTableView>("flux");
  const [kind, setKind] = useState<ForcedCsvKind>("long");
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const band = sizeBand(result, currentPath ?? null);
  const bandPath = band !== null ? result.bands[band].path : null;
  const settings = useResolutionSettings();
  const image = useImagePsf(bandPath);
  const choice = psfChoice(settings, image.psf, useHeaderSeeing(settings.psfSource === "header" ? bandPath : null));
  const sizes = useForcedSizes();
  const shownSizes = sizes.result === result ? sizes : null;
  const sizeColumnLabel = shownSizes !== null && shownSizes.band !== null ? result.bands[shownSizes.band].label : null;
  const columns = tableColumns(result);
  const colours = tableColours(result);
  const rows = result.sources.slice(0, FORCED_TABLE_ROW_LIMIT);
  const sizeBandShown = sizeColumnLabel !== null ? shownSizes?.band ?? null : null;
  const noFwhm = rows.map((_, index) => sizeBandShown !== null && fwhmMissing(result, index, sizeBandShown));
  const sizeLegend =
    sizeColumnLabel !== null && shownSizes !== null && shownSizes.psfText !== null
      ? [sizeSentence(sizeColumnLabel, shownSizes.psfText), ...(noFwhm.includes(true) ? [NO_FWHM_SENTENCE] : [])].join(" ")
      : null;

  const checkSizes = () => {
    if (band === null) return;
    void forcedSizeStore.run({
      result,
      band,
      psfText: sizePsfText(choice),
      requests: sizeRequests(result, band, settings, choice, FORCED_TABLE_ROW_LIMIT),
    });
  };

  const sizeStatus = () => {
    if (band === null) return "No band of this run has a measured FWHM: sizes cannot be checked.";
    if (choice.missing !== null) {
      return (
        <>
          {choice.missing}
          {settings.psfSource === "image" && image.psf === null && (
            <button
              type="button"
              data-action="measure-size-psf"
              disabled={image.loading}
              onClick={() => void imagePsfStore.measure(result.bands[band].path)}
              className={`${SMALL_BUTTON_CLASS} ml-1.5`}
            >
              {image.loading ? "Measuring the PSF…" : `Measure the PSF of ${result.bands[band].label}`}
            </button>
          )}
          {image.error !== null && sizeAlert(image.error)}
        </>
      );
    }
    if (image.error !== null) return sizeAlert(image.error);
    if (shownSizes === null) return null;
    if (shownSizes.running) return `Checking sizes: ${shownSizes.checked} of ${shownSizes.total}`;
    if (shownSizes.error !== null) return sizeAlert(`Size check failed: ${shownSizes.error}`);
    return null;
  };
  const status = sizeStatus();

  const csvText = () => {
    const input = { result, exportedUtc: new Date().toISOString(), appVersion: APP_VERSION };
    return kind === "long" ? forcedLongCsv(input) : forcedWideCsv(input);
  };

  const exportCsv = async (write: (text: string) => Promise<string | null>) => {
    setNotice(null);
    setError(null);
    try {
      setNotice(await write(csvText()));
    } catch (e: unknown) {
      setError(`Multi-band export failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const copyCsv = () =>
    exportCsv(async (text) => {
      await navigator.clipboard.writeText(text);
      return `Copied the ${kind} table`;
    });

  const saveCsv = () =>
    exportCsv(async (text) => {
      const path = await saveCsvDialog(text, forcedCsvFileName(kind), "Save the multi-band table");
      return path === null ? null : `Saved ${path}`;
    });

  const toggle = (index: number) => forcedPhotometryStore.select(index === selected ? null : index);

  const onRowKey = (index: number, e: KeyboardEvent<HTMLTableRowElement>) => {
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    toggle(index);
  };

  const viewButton = (value: ForcedTableView, label: string) => (
    <button
      type="button"
      data-view={value}
      aria-pressed={view === value}
      onClick={() => setView(value)}
      className={`text-[9px] px-1.5 py-0.5 transition-colors ${view === value ? VIEW_ON_CLASS : VIEW_OFF_CLASS}`}
    >
      {label}
    </button>
  );

  return (
    <div data-forced-table-block className="space-y-1">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[9px] text-zinc-500 uppercase tracking-wider">Sources</span>
        <div className="flex items-center rounded-md overflow-hidden border border-zinc-700/60">
          {viewButton("flux", "Flux")}
          {viewButton("mag", "AB mag")}
        </div>
        <select
          data-field="csvKind"
          aria-label="CSV form"
          value={kind}
          onChange={(e) => setKind(e.target.value as ForcedCsvKind)}
          className="text-[10px] bg-zinc-900 border border-zinc-700/60 rounded px-1 py-0.5 text-zinc-300"
        >
          <option value="long">long: one row per position and band</option>
          <option value="wide">wide: one row per position</option>
        </select>
        <button type="button" data-action="copy-forced-csv" onClick={() => void copyCsv()} className={SMALL_BUTTON_CLASS}>
          Copy CSV
        </button>
        <button type="button" data-action="save-forced-csv" onClick={() => void saveCsv()} className={SMALL_BUTTON_CLASS}>
          Save CSV
        </button>
        <button
          type="button"
          data-action="check-sizes"
          disabled={band === null || choice.missing !== null || sizes.running}
          title="Resolved or not, for the rows on screen: the FWHM measured at each forced position against the PSF of the Resolution section"
          onClick={checkSizes}
          className={SMALL_BUTTON_CLASS}
        >
          Check sizes
        </button>
      </div>
      {status !== null && (
        <div data-forced-size-status className="text-[9px] text-zinc-400">
          {status}
        </div>
      )}
      {notice !== null && (
        <div role="status" data-forced-export-notice className="text-[9px] text-zinc-400 font-mono break-all">
          {notice}
        </div>
      )}
      {error !== null && (
        <div role="alert" data-forced-export-error className="ab-error-alert ab-error-alert-compact">
          {error}
        </div>
      )}
      <div className="max-h-64 overflow-auto border border-zinc-800/60 rounded">
        <table data-forced-table className="w-full text-[9px] font-mono">
          <caption className="sr-only">Multi-band photometry: one row per position, one column per band</caption>
          <thead className="sticky top-0 bg-zinc-900 text-zinc-500">
            <tr>
              <th scope="col" className={HEAD_CLASS}>
                ID
              </th>
              {columns.map((column) => (
                <th key={`band-${column.band}`} scope="col" title={column.title} className={HEAD_CLASS}>
                  {column.label}
                </th>
              ))}
              {colours.map((colour) => (
                <th key={`colour-${colour.a}-${colour.b}`} scope="col" title={colour.title} className={HEAD_CLASS}>
                  {colour.label}
                </th>
              ))}
              {sizeColumnLabel !== null && (
                <th scope="col" className={HEAD_CLASS}>
                  {`Size (${sizeColumnLabel})`}
                </th>
              )}
              <th scope="col" className={HEAD_CLASS}>
                Flags
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((source, index) => {
              const isSelected = index === selected;
              return (
                <tr
                  key={index}
                  data-forced-row={index}
                  data-selected={isSelected || undefined}
                  aria-selected={isSelected}
                  tabIndex={0}
                  onClick={() => toggle(index)}
                  onKeyDown={(e) => onRowKey(index, e)}
                  className={`cursor-pointer text-zinc-300 hover:bg-zinc-800/60 focus-visible:outline focus-visible:outline-1 focus-visible:outline-sky-400 ${
                    isSelected ? "bg-zinc-800/80 outline outline-1 outline-sky-500/60" : ""
                  }`}
                >
                  <td className={CELL_CLASS}>{source.id}</td>
                  {columns.map((column) => {
                    const cell = source.cells[column.band] ?? null;
                    return (
                      <td key={`band-${column.band}`} className={CELL_CLASS}>
                        {view === "flux" ? fluxCellText(cell, result.bands[column.band]) : magCellText(cell)}
                      </td>
                    );
                  })}
                  {colours.map((colour) => (
                    <td key={`colour-${colour.a}-${colour.b}`} className={CELL_CLASS}>
                      {colourCellText(forcedColour(result, index, colour.a, colour.b))}
                    </td>
                  ))}
                  {sizeColumnLabel !== null && <td className={CELL_CLASS}>{sizeCellText(shownSizes?.verdicts[index], noFwhm[index])}</td>}
                  <td className={CELL_CLASS}>{flagsText(result, index)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {result.sources.length > FORCED_TABLE_ROW_LIMIT && (
          <div data-forced-row-note className="text-[9px] text-zinc-500 px-1.5 py-0.5">
            {`First ${FORCED_TABLE_ROW_LIMIT} of ${result.sources.length} positions shown; the CSV holds every row.`}
          </div>
        )}
      </div>
      <div data-forced-legend className="text-[9px] text-zinc-500">
        {legendText(result, colours, sizeLegend)}
      </div>
    </div>
  );
}

export default memo(ForcedPhotometryTableInner);
