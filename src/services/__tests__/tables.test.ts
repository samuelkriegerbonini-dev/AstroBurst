import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { probeTable, readTable } from "../tables";
import type {
  TableFormat,
  TableGuessKind,
  TableProbe,
  TableProbeColumn,
  TableProbeHdu,
  TableResult,
  TableResultColumn,
} from "../../shared/types/tables";

const PATH = "C:/d/cat.csv";

const PROBE: TableProbe = {
  format: "csv",
  n_rows: 2,
  columns: [{ name: "ra", unit: null, datatype: "float64", numeric: true }],
  tables: null,
  meta: null,
  comments: [["file", "a.fits"]],
  guess: { kind: "catalogue", reason: "sky position columns ra and dec" },
  notes: [],
};

const RESULT: TableResult = {
  format: "csv",
  n_rows: 2,
  columns: [
    { name: "flux_r", unit: "nanomaggy", datatype: "float64", values: [1.5, null], text: null },
    { name: "type", unit: null, datatype: "string", values: null, text: ["PSF", "REX"] },
  ],
  meta: null,
  comments: [["file", "a.fits"]],
  notes: [],
};

describe("table service", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("probeTable invokes probe_table_cmd with the path and returns the probe unchanged", async () => {
    typedInvokeMock.mockResolvedValue(PROBE);
    const probe = await probeTable(PATH);
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith("probe_table_cmd", { path: PATH });
    expect(probe).toBe(PROBE);
  });

  it("readTable invokes read_table_cmd with null columns by default and with the list when given", async () => {
    typedInvokeMock.mockResolvedValue(RESULT);
    const all = await readTable(PATH);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("read_table_cmd", { path: PATH, columns: null });
    expect(all).toBe(RESULT);
    expect(all.columns[0].values).toEqual([1.5, null]);
    expect(all.columns[1].text).toEqual(["PSF", "REX"]);

    const picked = await readTable(PATH, ["ra", "dec"]);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("read_table_cmd", { path: PATH, columns: ["ra", "dec"] });
    expect(typedInvokeMock).toHaveBeenCalledTimes(2);
    expect(picked).toBe(RESULT);
  });

  it("the table types accept the shapes the commands return", () => {
    expectTypeOf(PROBE).toEqualTypeOf<TableProbe>();
    expectTypeOf(RESULT).toEqualTypeOf<TableResult>();
    expectTypeOf<TableResult["format"]>().toEqualTypeOf<"csv" | "ecsv">();
    expectTypeOf<TableFormat>().toEqualTypeOf<"csv" | "ecsv" | "fits_bintable">();
    expectTypeOf<TableGuessKind>().toEqualTypeOf<"spectrum" | "light_curve" | "catalogue" | "unknown">();
    expectTypeOf<TableProbe["columns"]>().toEqualTypeOf<TableProbeColumn[]>();
    expectTypeOf<TableProbeColumn>().toEqualTypeOf<{
      name: string;
      unit: string | null;
      datatype: string;
      numeric: boolean;
    }>();
    expectTypeOf<TableProbe["tables"]>().toEqualTypeOf<TableProbeHdu[] | null>();
    expectTypeOf<TableProbeHdu>().toEqualTypeOf<{
      hdu: number;
      extname: string | null;
      n_rows: number;
      n_columns: number;
    }>();
    expectTypeOf<TableProbe["meta"]>().toEqualTypeOf<Record<string, unknown> | null>();
    expectTypeOf<TableProbe["comments"]>().toEqualTypeOf<[string, string][]>();
    expectTypeOf<TableResult["columns"]>().toEqualTypeOf<TableResultColumn[]>();
    expectTypeOf<TableResultColumn["values"]>().toEqualTypeOf<(number | null)[] | null>();
    expectTypeOf<TableResultColumn["text"]>().toEqualTypeOf<string[] | null>();
    expectTypeOf(probeTable).toEqualTypeOf<(path: string) => Promise<TableProbe>>();
    expectTypeOf(readTable).parameters.toEqualTypeOf<[path: string, columns?: readonly string[] | null]>();
    expectTypeOf(readTable).returns.toEqualTypeOf<Promise<TableResult>>();
  });
});
