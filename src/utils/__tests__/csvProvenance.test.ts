import { describe, expect, it } from "vitest";
import { buildCsv, CSV_LINE_END, type CsvColumn } from "../catalogCsv";
import { APP_VERSION } from "../constants";
import {
  exportEntries,
  PIXEL_SUM_SUFFIX,
  PROVENANCE_FILE_SEPARATOR,
  provenanceLines,
  summedPixelUnit,
  unitLines,
  withProvenance,
} from "../csvProvenance";

interface SampleRow {
  id: string;
  flux: number | null;
  err: number;
  note: string;
}

const SAMPLE_COLUMNS: CsvColumn<SampleRow>[] = [
  { header: "id", value: (row) => row.id },
  { header: "flux_ujy", value: (row) => row.flux },
  { header: "flux_err_ujy", value: (row) => row.err },
  { header: "note", value: (row) => row.note },
];

const SAMPLE_ROWS: SampleRow[] = [
  { id: "s1", flux: 2.5, err: 0.25, note: "a, b" },
  { id: "s2", flux: null, err: 0.5, note: 'q"x' },
];

const SAMPLE_HEADER_LINES = [
  "# software: AstroBurst v0.8.0",
  "# exported_utc: 2026-11-10T03:04:05.000Z",
  "# file: a.fits; b.fits",
  "# aperture_radius_arcsec: 1.5",
  "# note:",
  "# unit flux_ujy: uJy",
  "# unit flux_err_ujy: uJy",
];

function sampleText(): string {
  const table = buildCsv(SAMPLE_COLUMNS, SAMPLE_ROWS);
  return withProvenance(
    [
      ...provenanceLines([
        ["software", "AstroBurst v0.8.0"],
        ["exported_utc", "2026-11-10T03:04:05.000Z"],
        ["file", "a.fits; b.fits"],
        ["aperture_radius_arcsec", 1.5],
        ["note", null],
      ]),
      ...unitLines({ flux_ujy: "uJy", flux_err_ujy: "uJy" }),
    ],
    table,
    CSV_LINE_END,
  );
}

