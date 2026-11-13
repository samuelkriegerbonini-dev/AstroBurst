import { describe, expect, it, vi } from "vitest";
import { TableRouteStore, type TableRouteSource } from "../useTableRouteStore";
import type { TableProbe } from "../../shared/types/tables";

function probe(kind: TableProbe["guess"]["kind"]): TableProbe {
  return { format: "csv", n_rows: 2, columns: [], tables: null, meta: null, comments: [], guess: { kind, reason: "" }, notes: [] };
}

const A: TableRouteSource = { path: "C:/d/whtlt.ecsv", name: "whtlt.ecsv", probe: probe("light_curve") };
const C: TableRouteSource = { path: "C:/d/C1.csv", name: "C1.csv", probe: probe("catalogue") };

describe("TableRouteStore", () => {
  it("keeps one route per kind and numbers the requests", () => {
    const store = new TableRouteStore();
    const first = store.request("light_curve", A);
    expect(first).toEqual({ ...A, kind: "light_curve", seq: 1 });
    expect(store.get("light_curve")).toBe(first);
    expect(store.get("light_curve")).toBe(first);
    expect(store.get("catalogue")).toBeNull();
    const catalogue = store.request("catalogue", C);
    expect(catalogue).toEqual({ ...C, kind: "catalogue", seq: 2 });
    expect(store.get("catalogue")).toBe(catalogue);
    expect(store.get("light_curve")).toBe(first);
    const again = store.request("light_curve", A);
    expect(again.seq).toBe(3);
    expect(again).not.toBe(first);
    expect(store.get("light_curve")).toBe(again);
  });

  it("notifies on a request, and on a clear or a reset only when a route was removed", () => {
    const store = new TableRouteStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);
    store.request("light_curve", A);
    expect(listener).toHaveBeenCalledTimes(1);
    store.clear("catalogue");
    expect(listener).toHaveBeenCalledTimes(1);
    store.clear("light_curve");
    expect(listener).toHaveBeenCalledTimes(2);
    expect(store.get("light_curve")).toBeNull();
    store.reset();
    expect(listener).toHaveBeenCalledTimes(2);
    store.request("light_curve", A);
    const last = store.request("catalogue", C);
    expect(listener).toHaveBeenCalledTimes(4);
    store.reset();
    expect(listener).toHaveBeenCalledTimes(5);
    expect(store.get("light_curve")).toBeNull();
    expect(store.get("catalogue")).toBeNull();
    expect(store.request("light_curve", A).seq).toBe(last.seq + 1);
    expect(listener).toHaveBeenCalledTimes(6);
    unsubscribe();
    store.request("catalogue", C);
    store.reset();
    expect(listener).toHaveBeenCalledTimes(6);
  });
});
