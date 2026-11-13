export type TableFormat = "csv" | "ecsv" | "fits_bintable";

export type TableGuessKind = "spectrum" | "light_curve" | "catalogue" | "unknown";

export interface TableProbeColumn {
  name: string;
  unit: string | null;
  datatype: string;
  numeric: boolean;
}

export interface TableProbeHdu {
  hdu: number;
  extname: string | null;
  n_rows: number;
  n_columns: number;
}

export interface TableGuess {
  kind: TableGuessKind;
  reason: string;
}

export interface TableProbe {
  format: TableFormat;
  n_rows: number;
  columns: TableProbeColumn[];
  tables: TableProbeHdu[] | null;
  meta: Record<string, unknown> | null;
  comments: [string, string][];
  guess: TableGuess;
  notes: string[];
}

export interface TableResultColumn {
  name: string;
  unit: string | null;
  datatype: string;
  values: (number | null)[] | null;
  text: string[] | null;
}

export interface TableResult {
  format: Exclude<TableFormat, "fits_bintable">;
  n_rows: number;
  columns: TableResultColumn[];
  meta: Record<string, unknown> | null;
  comments: [string, string][];
  notes: string[];
}
