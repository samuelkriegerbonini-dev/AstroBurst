import { describe, expect, it } from "vitest";
import { fallbackSelectionId, isSelectableStatus, nextSelectableIndex, selectableRows } from "../fileSelection";

function row(id: string, status: string) {
  return { id, status };
}

const MIXED = [
  row("q", "queued"),
  row("e", "error"),
  row("s", "spectrum"),
  row("p", "processing"),
  row("d", "done"),
  row("e2", "error"),
];

describe("isSelectableStatus", () => {
  it("selects done and spectrum rows only", () => {
    expect(["queued", "processing", "done", "error", "spectrum"].filter(isSelectableStatus)).toEqual(["done", "spectrum"]);
  });
});

describe("selectableRows", () => {
  it("keeps image and spectrum rows in list order for the palette", () => {
    expect(selectableRows(MIXED).map((r) => r.id)).toEqual(["s", "d"]);
  });
});

describe("fallbackSelectionId", () => {
  it("keeps the selection while no filter is active or while the selected row is still listed", () => {
    expect(fallbackSelectionId(MIXED, "x", false)).toBe("x");
    expect(fallbackSelectionId(MIXED, "d", true)).toBe("d");
    expect(fallbackSelectionId(MIXED, null, true)).toBeNull();
  });

  it("falls back to the first selectable row, a spectrum included, when a filter hides the selection", () => {
    expect(fallbackSelectionId(MIXED, "hidden", true)).toBe("s");
    expect(fallbackSelectionId([row("e", "error"), row("d", "done")], "hidden", true)).toBe("d");
  });

  it("selects nothing when the filtered list holds no selectable row", () => {
    expect(fallbackSelectionId([row("q", "queued"), row("e", "error")], "hidden", true)).toBeNull();
  });
});

describe("nextSelectableIndex", () => {
  it("steps over queued, processing and error rows onto spectrum and image rows", () => {
    expect(nextSelectableIndex(MIXED, -1, 1)).toBe(2);
    expect(nextSelectableIndex(MIXED, 2, 1)).toBe(4);
    expect(nextSelectableIndex(MIXED, 4, -1)).toBe(2);
    expect(nextSelectableIndex(MIXED, -1, -1)).toBe(4);
  });

  it("stays put past either end", () => {
    expect(nextSelectableIndex(MIXED, 4, 1)).toBeNull();
    expect(nextSelectableIndex(MIXED, 2, -1)).toBeNull();
    expect(nextSelectableIndex([], -1, 1)).toBeNull();
  });
});
