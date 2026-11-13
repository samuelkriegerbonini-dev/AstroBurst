import { describe, expect, it, vi } from "vitest";
import { LightCurveTableLoader, type TableColumnsChoice } from "../useLightCurveTable";
import { LightCurveStore } from "../useLightCurveStore";
import { NO_TIME_COLUMN_NOTE } from "../../utils/lightCurveTable";
import type { TableResult, TableResultColumn } from "../../shared/types/tables";

function num(name: string, values: (number | null)[], unit: string | null = null): TableResultColumn {
  return { name, unit, datatype: "float64", values, text: null };
}

function tableOf(columns: TableResultColumn[]): TableResult {
  return { format: "ecsv", n_rows: columns[0]?.values?.length ?? 0, columns, meta: null, comments: [], notes: [] };
}

const WHTLT: TableResult = {
  format: "ecsv",
  n_rows: 2,
  columns: [
    num("MJD_UTC_NRS1", [60061.4413, 60061.4417]),
    num("BJD_TDB_NRS1", [60061.4422, 60061.4425]),
    num("whitelight_flux_NRS1", [0.1637, 0.164], "Jy"),
  ],
  meta: null,
  comments: [],
  notes: [],
};

const RADEC = tableOf([num("ra", [10.1, 10.2]), num("dec", [-5.1, -5.2]), num("flux_r", [1.5, 1.6])]);

const P = "C:/d/jw02358_whtlt.ecsv";
const G = "table:C:/d/jw02358_whtlt.ecsv";

function setup(table: TableResult = WHTLT) {
  const read = vi.fn<(path: string) => Promise<TableResult>>().mockResolvedValue(table);
  const store = new LightCurveStore();
  const loader = new LightCurveTableLoader(read, store);
  return { read, store, loader };
}

const MJD_CHOICE: TableColumnsChoice = {
  timeColumn: "MJD_UTC_NRS1",
  timeScale: "mjd_utc",
  valueColumn: "whitelight_flux_NRS1",
  quantity: "flux",
  errColumn: null,
};

