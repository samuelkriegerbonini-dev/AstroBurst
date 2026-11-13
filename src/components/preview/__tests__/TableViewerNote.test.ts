import { describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import TableViewerNote from "../TableViewerNote";
import { FITS_TABLE_NOTE_HINT, tableNoteOf, type TableNote } from "../../../utils/fileKinds";
import type { TableProbe } from "../../../shared/types/tables";

const NAME = "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv";

const WHTLT: TableProbe = {
  format: "ecsv",
  n_rows: 240,
  columns: ["MJD_UTC_NRS1", "BJD_TDB_NRS1", "whitelight_flux_NRS1"].map((name) => ({ name, unit: null, datatype: "float64", numeric: true })),
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "light_curve", reason: "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1" },
  notes: [],
};

const FITSCAT: TableProbe = {
  format: "fits_bintable",
  n_rows: 500,
  columns: Array.from({ length: 8 }, (_, i) => ({ name: `c${i}`, unit: null, datatype: "float64", numeric: true })),
  tables: [{ hdu: 1, extname: "CATALOG", n_rows: 500, n_columns: 8 }],
  meta: null,
  comments: [],
  guess: { kind: "catalogue", reason: "sky position columns RA and DEC" },
  notes: [],
};

function noteOf(table: TableProbe, name = NAME): TableNote {
  const note = tableNoteOf({ id: "t", name, status: "table", table });
  if (!note) throw new Error("no note for a table row");
  return note;
}

function findButton(node: ReactNode): { onClick: () => void; children: ReactNode } | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findButton(child);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Record<string, unknown>>(node)) return null;
  if (node.type === "button") return node.props as unknown as { onClick: () => void; children: ReactNode };
  return findButton(node.props.children as ReactNode);
}

describe("TableViewerNote", () => {
  it("names the table, what it holds and why, and shows no image", () => {
    const html = renderToStaticMarkup(createElement(TableViewerNote, { note: noteOf(WHTLT), onAction: () => {} }));
    expect(html).toContain("data-table-note");
    expect(html).toContain(`>${NAME} is a table<`);
    expect(html).toContain(">Table, 240 rows x 3 columns (ECSV)<");
    expect(html).toContain(">time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1<");
    expect(html).toContain(">It has no image to show here.<");
    expect(html).toContain(">Open as light curve</button>");
    expect(html).toContain('title="Open this table as a light curve in the Series section of Photometry"');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<canvas");

    const fits = renderToStaticMarkup(createElement(TableViewerNote, { note: noteOf(FITSCAT, "cat.fits"), onAction: () => {} }));
    expect(fits).toContain(">cat.fits is a table<");
    expect(fits).toContain(">Table, 500 rows x 8 columns (FITS table)<");
    expect(fits).toContain(`>${FITS_TABLE_NOTE_HINT}<`);
    expect(fits).not.toContain("<button");

    const unexplained = renderToStaticMarkup(createElement(TableViewerNote, { note: noteOf({ ...WHTLT, guess: { kind: "unknown", reason: "" } }) }));
    expect(unexplained.match(/<span/g)).toHaveLength(3);
    expect(unexplained).not.toContain("<button");
  });

  it("hands the table to its panel from the button", () => {
    const spy = vi.fn();
    const note = noteOf(WHTLT);
    const button = findButton(TableViewerNote({ note, onAction: spy }));
    expect(button?.children).toBe("Open as light curve");
    button?.onClick();
    expect(spy.mock.calls).toEqual([["t", "light_curve"]]);
    expect(findButton(TableViewerNote({ note }))).toBeNull();
    expect(renderToStaticMarkup(createElement(TableViewerNote, { note }))).not.toContain("<button");
  });
});
