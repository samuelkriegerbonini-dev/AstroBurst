import { describe, expect, it } from "vitest";
import {
  MAX_ROWS,
  PAGE_SIZE,
  SORT_BLOCKED_HINT,
  allRowsLoaded,
  mergePage,
  nextOffset,
  nextSort,
  sortMode,
  sortRows,
} from "../logic/paging";
import { COLUMNS, page, row } from "./fixtures";
import type { MastColumn } from "../../../shared/types/mast";

function rows(n: number, from = 0) {
  return Array.from({ length: n }, (_, i) => row(`ds-${String(from + i).padStart(4, "0")}`));
}

const col = (name: string): MastColumn => {
  const c = COLUMNS.find((x) => x.name === name);
  if (!c) throw new Error(name);
  return c;
};

describe("mergePage", () => {
  it("replaces the set on offset 0", () => {
    const first = mergePage(null, page(rows(3)));
    const second = mergePage(first, page(rows(2, 10)));
    expect(second.rows.map((r) => r.datasetId)).toEqual(["ds-0010", "ds-0011"]);
    expect(PAGE_SIZE).toBe(200);
  });

  it("appends the next page and keeps hasMore from MAST", () => {
    const first = mergePage(null, page(rows(200), { hasMore: true, totalReported: 450 }));
    expect(first.hasMore).toBe(true);
    expect(nextOffset(first)).toBe(200);
    const second = mergePage(first, page(rows(200, 200), { offset: 200, hasMore: true, totalReported: 450 }));
    expect(second.rows).toHaveLength(400);
    expect(nextOffset(second)).toBe(400);
    const third = mergePage(second, page(rows(50, 400), { offset: 400, hasMore: false, totalReported: 450 }));
    expect(third.rows).toHaveLength(450);
    expect(third.hasMore).toBe(false);
    expect(nextOffset(third)).toBeNull();
  });

  it("drops rows already loaded when MAST repeats a dataset across pages", () => {
    const first = mergePage(null, page(rows(200), { hasMore: true }));
    const second = mergePage(first, page([...rows(1, 199), ...rows(2, 200)], { offset: 200, hasMore: false }));
    expect(second.rows).toHaveLength(202);
  });

  it("keeps a later page's row whose dataset id repeats with other values, as IUE lists one row per aperture", () => {
    const aperture = (id: string, aper: string) => row(id, { values: { iue_data_id: id, iue_aper: aper } });
    const first = mergePage(null, page([...rows(199), aperture("LWP15521", "LARGE")], { hasMore: true, totalReported: 272 }));
    const second = mergePage(first, page([aperture("LWP15521", "LARGE"), aperture("LWP15521", "SMALL"), aperture("LWP15526", "LARGE")], { offset: 200, hasMore: false }));
    expect(second.rows).toHaveLength(202);
    expect(second.rows.slice(-3).map((r) => `${r.datasetId} ${r.values.iue_aper}`)).toEqual(["LWP15521 LARGE", "LWP15521 SMALL", "LWP15526 LARGE"]);
  });

  it("ignores a page whose offset does not continue the set", () => {
    const first = mergePage(null, page(rows(200), { hasMore: true }));
    expect(mergePage(first, page(rows(200, 600), { offset: 600, hasMore: true }))).toBe(first);
  });

  it("stops at 2,000 rows", () => {
    let set = mergePage(null, page(rows(200), { hasMore: true, totalReported: 5000 }));
    for (let offset = 200; offset < MAX_ROWS; offset += 200) {
      set = mergePage(set, page(rows(200, offset), { offset, hasMore: true, totalReported: 5000 }));
    }
    expect(set.rows).toHaveLength(MAX_ROWS);
    expect(set.hasMore).toBe(false);
    expect(set.truncated).toBe(true);
    expect(nextOffset(set)).toBeNull();
  });
});

