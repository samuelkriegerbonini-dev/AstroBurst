import { describe, expect, it } from "vitest";
import { galleryColumns, galleryDemand, galleryMove, tableMove } from "../logic/gridNav";

const key = (k: string, ctrl = false) => ({ key: k, ctrlKey: ctrl, metaKey: false });

describe("galleryColumns", () => {
  it("fits 152 px cards with a 12 px gap and never returns fewer than one column", () => {
    expect(galleryColumns(800, 152, 12)).toBe(4);
    expect(galleryColumns(152, 152, 12)).toBe(1);
    expect(galleryColumns(20, 152, 12)).toBe(1);
    expect(galleryColumns(164 * 5 + 152 - 164, 152, 12)).toBe(5);
  });
});

describe("galleryDemand", () => {
  const rows = Array.from({ length: 11 }, (_, i) => ({ datasetId: `d${i}` }));

  it("names the cards on screen, the next page and the width of the first visible row", () => {
    expect(galleryDemand(rows, { columns: 3, firstVisible: 1, lastVisible: 3, pageRows: 1 }, "d4")).toEqual({
      selected: ["d4"],
      visible: ["d3", "d4", "d5", "d6", "d7", "d8"],
      ahead: ["d9", "d10"],
      firstRow: 3,
    });
  });

  it("drops a selection that is not among the rows", () => {
    expect(galleryDemand(rows, { columns: 4, firstVisible: 0, lastVisible: 1, pageRows: 1 }, "gone").selected).toEqual([]);
    expect(galleryDemand(rows, { columns: 4, firstVisible: 0, lastVisible: 1, pageRows: 1 }, null).selected).toEqual([]);
  });
});

describe("galleryMove", () => {
  const grid = { count: 10, columns: 4, pageRows: 2 };

  it("moves card by card with Left and Right, clamped at the ends", () => {
    expect(galleryMove(0, key("ArrowRight"), grid)).toEqual({ index: 1, atEnd: false });
    expect(galleryMove(4, key("ArrowLeft"), grid)).toEqual({ index: 3, atEnd: false });
    expect(galleryMove(0, key("ArrowLeft"), grid)).toEqual({ index: 0, atEnd: false });
    expect(galleryMove(9, key("ArrowRight"), grid)).toEqual({ index: 9, atEnd: true });
  });

  it("moves by rows with Up and Down, landing on the last card of a shorter last row", () => {
    expect(galleryMove(1, key("ArrowDown"), grid)).toEqual({ index: 5, atEnd: false });
    expect(galleryMove(5, key("ArrowDown"), grid)).toEqual({ index: 9, atEnd: true });
    expect(galleryMove(7, key("ArrowDown"), grid)).toEqual({ index: 9, atEnd: true });
    expect(galleryMove(9, key("ArrowDown"), grid)).toEqual({ index: 9, atEnd: true });
    expect(galleryMove(5, key("ArrowUp"), grid)).toEqual({ index: 1, atEnd: false });
    expect(galleryMove(2, key("ArrowUp"), grid)).toEqual({ index: 2, atEnd: false });
  });

  it("goes to the first or last card of the row with Home and End, and of the set with Ctrl", () => {
    expect(galleryMove(6, key("Home"), grid)).toEqual({ index: 4, atEnd: false });
    expect(galleryMove(5, key("End"), grid)).toEqual({ index: 7, atEnd: false });
    expect(galleryMove(8, key("End"), grid)).toEqual({ index: 9, atEnd: true });
    expect(galleryMove(6, key("Home", true), grid)).toEqual({ index: 0, atEnd: false });
    expect(galleryMove(1, key("End", true), grid)).toEqual({ index: 9, atEnd: true });
  });

  it("moves one screen with PageUp and PageDown", () => {
    expect(galleryMove(1, key("PageDown"), { count: 40, columns: 4, pageRows: 3 })).toEqual({ index: 13, atEnd: false });
    expect(galleryMove(13, key("PageUp"), { count: 40, columns: 4, pageRows: 3 })).toEqual({ index: 1, atEnd: false });
    expect(galleryMove(30, key("PageDown"), { count: 40, columns: 4, pageRows: 3 })).toEqual({ index: 39, atEnd: true });
  });

  it("follows a column count change", () => {
    expect(galleryMove(1, key("ArrowDown"), { count: 10, columns: 2, pageRows: 2 })).toEqual({ index: 3, atEnd: false });
  });

  it("ignores other keys and empty sets", () => {
    expect(galleryMove(3, key("a"), grid)).toBeNull();
    expect(galleryMove(0, key("ArrowRight"), { count: 0, columns: 4, pageRows: 2 })).toBeNull();
  });
});

describe("tableMove", () => {
  const table = { count: 30, pageRows: 10 };

  it("moves the focused row with Up and Down", () => {
    expect(tableMove(0, key("ArrowDown"), table)).toEqual({ index: 1, atEnd: false });
    expect(tableMove(0, key("ArrowUp"), table)).toEqual({ index: 0, atEnd: false });
    expect(tableMove(28, key("ArrowDown"), table)).toEqual({ index: 29, atEnd: true });
  });

  it("jumps with Home, End and the Ctrl variants", () => {
    expect(tableMove(12, key("Home"), table)).toEqual({ index: 0, atEnd: false });
    expect(tableMove(12, key("End"), table)).toEqual({ index: 29, atEnd: true });
    expect(tableMove(12, key("End", true), table)).toEqual({ index: 29, atEnd: true });
  });

  it("pages by the visible row count and signals the last row", () => {
    expect(tableMove(5, key("PageDown"), table)).toEqual({ index: 15, atEnd: false });
    expect(tableMove(25, key("PageDown"), table)).toEqual({ index: 29, atEnd: true });
    expect(tableMove(5, key("PageUp"), table)).toEqual({ index: 0, atEnd: false });
  });

  it("leaves Left and Right to the cells", () => {
    expect(tableMove(5, key("ArrowLeft"), table)).toBeNull();
    expect(tableMove(5, key("ArrowRight"), table)).toBeNull();
  });
});
