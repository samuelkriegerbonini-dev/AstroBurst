import { FILE_STATUS } from "./constants";
import { ONE_D_ARRAY_HINT } from "./fitsErrors";
import { TABLE_ONLY_HINT } from "./x1dCompare";
import type { PlaneInfo, ProcessedFile } from "../shared/types/fits.types";
import type { LayerGuess, LayerRole } from "../shared/types/layers";
import type { Spectrum1dKind } from "../shared/types/spectrum1d";
import type { TableFormat, TableProbe } from "../shared/types/tables";

export const CSV_SPECTRUM_HINT = "1D spectrum, CSV file";

const TABLE_FORMAT_LABEL: Record<TableFormat, string> = { csv: "CSV", ecsv: "ECSV", fits_bintable: "FITS table" };

export function isSpectrumSeries(file: Pick<ProcessedFile, "status" | "spectrum"> | null | undefined): boolean {
  const kind: string | undefined = file?.spectrum?.kind;
  return file?.status === FILE_STATUS.SPECTRUM && kind === "x1dints";
}

export function spectrumHdu(kind: Spectrum1dKind | null | undefined): number | null {
  if (!kind || !("hdu" in kind)) return null;
  return kind.hdu;
}

function counted(count: number, unit: string): string {
  return `${count} ${unit}${count === 1 ? "" : "s"}`;
}

export interface SpectrumRowNote {
  text: string;
  title: string;
  series: boolean;
}

export function seriesRowText(counts: { n_series: number; n_integrations: number; n_channels: number }): string {
  return `Time-series spectra, ${counted(counts.n_integrations, "integration")} x ${counted(counts.n_channels, "channel")}, ${counts.n_series} series`;
}

export function spectrumRowNote(row: { spectrum?: Spectrum1dKind; table?: Pick<TableProbe, "format"> }): SpectrumRowNote {
  const spectrum = row.spectrum;
  if (spectrum === undefined) {
    const text = row.table?.format === "csv" ? CSV_SPECTRUM_HINT : "1D spectrum";
    return { text, title: text, series: false };
  }
  if (spectrum.kind === "x1dints") {
    const text = seriesRowText(spectrum);
    const layout = spectrum.layout === "rows" ? "; one table row per integration" : "; one EXTRACT1D table per integration";
    const firstSeries = spectrum.n_series > 1 ? "; counts of the first series" : "";
    return { text, title: `${text}${layout}${firstSeries}`, series: true };
  }
  const hdu = spectrumHdu(spectrum);
  const tableCount = spectrum.kind === "x1d" ? (spectrum.n_tables ?? 1) : 1;
  if (tableCount > 1) {
    const text = `1D spectra, ${tableCount} EXTRACT1D tables`;
    return { text, title: `${text}, HDU ${hdu} opens first`, series: false };
  }
  const text = spectrum.kind === "x1d" ? TABLE_ONLY_HINT : ONE_D_ARRAY_HINT;
  return { text, title: `${text}, HDU ${hdu}`, series: false };
}

export function tableRowText(probe: Pick<TableProbe, "format" | "n_rows" | "columns" | "tables">): string {
  const text = `Table, ${counted(probe.n_rows, "row")} x ${counted(probe.columns.length, "column")} (${TABLE_FORMAT_LABEL[probe.format]})`;
  const tableCount = probe.tables?.length ?? 0;
  return tableCount > 1 ? `${text}, ${tableCount} tables` : text;
}

export function tableRowTitle(probe: TableProbe): string {
  const text = tableRowText(probe);
  return probe.guess.reason ? `${text}; ${probe.guess.reason}` : text;
}

export interface LayerTag {
  text: string;
  title: string;
}

const LAYER_ROLE_WORDS: Partial<Record<LayerRole, string>> = {
  sigma: "sigma (error)",
  variance: "variance",
  inverse_variance: "inverse variance",
  mask: "mask",
};

export function layerTag(layer: LayerGuess | null | undefined): LayerTag | null {
  const words = layer ? LAYER_ROLE_WORDS[layer.role] : undefined;
  if (!layer || !words) return null;
  const text = layer.assumed ? `${words} (assumed)` : words;
  return { text, title: `${text} layer, from ${layer.basis}` };
}

export function layerNotes(plane: Pick<PlaneInfo, "err_unavailable" | "dq_unavailable"> | null | undefined): string[] {
  const notes: string[] = [];
  if (plane?.err_unavailable) notes.push(`Error layer not used: ${plane.err_unavailable}`);
  if (plane?.dq_unavailable) notes.push(`Mask layer not used: ${plane.dq_unavailable}`);
  return notes;
}

export function previewableFile<T extends Pick<ProcessedFile, "status">>(file: T | null): T | null {
  return file?.status === FILE_STATUS.TABLE ? null : file;
}

export type TableRouteKind = "light_curve" | "catalogue";

export interface TableRowAction {
  kind: TableRouteKind;
  label: string;
  title: string;
}

export const TABLE_NOTE_HINT = "It has no image to show here.";
export const FITS_TABLE_NOTE_HINT = "Binary tables of a FITS file are listed only: save the table as CSV or ECSV to use it.";

const TABLE_ROW_ACTIONS: Record<TableRouteKind, TableRowAction> = {
  light_curve: { kind: "light_curve", label: "Open as light curve", title: "Open this table as a light curve in the Series section of Photometry" },
  catalogue: { kind: "catalogue", label: "Use as catalogue", title: "Use this table as the catalogue of the Compare section of Photometry" },
};

export function tableRowAction(probe: Pick<TableProbe, "format" | "guess">): TableRowAction | null {
  if (probe.format === "fits_bintable") return null;
  const kind = probe.guess.kind;
  return kind === "light_curve" || kind === "catalogue" ? TABLE_ROW_ACTIONS[kind] : null;
}

export interface TableNote {
  id: string;
  name: string;
  text: string;
  reason: string;
  hint: string;
  action: TableRowAction | null;
}

export function tableNoteOf(file: Pick<ProcessedFile, "id" | "name" | "status" | "table"> | null): TableNote | null {
  if (file?.status !== FILE_STATUS.TABLE || !file.table) return null;
  const table = file.table;
  return {
    id: file.id,
    name: file.name,
    text: tableRowText(table),
    reason: table.guess.reason,
    hint: table.format === "fits_bintable" ? FITS_TABLE_NOTE_HINT : TABLE_NOTE_HINT,
    action: tableRowAction(table),
  };
}
