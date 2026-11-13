import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LightCurveTableState, TableColumnsChoice } from "../../../hooks/useLightCurveTable";
import type { TableResult, TableResultColumn } from "../../../shared/types/tables";
import type { LightCurveDoc } from "../../../shared/types/lightCurve";

const holder = vi.hoisted(() => ({
  state: null as unknown,
  applyColumns: vi.fn(),
  automatic: vi.fn(),
  close: vi.fn(),
}));

vi.mock("react", async (io) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await io()));
vi.mock("../../../hooks/useLightCurveTable", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useLightCurveTable")>()),
  useLightCurveTableState: () => holder.state,
  lightCurveTableLoader: { applyColumns: holder.applyColumns, automatic: holder.automatic, close: holder.close },
}));

import { hooks } from "../../../test/hookRuntime";
import { elementsOf, textOf } from "../../../test/elementTree";
import { lightCurveStore } from "../../../hooks/useLightCurveStore";
import { TIME_SCALE_NAMES, lightCurveDocFromTable } from "../../../utils/lightCurveDoc";
import { NO_TIME_COLUMN_NOTE, lightCurveDocsFromTable, planLightCurveTable } from "../../../utils/lightCurveTable";
import { ErrorAlert } from "../../ui";

const { default: LightCurveTablePanel } = await import("../LightCurveTablePanel");

type Props = Record<string, unknown>;

const P = "C:/d/jw02358_whtlt.ecsv";
const G = "table:C:/d/jw02358_whtlt.ecsv";

