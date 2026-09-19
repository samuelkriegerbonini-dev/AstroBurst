import { describe, it, expect } from "vitest";
import { parseDefectList, formatDefectError } from "../defectList";

describe("parseDefectList", () => {
  it("accepts the three PixInsight forms, comments and blank lines", () => {
    const text = "# hot pixels\nPoint 10 20\ncol 5\nCOL 7 2 9\nRow 3\n\nrow 4 1 6 // trailing note\n// end\n";
    const parsed = parseDefectList(text);
    expect(parsed.errors).toEqual([]);
    expect(parsed.defects).toEqual([
      { kind: "point", x: 10, y: 20 },
      { kind: "column", x: 5, y0: null, y1: null },
      { kind: "column", x: 7, y0: 2, y1: 9 },
      { kind: "row", y: 3, x0: null, x1: null },
      { kind: "row", y: 4, x0: 1, x1: 6 },
    ]);
    expect(parseDefectList("")).toEqual({ defects: [], errors: [] });
    expect(parseDefectList("line one\r\nPoint 1 1").defects).toEqual([{ kind: "point", x: 1, y: 1 }]);
  });

  it("reports every bad line with its 1-based line number", () => {
    const parsed = parseDefectList("Point 1 2\n\nCol x\nFoo 1 2\nPoint 1\nRow 1 5 2\nPoint -1 0");
    expect(parsed.defects).toEqual([{ kind: "point", x: 1, y: 2 }]);
    expect(parsed.errors.map((e) => e.line)).toEqual([3, 4, 5, 6, 7]);
    expect(parsed.errors[1].message).toContain("Foo");
    expect(formatDefectError(parsed.errors[0])).toMatch(/^Line 3: /);
  });

  it("keeps coordinates 0-based and unshifted", () => {
    const parsed = parseDefectList("Point 0 0\nCol 0 0 0\nRow 0");
    expect(parsed.errors).toEqual([]);
    expect(parsed.defects).toEqual([
      { kind: "point", x: 0, y: 0 },
      { kind: "column", x: 0, y0: 0, y1: 0 },
      { kind: "row", y: 0, x0: null, x1: null },
    ]);
  });
});