describe("csvProvenance", () => {
  it("writes the ten-line sample the table reader pins on its side", () => {
    const expected =
      [
        ...SAMPLE_HEADER_LINES,
        "id,flux_ujy,flux_err_ujy,note",
        "s1,2.5,0.25,\"a, b\"",
        "s2,,0.5,\"q\"\"x\"",
      ].join("\r\n") + "\r\n";
    expect(sampleText()).toBe(expected);
  });

  it("prints a number as the shortest text that reads back and nothing for a missing value", () => {
    expect(
      provenanceLines([
        ["a", 1.5],
        ["b", 0.1 + 0.2],
        ["c", 1e-7],
        ["d", 1e21],
        ["e", -0],
        ["f", Number.NaN],
        ["g", Number.POSITIVE_INFINITY],
        ["h", null],
        ["i", undefined],
        ["j", ""],
        ["k", "  x  "],
        ["l", 42],
        ["m", -2.5e-17],
      ]),
    ).toEqual([
      "# a: 1.5",
      "# b: 0.30000000000000004",
      "# c: 1e-7",
      "# d: 1e+21",
      "# e: 0",
      "# f:",
      "# g:",
      "# h:",
      "# i:",
      "# j:",
      "# k: x",
      "# l: 42",
      "# m: -2.5e-17",
    ]);
  });

  it("keeps every entry on one line and every key free of colons", () => {
    const lines = provenanceLines([
      ["note", "line one\r\nline two\n three"],
      ["a: b", "x"],
      ["time: utc\n", "12:00:00"],
      ["", "dropped"],
      [" : ", "dropped"],
      ["  padded  ", "v"],
    ]);
    expect(lines).toEqual(["# note: line one line two three", "# a b: x", "# time utc: 12:00:00", "# padded: v"]);
    for (const line of lines) {
      expect(line).not.toMatch(/[\r\n]/);
    }
  });

  it("writes one unit line for each column that has a unit, in the order given", () => {
    expect(
      unitLines({
        flux: "Jy",
        err: null,
        none: undefined,
        blank: "  ",
        time: "d, BJD_TDB",
        "a:b": "mag",
        " spaced ": " um ",
      }),
    ).toEqual(["# unit flux: Jy", "# unit time: d, BJD_TDB", "# unit a b: mag", "# unit spaced: um"]);
    expect(unitLines({})).toEqual([]);
  });

  it("joins the lines and the table with the line end of the table and adds nothing else", () => {
    const joined = withProvenance(["# a: 1", "# b: 2"], "x,y\n1,2\n", "\n");
    expect(joined).toBe("# a: 1\n# b: 2\nx,y\n1,2\n");
    expect(joined).toBe(["# a: 1", "# b: 2", "x,y\n1,2\n"].join("\n"));
    expect(withProvenance(["# a: 1", "# b: 2"], "x,y\r\n1,2\r\n", "\r\n")).toBe("# a: 1\r\n# b: 2\r\nx,y\r\n1,2\r\n");
    expect(withProvenance([], "x,y\n", "\n")).toBe("x,y\n");
    expect(withProvenance(["# a: 1"], "", "\n")).toBe("# a: 1\n");
  });

  it("starts every export with the program, the time and the file", () => {
    expect(exportEntries("m16.fits", "2026-11-12T12:00:00.000Z")).toEqual([
      ["software", "AstroBurst " + APP_VERSION],
      ["exported_utc", "2026-11-12T12:00:00.000Z"],
      ["file", "m16.fits"],
    ]);
    expect(exportEntries(["g.fits", null, " r.fits ", "g.fits", "", undefined], "T")[2]).toEqual([
      "file",
      "g.fits; r.fits",
    ]);
    expect(exportEntries(null, "T")[2]).toEqual(["file", null]);
    expect(exportEntries([], "T")[2]).toEqual(["file", null]);
    expect(exportEntries("  ", "T")[2]).toEqual(["file", null]);
    expect(exportEntries("a.fits")[1][1]).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
    expect(provenanceLines(exportEntries(null, "T"))).toEqual([
      "# software: AstroBurst " + APP_VERSION,
      "# exported_utc: T",
      "# file:",
    ]);
  });

  it("names the unit of a sum over pixels", () => {
    const cases: [string | null | undefined, string | null][] = [
      ["MJy/sr", "MJy/sr x pix"],
      [" mJy/arcsec2 ", "mJy/arcsec2 x pix"],
      ["mJy/arcsec^2", "mJy/arcsec^2 x pix"],
      ["Jy/beam", "Jy/beam x pix"],
      ["MJy / sr", "MJy / sr x pix"],
      ["MJY/SR", "MJY/SR x pix"],
      ["ADU", "ADU"],
      ["DN/s", "DN/s"],
      ["Jy", "Jy"],
      ["electron", "electron"],
      ["nanomaggy", "nanomaggy"],
      ["MJy sr-1", "MJy sr-1"],
      ["sr", "sr"],
      ["", null],
      ["   ", null],
      [null, null],
      [undefined, null],
    ];
    for (const [input, output] of cases) {
      expect(summedPixelUnit(input)).toBe(output);
    }
    expect(PIXEL_SUM_SUFFIX).toBe(" x pix");
    expect(PROVENANCE_FILE_SEPARATOR).toBe("; ");
  });

  it("every line splits back into its key and value the way the reader splits it", () => {
    const headerLines = sampleText().split("\r\n").slice(0, 7);
    expect(headerLines).toEqual(SAMPLE_HEADER_LINES);
    const pairs = headerLines.map((line) => {
      const content = line.replace(/^#+/, "");
      const colon = content.indexOf(":");
      return [content.slice(0, colon).trim(), content.slice(colon + 1).trim()];
    });
    expect(pairs).toEqual([
      ["software", "AstroBurst v0.8.0"],
      ["exported_utc", "2026-11-10T03:04:05.000Z"],
      ["file", "a.fits; b.fits"],
      ["aperture_radius_arcsec", "1.5"],
      ["note", ""],
      ["unit flux_ujy", "uJy"],
      ["unit flux_err_ujy", "uJy"],
    ]);
    for (const line of headerLines) {
      expect(line.startsWith("# ")).toBe(true);
    }
  });
});