describe("sorting rules", () => {
  it("sorts locally once every row is loaded", () => {
    const set = mergePage(null, page(rows(14)));
    expect(allRowsLoaded(set)).toBe(true);
    expect(sortMode(set, col("date_obs"))).toBe("local");
    expect(sortMode(set, col("_prog_obs"))).toBe("local");
  });

  it("offers a server sort on sortable columns while more rows exist, and refuses the rest", () => {
    const set = mergePage(null, page(rows(200), { hasMore: true }));
    expect(sortMode(set, col("date_obs"))).toBe("server");
    expect(sortMode(set, col("_prog_obs"))).toBe("none");
    expect(SORT_BLOCKED_HINT).toBe("MAST cannot sort by this column while more rows exist");
  });

  it("cycles a header through ascending, descending and default, and adds secondary keys with Shift", () => {
    expect(nextSort([], "date_obs", false)).toEqual([{ column: "date_obs", desc: false }]);
    expect(nextSort([{ column: "date_obs", desc: false }], "date_obs", false)).toEqual([{ column: "date_obs", desc: true }]);
    expect(nextSort([{ column: "date_obs", desc: true }], "date_obs", false)).toEqual([]);
    expect(nextSort([{ column: "date_obs", desc: true }], "effexptm", false)).toEqual([{ column: "effexptm", desc: false }]);
    expect(nextSort([{ column: "date_obs", desc: true }], "effexptm", true)).toEqual([
      { column: "date_obs", desc: true },
      { column: "effexptm", desc: false },
    ]);
    expect(nextSort([{ column: "date_obs", desc: true }, { column: "effexptm", desc: true }], "effexptm", true)).toEqual([{ column: "date_obs", desc: true }]);
  });
});

describe("sortRows", () => {
  const data = [
    row("a", { values: { effexptm: 10, date_obs: "2023-04-06T10:12:00", program: 2739, fileSetName: "a", _prog_obs: "2739.13" } }),
    row("b", { values: { effexptm: null, date_obs: "2023-09-04T00:02:00", program: 9, fileSetName: "b", _prog_obs: "2739.11" } }),
    row("c", { values: { effexptm: 2.5, date_obs: null, program: 100, fileSetName: "c", _prog_obs: "2739.12" } }),
    row("d", { values: { effexptm: 10, date_obs: "2023-04-05T07:29:00", program: 2739, fileSetName: "d", _prog_obs: "2739.2" } }),
  ];

  it("compares floats numerically and keeps nulls last in both directions", () => {
    expect(sortRows(data, [{ column: "effexptm", desc: false }], COLUMNS).map((r) => r.datasetId)).toEqual(["c", "a", "d", "b"]);
    expect(sortRows(data, [{ column: "effexptm", desc: true }], COLUMNS).map((r) => r.datasetId)).toEqual(["a", "d", "c", "b"]);
  });

  it("is stable for equal keys", () => {
    expect(sortRows(data, [{ column: "program", desc: true }], COLUMNS).map((r) => r.datasetId)).toEqual(["a", "d", "c", "b"]);
  });

  it("orders datetimes chronologically", () => {
    expect(sortRows(data, [{ column: "date_obs", desc: true }], COLUMNS).map((r) => r.datasetId)).toEqual(["b", "a", "d", "c"]);
  });

  it("orders text naturally, so 2739.2 comes before 2739.11", () => {
    expect(sortRows(data, [{ column: "_prog_obs", desc: false }], COLUMNS).map((r) => r.datasetId)).toEqual(["d", "b", "c", "a"]);
  });

  it("applies a secondary key", () => {
    expect(sortRows(data, [{ column: "program", desc: true }, { column: "date_obs", desc: false }], COLUMNS).map((r) => r.datasetId)).toEqual([
      "d",
      "a",
      "c",
      "b",
    ]);
  });

  it("returns the original order without keys and never mutates the input", () => {
    const copy = [...data];
    expect(sortRows(data, [], COLUMNS)).toEqual(data);
    sortRows(data, [{ column: "effexptm", desc: true }], COLUMNS);
    expect(data).toEqual(copy);
  });
});