describe("LightCurveTableLoader", () => {
  it("open reads the table, stores one document per value column and makes the table active", async () => {
    const { read, store, loader } = setup();
    const opening = loader.open(P);
    expect(loader.getState().loading).toBe(true);
    expect(loader.getState().path).toBe(P);
    expect(store.getActiveTable()).toBe(G);
    await opening;
    expect(read).toHaveBeenCalledWith(P);
    const docs = store.get(G);
    expect(docs).toHaveLength(1);
    expect(docs[0].id).toBe("whitelight_flux_NRS1");
    expect(docs[0].timeScale).toBe("bmjd_tdb");
    expect(store.getActiveTable()).toBe(G);
    const state = loader.getState();
    expect(state.loading).toBe(false);
    expect(state.error).toBeNull();
    expect(state.plan?.values).toHaveLength(1);
    expect(state.manual).toBeNull();
    expect(state.table).toBe(WHTLT);
  });

  it("a failed read reports the message and stores nothing", async () => {
    const { read, store, loader } = setup();
    read.mockReset().mockRejectedValue(new Error("the ECSV header has no datatype list"));
    await loader.open(P);
    const state = loader.getState();
    expect(state.error).toBe("the ECSV header has no datatype list");
    expect(state.loading).toBe(false);
    expect(state.table).toBeNull();
    expect(store.get(G)).toHaveLength(0);
  });

  it("a table without a recognised time column opens with the note and no curve, and stays the active table", async () => {
    const { store, loader } = setup(RADEC);
    await loader.open(P);
    const state = loader.getState();
    expect(state.error).toBe(NO_TIME_COLUMN_NOTE);
    expect(store.get(G)).toHaveLength(0);
    expect(store.getActiveTable()).toBe(G);
    expect(state.table).toBe(RADEC);
    expect(state.plan?.numericColumns).toEqual(["ra", "dec", "flux_r"]);
  });

  it("applyColumns builds one curve from the chosen columns and automatic restores the plan", async () => {
    const { store, loader } = setup();
    await loader.open(P);
    loader.applyColumns(MJD_CHOICE);
    expect(store.get(G)).toHaveLength(1);
    expect(store.get(G)[0].timeScale).toBe("mjd_utc");
    expect(store.get(G)[0].time[0]).toBe(60061.4413);
    expect(loader.getState().manual).toEqual(MJD_CHOICE);
    expect(loader.getState().error).toBeNull();
    loader.automatic();
    expect(store.get(G)).toHaveLength(1);
    expect(store.get(G)[0].timeScale).toBe("bmjd_tdb");
    expect(loader.getState().manual).toBeNull();

    const radec = setup(RADEC);
    await radec.loader.open(P);
    radec.loader.applyColumns({ timeColumn: "ra", timeScale: "index", valueColumn: "flux_r", quantity: "flux", errColumn: null });
    expect(radec.store.get(G)).toHaveLength(1);
    expect(radec.store.get(G)[0].value).toEqual([1.5, 1.6]);
    expect(radec.loader.getState().error).toBeNull();
    expect(radec.store.getActiveTable()).toBe(G);
    radec.loader.automatic();
    expect(radec.store.get(G)).toHaveLength(0);
    expect(radec.store.getActiveTable()).toBe(G);
    expect(radec.loader.getState().error).toBe(NO_TIME_COLUMN_NOTE);
    expect(radec.loader.getState().manual).toBeNull();
  });

  it("a choice that cannot be read keeps the stored curve and says why", async () => {
    const withText: TableResult = {
      ...WHTLT,
      columns: [...WHTLT.columns, { name: "file", unit: null, datatype: "string", values: null, text: ["a", "b"] }],
    };
    const { store, loader } = setup(withText);
    await loader.open(P);
    const before = store.get(G);
    loader.applyColumns({ ...MJD_CHOICE, valueColumn: "file" });
    expect(loader.getState().error).toBe("column 'file' holds text, not numbers");
    expect(loader.getState().manual).toBeNull();
    expect(store.get(G)).toBe(before);

    const apart = setup(tableOf([num("t", [1, null]), num("v", [null, 2])]));
    await apart.loader.open(P);
    apart.loader.applyColumns({ timeColumn: "t", timeScale: "index", valueColumn: "v", quantity: "flux", errColumn: null });
    expect(apart.loader.getState().error).toBe("no row has both a time and a value in these columns");
    expect(apart.store.get(G)).toHaveLength(0);
    expect(apart.store.getActiveTable()).toBe(G);
  });

  it("a slow earlier read does not replace a later one", async () => {
    const { read, store, loader } = setup();
    const P1 = "C:/d/first.ecsv";
    const P2 = "C:/d/second.ecsv";
    let finishFirst: (table: TableResult) => void = () => {};
    read.mockReset().mockImplementation((path) =>
      path === P1 ? new Promise<TableResult>((resolve) => (finishFirst = resolve)) : Promise.resolve(WHTLT),
    );
    const first = loader.open(P1);
    await loader.open(P2);
    finishFirst(WHTLT);
    await first;
    expect(loader.getState().path).toBe(P2);
    expect(loader.getState().loading).toBe(false);
    expect(store.getActiveTable()).toBe(`table:${P2}`);
    expect(store.get(`table:${P1}`)).toHaveLength(0);
    expect(store.get(`table:${P2}`)).toHaveLength(1);
  });

  it("close clears the active table and activate shows it again without reading the file", async () => {
    const { read, store, loader } = setup();
    await loader.open(P);
    loader.close();
    expect(store.getActiveTable()).toBeNull();
    expect(store.get(G)).toHaveLength(1);
    await loader.activate(G);
    expect(store.getActiveTable()).toBe(G);
    expect(read).toHaveBeenCalledTimes(1);
    await loader.activate("table:C:/d/other.csv");
    expect(read).toHaveBeenCalledTimes(2);
    expect(read).toHaveBeenLastCalledWith("C:/d/other.csv");
  });

  it("the state object is stable between changes", async () => {
    const { loader } = setup();
    expect(loader.getState()).toBe(loader.getState());
    expect(Object.isFrozen(loader.getState())).toBe(true);
    await loader.open(P);
    const opened = loader.getState();
    expect(loader.getState()).toBe(opened);
    loader.close();
    expect(loader.getState()).toBe(opened);
    loader.applyColumns(MJD_CHOICE);
    expect(loader.getState()).not.toBe(opened);
    expect(Object.isFrozen(loader.getState())).toBe(true);
  });
});