function num(name: string, values: (number | null)[], unit: string | null = null): TableResultColumn {
  return { name, unit, datatype: "float64", values, text: null };
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

const PLAN = planLightCurveTable(WHTLT);

function loaded(extra: Partial<LightCurveTableState> = {}): LightCurveTableState {
  return { path: P, loading: false, error: null, table: WHTLT, plan: PLAN, manual: null, ...extra };
}

const panel = (LightCurveTablePanel as unknown as { type: () => unknown }).type;
const show = () => hooks.render(panel, undefined as never);

const all = (tree: unknown) => elementsOf<Props>(tree);
const propsOf = (tree: unknown) => all(tree).map((e) => e.props);

function selectLabelled(tree: unknown, label: string): Props | undefined {
  const caption = propsOf(tree).find((p) => typeof p.htmlFor === "string" && textOf(p.children).trim() === label);
  return all(tree).find((e) => e.type === "select" && e.props.id === caption?.htmlFor)?.props;
}

function optionTexts(select: Props | undefined): string[] {
  return elementsOf<Props>(select?.children)
    .filter((e) => e.type === "option")
    .map((e) => textOf(e.props.children));
}

function buttonNamed(tree: unknown, name: string): Props | undefined {
  return all(tree).find((e) => e.type === "button" && textOf(e.props.children).trim() === name)?.props;
}

const SELECT_LABELS = ["time column", "time scale", "value column", "error column", "quantity"];

describe("LightCurveTablePanel", () => {
  beforeEach(() => {
    hooks.mount();
    holder.state = loaded();
    holder.applyColumns.mockReset();
    holder.automatic.mockReset();
    holder.close.mockReset();
    lightCurveStore.put(G, lightCurveDocsFromTable(WHTLT, P, PLAN));
    lightCurveStore.setActiveTable(G);
  });

  afterEach(() => {
    hooks.mount();
    lightCurveStore.setActiveTable(null);
    lightCurveStore.remove(G);
  });

  it("shows the file, the column selects with the plan's choice and the view of the table's group", () => {
    const tree = show();
    expect(textOf(tree)).toContain("Light curve table");
    expect(textOf(tree)).toContain("jw02358_whtlt.ecsv");
    expect(propsOf(tree).some((p) => p.title === P && textOf(p.children) === "jw02358_whtlt.ecsv")).toBe(true);
    expect(SELECT_LABELS.map((label) => selectLabelled(tree, label)?.value)).toEqual([
      "BJD_TDB_NRS1",
      "bmjd_tdb",
      "whitelight_flux_NRS1",
      "",
      "flux",
    ]);
    expect(optionTexts(selectLabelled(tree, "time scale"))).toEqual(Object.values(TIME_SCALE_NAMES));
    expect(optionTexts(selectLabelled(tree, "time column"))).toEqual(PLAN.numericColumns);
    expect(optionTexts(selectLabelled(tree, "error column"))).toEqual(["none", ...PLAN.numericColumns]);
    expect(optionTexts(selectLabelled(tree, "quantity"))).toEqual(["magnitude", "flux", "relative flux"]);
    const views = propsOf(tree).filter((p) => p.group === G);
    expect(views).toHaveLength(1);
    expect(views[0].csvName).toBe("jw02358_whtlt");
    expect(all(tree)[0].props["data-light-curve-table"]).toBeDefined();
  });

  it("Use these columns sends the five choices and Automatic columns appears only for a manual choice", () => {
    const tree = show();
    expect(buttonNamed(tree, "Automatic columns")).toBeUndefined();
    (selectLabelled(tree, "time column")!.onChange as (e: unknown) => void)({ target: { value: "MJD_UTC_NRS1" } });
    (selectLabelled(tree, "time scale")!.onChange as (e: unknown) => void)({ target: { value: "mjd_utc" } });
    const changed = show();
    expect(selectLabelled(changed, "time column")?.value).toBe("MJD_UTC_NRS1");
    (buttonNamed(changed, "Use these columns")!.onClick as () => void)();
    const expected: TableColumnsChoice = {
      timeColumn: "MJD_UTC_NRS1",
      timeScale: "mjd_utc",
      valueColumn: "whitelight_flux_NRS1",
      quantity: "flux",
      errColumn: null,
    };
    expect(holder.applyColumns).toHaveBeenCalledWith(expected);
    holder.state = loaded({ manual: expected });
    const manual = show();
    expect(selectLabelled(manual, "time scale")?.value).toBe("mjd_utc");
    (buttonNamed(manual, "Automatic columns")!.onClick as () => void)();
    expect(holder.automatic).toHaveBeenCalledTimes(1);
  });

  it("Close table calls close, and the loading and error states are shown", () => {
    (buttonNamed(show(), "Close table")!.onClick as () => void)();
    expect(holder.close).toHaveBeenCalledTimes(1);
    holder.state = { path: P, loading: true, error: null, table: null, plan: null, manual: null };
    const reading = show();
    expect(textOf(reading)).toContain("Reading the table...");
    expect(all(reading).some((e) => e.type === "select")).toBe(false);
    holder.state = loaded({ error: NO_TIME_COLUMN_NOTE });
    const noted = show();
    const alerts = all(noted).filter((e) => e.type === ErrorAlert);
    expect(alerts.map((e) => e.props.message)).toEqual([NO_TIME_COLUMN_NOTE]);
    expect(SELECT_LABELS.every((label) => selectLabelled(noted, label) !== undefined)).toBe(true);
  });

  it("the notes of the documents are listed once", () => {
    const kept = "2 of 4 rows kept (2 without a finite time or value)";
    const bmjd = "time column BJD_TDB_NRS1: values below 2400000.5: read as BMJD (TDB) = BJD_TDB - 2400000.5";
    const doc: LightCurveDoc = {
      ...lightCurveDocFromTable(WHTLT, { path: P, timeColumn: "BJD_TDB_NRS1", timeScale: "bmjd_tdb", valueColumn: "whitelight_flux_NRS1", quantity: "flux" }),
      notes: [kept, bmjd],
    };
    lightCurveStore.put(G, [doc, { ...doc, id: "second" }]);
    holder.state = loaded({ table: { ...WHTLT, notes: [kept] } });
    const tree = show();
    const details = all(tree).find((e) => e.type === "details");
    expect(textOf(details?.props.children)).toContain("Table notes");
    const items = elementsOf<Props>(details?.props.children).filter((e) => e.type === "li").map((e) => textOf(e.props.children));
    expect(items).toEqual([kept, bmjd]);
    expect(details?.props.open).toBeUndefined();
    lightCurveStore.put(G, [{ ...doc, notes: [] }]);
    holder.state = loaded();
    expect(all(show()).some((e) => e.type === "details")).toBe(false);
  });
});
