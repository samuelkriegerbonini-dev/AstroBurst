import { afterEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createElement, isValidElement, type ComponentProps, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));

import { hooks } from "../../../test/hookRuntime";
import MetadataFileList, { type MetadataFile } from "../MetadataFileList";
import { rowTops, rowWindow } from "../../../utils/fileListRows";
import { tableRowTitle } from "../../../utils/fileKinds";
import type { Spectrum1dKind } from "../../../shared/types/spectrum1d";
import type { TableProbe } from "../../../shared/types/tables";

type ListProps = ComponentProps<typeof MetadataFileList>;
type AnyProps = Record<string, unknown> & { children?: ReactNode };

const listView = (MetadataFileList as unknown as { type: (props: ListProps) => ReactNode }).type;

function elements(node: ReactNode): ReactElement<AnyProps>[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!isValidElement<AnyProps>(node)) return [];
  return [node, ...elements(node.props.children)];
}

function buttonTag(html: string, text: string): string {
  const tag = new RegExp(`(<button[^>]*>)${text}</button>`).exec(html)?.[1];
  if (!tag) throw new Error(`no button reading ${text}`);
  return tag;
}

function summaryText(html: string): string {
  return (/<summary[^>]*>(.*?)<\/summary>/.exec(html)?.[1] ?? "").replace(/<[^>]*>/g, "");
}

function doneFile(id: string, name: string, path = `C:/data/${name}`): MetadataFile {
  return { id, name, path, size: 1, status: "done" };
}

function renderList(files: MetadataFile[]): string {
  return renderToStaticMarkup(createElement(MetadataFileList, { files, selectedId: null, onSelect: () => {} }));
}

interface ShownRow {
  title: string;
  text: string;
  style: string | null;
  isolated: boolean;
}

function shownRows(files: MetadataFile[]): ShownRow[] {
  const pattern = /<span class="ab-mfl-filename" title="([^"]*)"(?: style="([^"]*)")?>(<bdi>)?([^<]*)(?:<\/bdi>)?<\/span>/g;
  return [...renderList(files).matchAll(pattern)].map(([, title, style, bdi, text]) => ({
    title,
    text,
    style: style ?? null,
    isolated: bdi !== undefined,
  }));
}

function shownNames(names: string[]): { title: string; text: string }[] {
  return shownRows(names.map((name, i) => doneFile(`f${i}`, name))).map(({ title, text }) => ({ title, text }));
}

describe("MetadataFileList file names", () => {
  it("shows clear-f444w and f444w-f470n as different rows and keeps the full path in the title", () => {
    const clearF444w = "jw02739-o001_t001_nircam_clear-f444w_i2d.fits";
    const f444wF470n = "jw02739-o001_t001_nircam_f444w-f470n_i2d.fits";
    expect(shownNames([clearF444w, f444wF470n])).toEqual([
      { title: `C:/data/${clearF444w}`, text: "...clear-f444w_i2d" },
      { title: `C:/data/${f444wF470n}`, text: "...f444w-f470n_i2d" },
    ]);
  });

  it("names each row against the whole list, so rows that share the short form get their differing token", () => {
    const names = [
      "jw06565-o002_t001_nircam_clear-f090w_i2d.fits",
      "jw06565-o003_t001_nircam_clear-f090w_i2d.fits",
    ];
    expect(shownNames(names)).toEqual([
      { title: `C:/data/${names[0]}`, text: "o002...clear-f090w_i2d" },
      { title: `C:/data/${names[1]}`, text: "o003...clear-f090w_i2d" },
    ]);
  });

  it("tells same-named files from two folders apart by the folder and titles them with the full path", () => {
    const rows = shownRows([
      doneFile("ha", "master_flat.fits", "C:\\astro\\b16\\Ha\\master_flat.fits"),
      doneFile("oiii", "master_flat.fits", "C:\\astro\\b16\\OIII\\master_flat.fits"),
    ]);
    expect(rows.map(({ title, text }) => ({ title, text }))).toEqual([
      { title: "C:\\astro\\b16\\Ha\\master_flat.fits", text: "Ha/master_flat" },
      { title: "C:\\astro\\b16\\OIII\\master_flat.fits", text: "OIII/master_flat" },
    ]);
  });

  it("starts the ASIAIR light label with the dropped-part marker", () => {
    const [row] = shownNames(["Light_M42_120.0s_Bin1_294MC_gain100_20231010-203040_-10.0C_0001.fits"]);
    expect(row.text.startsWith("...")).toBe(true);
  });

  it("clips an unshortened name at its start inside an isolated run and leaves shortened names alone", () => {
    const rows = shownRows([doneFile("a", "M42_LRGB_final.fits"), doneFile("b", "jw02739-o001_t001_nircam_clear-f444w_i2d.fits")]);
    expect(rows).toEqual([
      { title: "C:/data/M42_LRGB_final.fits", text: "M42_LRGB_final", style: "direction:rtl;text-align:left", isolated: true },
      { title: "C:/data/jw02739-o001_t001_nircam_clear-f444w_i2d.fits", text: "...clear-f444w_i2d", style: null, isolated: true },
    ]);
  });
});

describe("MetadataFileList filter chips", () => {
  it("places search before the collapsed filter controls and retains every product and custom filter", () => {
    const html = renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE", "S3D"], customChips: ["NIRSPEC"] });
    expect(html.indexOf('aria-label="Search files, filters, instruments"')).toBeLessThan(html.indexOf('<details class="ab-mfl-filters"'));
    expect(html).toContain('<details class="ab-mfl-filters">');
    expect(html).toContain(">Filters</span>");
    for (const label of ["CAL", "RATE", "S3D", "NIRSPEC"]) expect(html).toContain(`>${label}</button>`);
    expect(html).toContain('title="Remove &quot;NIRSPEC&quot; filter"');
  });

  it("keeps the active filter count and match mode visible when filter controls are collapsed", () => {
    const html = renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE", "S3D"], activeFilters: ["CAL", "RATE"], filterMode: "and" });
    expect(summaryText(html)).toContain("2 active");
    expect(summaryText(html)).toContain("Match all");
    expect(html).toContain('aria-label="Clear all filters"');
  });

  it("marks only the active product chips as pressed", () => {
    const html = renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE", "S3D"], activeFilters: ["CAL", "RATE"], filterMode: "and" });
    expect(buttonTag(html, "CAL")).toContain('aria-pressed="true"');
    expect(buttonTag(html, "RATE")).toContain('aria-pressed="true"');
    expect(buttonTag(html, "S3D")).toContain('aria-pressed="false"');
  });

  it("reports All files while no filter is active and Match any in or mode", () => {
    const idle = renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE"] });
    expect(summaryText(idle)).toContain("All files");
    expect(summaryText(idle)).not.toContain("active");
    expect(summaryText(idle)).not.toContain("Match");
    const any = renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE"], activeFilters: ["CAL", "RATE"], filterMode: "or" });
    expect(summaryText(any)).toContain("2 active · Match any");
  });

  it("names the remove button of a pinned chip after the chip", () => {
    const html = renderRow(doneFile("f", "image.fits"), { customChips: ["NIRSPEC"] });
    expect(html).toContain('aria-label="Remove &quot;NIRSPEC&quot; filter"');
  });

  it("offers the match mode as an AND and OR pair with aria-pressed", () => {
    const and = renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE"], activeFilters: ["CAL", "RATE"], filterMode: "and" });
    expect(and).toMatch(/<div role="group" aria-label="Match mode" class="ab-mfl-mode-toggle">/);
    expect(buttonTag(and, "AND")).toContain('aria-pressed="true"');
    expect(buttonTag(and, "OR")).toContain('aria-pressed="false"');
    const or = renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE"], activeFilters: ["CAL", "RATE"], filterMode: "or" });
    expect(buttonTag(or, "AND")).toContain('aria-pressed="false"');
    expect(buttonTag(or, "OR")).toContain('aria-pressed="true"');
    expect(renderRow(doneFile("f", "image.fits"), { productTypes: ["CAL", "RATE"], activeFilters: ["CAL"] })).not.toContain("Match mode");
  });

  it("titles the panel Files with an h2", () => {
    expect(renderList([doneFile("f", "image.fits")])).toContain('<h2 class="ab-mfl-title">Files</h2>');
  });

  it("keeps active filtering discoverable even when no product or custom choices remain", () => {
    const html = renderRow(doneFile("f", "image.fits"), { activeFilters: ["CAL"] });
    expect(html).toContain('<details class="ab-mfl-filters">');
    expect(html).toContain("1 active");
    expect(html).toContain('title="Clear all filters"');
  });

  it("draws a filter outside the colour table with the muted text token instead of a grey literal", () => {
    const html = renderList([
      { ...doneFile("ha", "ha.fits"), metadata: { filter: "Ha" } },
      { ...doneFile("f444w", "f444w.fits"), metadata: { filter: "F444W" } },
    ]);
    expect(html).not.toContain("#71717a");
    expect(html).toContain(
      'style="color:var(--ab-text-3);background:color-mix(in srgb, var(--ab-text-3) 8%, transparent);border-color:color-mix(in srgb, var(--ab-text-3) 19%, transparent)"',
    );
    expect(html).toContain(
      'style="color:#991b1b;background:color-mix(in srgb, #991b1b 8%, transparent);border-color:color-mix(in srgb, #991b1b 19%, transparent)"',
    );
  });
});

describe("MetadataFileList pinned search", () => {
  afterEach(() => hooks.mount());

  function view(props: Partial<ListProps>): ReactElement<AnyProps>[] {
    return elements(hooks.render(listView, { files: [doneFile("f", "image.fits")], selectedId: null, onSelect: () => {}, productTypes: ["CAL"], ...props }) as ReactNode);
  }

  function details(tree: ReactElement<AnyProps>[]): ReactElement<AnyProps> | undefined {
    return tree.find((el) => el.type === "details");
  }

  function pinWithEnter(text: string, props: Partial<ListProps>): void {
    const input = view(props).find((el) => el.type === "input");
    (input?.props.onChange as (e: { target: { value: string } }) => void)({ target: { value: text } });
    const typed = view(props).find((el) => el.type === "input");
    (typed?.props.onKeyDown as (e: { key: string; preventDefault: () => void }) => void)({ key: "Enter", preventDefault: () => {} });
  }

  it("opens the Filters block when a search is pinned, so the new chip, its remove button and Clear are in view", () => {
    const pinned: string[] = [];
    const onAddCustomChip = (text: string) => void pinned.push(text);
    expect(details(view({ onAddCustomChip }))).toBeUndefined();
    pinWithEnter("F200W", { onAddCustomChip });
    expect(pinned).toEqual(["F200W"]);
    expect(details(view({ onAddCustomChip, customChips: ["f200w"], activeFilters: ["f200w"] }))?.props.open).toBe(true);
  });

  it("keeps the Filters block closed once the user collapses it after a pin", () => {
    const props: Partial<ListProps> = { onAddCustomChip: () => {}, customChips: ["f200w"], activeFilters: ["f200w"] };
    pinWithEnter("F200W", props);
    const onToggle = details(view(props))?.props.onToggle as (e: { currentTarget: { open: boolean } }) => void;
    onToggle({ currentTarget: { open: false } });
    expect(details(view(props))?.props.open).toBe(false);
  });
});

describe("MetadataFileList stylesheet", () => {
  const css = ["../../../advanced-viewer.css", "../metadata-file-list.css"]
    .map((rel) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8"))
    .join("\n")
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selectors: selector.split(",").map((s) => s.trim()), body }));
  const selectors = rules.flatMap((rule) => rule.selectors);

  function ruleIndex(selector: string): number {
    return rules.findIndex((rule) => rule.selectors.includes(selector));
  }

  function declared(selector: string, property: string): string | undefined {
    const values = rules
      .filter((rule) => rule.selectors.includes(selector))
      .flatMap((rule) => [...rule.body.matchAll(/([\w-]+)\s*:\s*([^;]+);/g)])
      .filter(([, name]) => name === property)
      .map(([, , value]) => value.trim());
    return values.pop();
  }

  it("keeps no rule for the removed product filter row or filter label", () => {
    expect(selectors.filter((s) => /\.ab-mfl-(?:product-filter|filter-label)(?![\w-])/.test(s))).toEqual([]);
  });

  it("styles the list through its own classes, with no ancestor-qualified rule over the header, search, footer or chips", () => {
    expect(selectors.filter((s) => /^\.ab-mfl-root\s/.test(s) || /\s\.ab-mfl-product-chip(?![\w-])/.test(s))).toEqual([]);
    expect(declared(".ab-mfl-product-chip", "min-height")).toBe("24px");
    expect(declared(".ab-mfl-product-chip", "padding")).toBe("4px 8px");
  });

  it("orders the Clear and pinned chip variants after the base chip rule, so their padding and colors win", () => {
    const base = ruleIndex(".ab-mfl-product-chip");
    expect(base).toBeGreaterThanOrEqual(0);
    for (const variant of [".ab-mfl-product-chip-clear", ".ab-mfl-product-chip-custom"]) expect(ruleIndex(variant), variant).toBeGreaterThan(base);
  });
});

const X1D_REJECTION =
  "Failed to load C:/d/jw01_nrs1_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=EXTRACT1D XTENSION=BINTABLE NAXIS=2 [232x1024] -- BINTABLE extension, holds no image pixels";

const X1D: Spectrum1dKind = { kind: "x1d", hdu: 1 };
const WCS1D: Spectrum1dKind = { kind: "wcs1d", hdu: 0 };
const E1: Spectrum1dKind = { kind: "x1dints", layout: "rows", n_series: 1, n_integrations: 240, n_channels: 431 };
const E1_PATH = "C:/d/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits";

function spectrumFile(id: string, path: string, spectrum: Spectrum1dKind): MetadataFile {
  return { id, name: path.split("/").pop() ?? path, path, size: 1, status: "spectrum", spectrum };
}

function renderRow(file: MetadataFile, props: Partial<ComponentProps<typeof MetadataFileList>> = {}): string {
  return renderToStaticMarkup(
    createElement(MetadataFileList, {
      files: [file],
      selectedId: null,
      onSelect: () => {},
      onOpenSpectrum: () => {},
      onOpenTable: () => {},
      ...props,
    }),
  );
}

describe("MetadataFileList spectrum rows", () => {
  it("renders an x1d spectrum row as clickable with Open as spectrum and Compare with its cube when its cube is loaded", () => {
    const x1d = spectrumFile("x", "C:/d/jw01_nrs1_x1d.fits", X1D);
    const html = renderRow(x1d, { comparableX1d: new Set(["x"]) });
    expect(html).toContain("ab-mfl-item-clickable");
    expect(html).toContain('data-status="spectrum"');
    expect(html).toContain('class="ab-mfl-spectrum-text"');
    expect(html).toContain(">1D spectrum, EXTRACT1D table<");
    expect(html).toContain(">Open as spectrum<");
    expect(html).toContain(">Compare with its cube<");
    expect(html).toContain('title="Select this spectrum and open the Spectra tool"');
    expect(html).toContain('title="Select its cube, open the Spectra tool and compare this x1d with the cube spectrum"');
  });

  it("drops Compare with its cube while the sibling cube is not loaded and keeps Open as spectrum enabled", () => {
    const x1d = spectrumFile("x", "C:/d/jw01_nrs1_x1d.fits", X1D);
    const html = renderRow(x1d, { comparableX1d: new Set() });
    expect(html).toContain(">Open as spectrum<");
    expect(html).not.toContain("Compare with its cube");
    expect(html).not.toContain("disabled");
  });

  it("offers no comparison for a 1D array spectrum", () => {
    const html = renderRow(spectrumFile("w", "C:/d/ha_spectrum.fits", WCS1D), { comparableX1d: new Set() });
    expect(html).toContain("ab-mfl-item-clickable");
    expect(html).toContain(">1D spectrum, 1D array<");
    expect(html).toContain(">Open as spectrum<");
    expect(html).not.toContain("Compare with its cube");
  });

  it("names the spectrum from the probe and its HDU, never from an image rejection", () => {
    const html = renderRow({ ...spectrumFile("x", "C:/d/jw01_nrs1_x1d.fits", X1D), error: X1D_REJECTION });
    expect(html).toContain('<span class="ab-mfl-spectrum-text" title="1D spectrum, EXTRACT1D table, HDU 1">1D spectrum, EXTRACT1D table</span>');
    expect(html).not.toContain("Failed to load");
    const oneD = renderRow({ ...spectrumFile("w", "C:/d/ha_spectrum.fits", WCS1D), error: X1D_REJECTION });
    expect(oneD).toContain(">1D spectrum, 1D array<");
    expect(oneD).not.toContain("EXTRACT1D");
  });

  it("keeps an error row unclickable, shows its message and offers no spectrum action", () => {
    const failed: MetadataFile = { id: "e", name: "jw01_nrs1_x1d.fits", path: "C:/d/jw01_nrs1_x1d.fits", size: 1, status: "error", error: X1D_REJECTION };
    const html = renderRow(failed, { comparableX1d: new Set(["e"]) });
    expect(html).not.toContain("ab-mfl-item-clickable");
    expect(html).toContain('class="ab-mfl-error-text"');
    expect(html).not.toContain("Open as spectrum");
    expect(html).not.toContain("Compare with its cube");
  });

  it("opens the selected spectrum row on Enter and does nothing on an image row", () => {
    const x1d = spectrumFile("x", "C:/d/jw01_nrs1_x1d.fits", X1D);
    const image = doneFile("img", "673nmos.fits");
    type EnterEvent = { key: string; preventDefault: () => void; target?: unknown; currentTarget?: unknown };
    function pressEnter(selectedId: string, event: Omit<EnterEvent, "key" | "preventDefault"> = {}) {
      hooks.mount();
      const spy = vi.fn();
      const preventDefault = vi.fn();
      const tree = elements(hooks.render(listView, { files: [x1d, image], selectedId, onSelect: () => {}, onOpenSpectrum: spy }) as ReactNode);
      const listbox = tree.find((el) => el.props.role === "listbox");
      if (!listbox) throw new Error("no listbox in the Files list");
      (listbox.props.onKeyDown as (e: EnterEvent) => void)({ key: "Enter", preventDefault, ...event });
      hooks.mount();
      return { spy, preventDefault };
    }
    const spectrumRow = pressEnter("x");
    expect(spectrumRow.spy.mock.calls).toEqual([["x"]]);
    expect(spectrumRow.preventDefault).toHaveBeenCalledTimes(1);
    const imageRow = pressEnter("img");
    expect(imageRow.spy).not.toHaveBeenCalled();
    expect(imageRow.preventDefault).not.toHaveBeenCalled();
    const rowButton = pressEnter("x", { target: {}, currentTarget: {} });
    expect(rowButton.spy).not.toHaveBeenCalled();
    expect(rowButton.preventDefault).not.toHaveBeenCalled();
  });

  it("does not offer a spectrum row for dragging into a composite", () => {
    const html = renderRow(spectrumFile("x", "C:/d/jw01_nrs1_x1d.fits", X1D));
    expect(html).toContain('draggable="false"');
  });

  it("names a row-per-integration file as time-series spectra, never as a 1D spectrum", () => {
    const html = renderRow(spectrumFile("s", E1_PATH, E1), { comparableX1d: new Set(["s"]) });
    expect(html).toContain(
      '<span class="ab-mfl-spectrum-text" title="Time-series spectra, 240 integrations x 431 channels, 1 series; one table row per integration">Time-series spectra, 240 integrations x 431 channels, 1 series</span>',
    );
    expect(html).toContain("ab-mfl-item-clickable");
    expect(html).toContain('data-status="spectrum"');
    expect(html).toContain(">Open time series<");
    expect(buttonTag(html, "Open time series")).toContain('title="Select these time-series spectra and open the Spectra tool"');
    expect(html).not.toContain("1D spectrum");
    expect(html).not.toContain(">Open as spectrum<");
    expect(html).not.toContain("Compare with its cube");
  });

  it("says how many EXTRACT1D tables an x1d file holds", () => {
    const html = renderRow(spectrumFile("m", "C:/d/jw01345_nirspec_mos_x1d.fits", { kind: "x1d", hdu: 2, n_tables: 3 }));
    expect(html).toContain(">1D spectra, 3 EXTRACT1D tables<");
    expect(html).toContain('title="1D spectra, 3 EXTRACT1D tables, HDU 2 opens first"');
    expect(html).toContain(">Open as spectrum<");
  });

  it("opens a selected time-series row on Enter", () => {
    hooks.mount();
    const spy = vi.fn();
    const preventDefault = vi.fn();
    const tree = elements(hooks.render(listView, { files: [spectrumFile("s", "C:/d/x_x1dints.fits", E1)], selectedId: "s", onSelect: () => {}, onOpenSpectrum: spy }) as ReactNode);
    const listbox = tree.find((el) => el.props.role === "listbox");
    if (!listbox) throw new Error("no listbox in the Files list");
    (listbox.props.onKeyDown as (e: { key: string; preventDefault: () => void }) => void)({ key: "Enter", preventDefault });
    hooks.mount();
    expect(spy.mock.calls).toEqual([["s"]]);
  });
});

const C1_PROBE: TableProbe = {
  format: "csv",
  n_rows: 297,
  columns: Array.from({ length: 17 }, (_, i) => ({ name: i === 0 ? "ra" : i === 1 ? "dec" : `col${i}`, unit: null, datatype: "float64", numeric: true })),
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "catalogue", reason: "sky position columns ra and dec" },
  notes: [],
};

const CSV_SPECTRUM_PROBE: TableProbe = {
  format: "csv",
  n_rows: 431,
  columns: [
    { name: "channel", unit: null, datatype: "float64", numeric: true },
    { name: "wavelength_vacuum_um", unit: null, datatype: "float64", numeric: true },
    { name: "flux", unit: null, datatype: "float64", numeric: true },
  ],
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "spectrum", reason: "columns wavelength_vacuum_um and flux: a spectrum exported by AstroBurst" },
  notes: [],
};

const WHTLT_PROBE: TableProbe = {
  format: "ecsv",
  n_rows: 240,
  columns: ["MJD_UTC_NRS1", "BJD_TDB_NRS1", "whitelight_flux_NRS1"].map((name) => ({ name, unit: null, datatype: "float64", numeric: true })),
  tables: null,
  meta: null,
  comments: [],
  guess: { kind: "light_curve", reason: "time column MJD_UTC_NRS1 and value column whitelight_flux_NRS1" },
  notes: [],
};

const FITSCAT_PROBE: TableProbe = {
  format: "fits_bintable",
  n_rows: 500,
  columns: Array.from({ length: 8 }, (_, i) => ({ name: `c${i}`, unit: null, datatype: "float64", numeric: true })),
  tables: [{ hdu: 1, extname: "CATALOG", n_rows: 500, n_columns: 8 }],
  meta: null,
  comments: [],
  guess: { kind: "catalogue", reason: "sky position columns RA and DEC" },
  notes: [],
};

const WHTLT_NAME = "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_whtlt.ecsv";

function tableFile(id: string, name: string, table: TableProbe): MetadataFile {
  return { id, name, path: `C:/d/${name}`, size: 1, status: "table", table };
}

const ROW_CHIP_BUTTON = "<button type=\"button\" class=\"ab-mfl-chip";

describe("MetadataFileList table rows", () => {
  it("renders a table row as a clickable row with the action of its kind", () => {
    const table = tableFile("t", "C1_ls_dr10_box_A.csv", C1_PROBE);
    const html = renderRow(table, { comparableX1d: new Set(["t"]) });
    expect(html).toContain('data-status="table"');
    expect(html).toContain('<div class="ab-mfl-status-dot" data-status="table" style="background:var(--ab-sky)"></div>');
    expect(html).toContain(">Table, 297 rows x 17 columns (CSV)<");
    expect(html).toContain(`<span class="ab-mfl-spectrum-text" title="${tableRowTitle(C1_PROBE)}">Table, 297 rows x 17 columns (CSV)</span>`);
    expect(html).toContain("ab-mfl-item-clickable");
    expect(html).not.toContain("Open as spectrum");
    expect(html).not.toContain("Compare with its cube");
    expect(html).not.toContain(ROW_CHIP_BUTTON);
    expect(html).toContain('draggable="false"');
    expect(rowHeights(html)).toEqual([{ id: "t", height: 72 }]);

    const lightCurve = renderRow(tableFile("w", WHTLT_NAME, WHTLT_PROBE), { onTableAction: () => {} });
    expect(lightCurve).toContain("ab-mfl-item-clickable");
    expect(lightCurve).toContain(">Open as light curve<");
    expect(buttonTag(lightCurve, "Open as light curve")).toContain('title="Open this table as a light curve in the Series section of Photometry"');
    expect(lightCurve).not.toContain("Use as catalogue");
    expect(rowHeights(lightCurve)).toEqual([{ id: "w", height: 72 }]);

    const catalogue = renderRow(table, { onTableAction: () => {} });
    expect(catalogue).toContain(">Use as catalogue<");
    expect(buttonTag(catalogue, "Use as catalogue")).toContain('title="Use this table as the catalogue of the Compare section of Photometry"');
    expect(rowHeights(catalogue)).toEqual([{ id: "t", height: 72 }]);

    const fitsTable = renderRow(tableFile("f", "cat.fits", FITSCAT_PROBE), { onTableAction: () => {} });
    expect(fitsTable).toContain("ab-mfl-item-clickable");
    expect(fitsTable).toContain(">Table, 500 rows x 8 columns (FITS table)<");
    expect(fitsTable).not.toContain(ROW_CHIP_BUTTON);
    expect(rowHeights(fitsTable)).toEqual([{ id: "f", height: 72 }]);

    const noCallback = renderRow(tableFile("w", WHTLT_NAME, WHTLT_PROBE));
    expect(noCallback).not.toContain(ROW_CHIP_BUTTON);
    expect(noCallback).not.toContain("Open as light curve");
  });

  it("selects a table row on click and hands it over from its button without selecting it", () => {
    hooks.mount();
    const select = vi.fn();
    const act = vi.fn();
    const stop = vi.fn();
    const tree = elements(hooks.render(listView, { files: [tableFile("t", WHTLT_NAME, WHTLT_PROBE)], selectedId: null, onSelect: select, onTableAction: act }) as ReactNode);
    const item = tree.find((el) => (el.props.file as MetadataFile | undefined)?.id === "t");
    if (!item) throw new Error("no row element for the table");
    hooks.mount();
    const row = elements(hooks.render((item.type as unknown as { type: (props: unknown) => ReactNode }).type, item.props) as ReactNode);
    hooks.mount();
    (row[0].props.onClick as () => void)();
    expect(select.mock.calls).toEqual([["t"]]);
    const button = row.find((el) => el.type === "button");
    if (!button) throw new Error("no action button on the table row");
    (button.props.onClick as (e: { stopPropagation: () => void }) => void)({ stopPropagation: stop });
    expect(stop).toHaveBeenCalledTimes(1);
    expect(act.mock.calls).toEqual([["t", "light_curve"]]);
    expect(select).toHaveBeenCalledTimes(1);
  });

  it("names a CSV spectrum row", () => {
    const spectrum: MetadataFile = { id: "s", name: "spectrum.csv", path: "C:/d/spectrum.csv", size: 1, status: "spectrum", table: CSV_SPECTRUM_PROBE };
    const html = renderRow(spectrum);
    expect(html).toContain('<span class="ab-mfl-spectrum-text" title="1D spectrum, CSV file">1D spectrum, CSV file</span>');
    expect(html).toContain(">Open as spectrum<");
    expect(html).toContain("ab-mfl-item-clickable");
  });
});

function rowHeights(html: string): { id: string; height: number }[] {
  const pattern = /<div id="mfl-opt-([^"]+)" role="option"[^>]*style="height:(\d+)px;overflow:hidden"/g;
  return [...html.matchAll(pattern)].map(([, id, height]) => ({ id, height: Number(height) }));
}

function spacerHeight(html: string): number | null {
  const match = /<div style="height:(\d+)px;position:relative">/.exec(html);
  return match ? Number(match[1]) : null;
}

describe("MetadataFileList row heights", () => {
  const image = doneFile("img", "673nmos.fits");
  const x1d = spectrumFile("x", "C:/d/jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits", X1D);
  const lone = spectrumFile("y", "C:/d/jw01266-o005_t001_nirspec_g235h-f170lp_x1d.fits", X1D);

  it("gives an x1d row with Open as spectrum and Compare with its cube room for the second button line", () => {
    const html = renderToStaticMarkup(
      createElement(MetadataFileList, {
        files: [image, x1d, lone],
        selectedId: null,
        onSelect: () => {},
        onOpenSpectrum: () => {},
        onOpenTable: () => {},
        comparableX1d: new Set(["x"]),
      }),
    );
    const heights = rowHeights(html);
    expect(heights.map((r) => r.id)).toEqual(["img", "x", "y"]);
    const [imageRow, twoButtons, oneButton] = heights.map((r) => r.height);
    expect(imageRow).toBe(72);
    expect(oneButton).toBe(72);
    expect(twoButtons).toBeGreaterThanOrEqual(90);
    expect(spacerHeight(html)).toBe(imageRow + twoButtons + oneButton);
  });

  it("keeps the one-line height when the list offers no cube comparison", () => {
    const html = renderToStaticMarkup(
      createElement(MetadataFileList, { files: [x1d], selectedId: null, onSelect: () => {}, onOpenSpectrum: () => {}, comparableX1d: new Set(["x"]) }),
    );
    expect(rowHeights(html).map((r) => r.height)).toEqual([72]);
  });

  it("keeps a time-series row at one button line even beside its cube", () => {
    const html = renderToStaticMarkup(
      createElement(MetadataFileList, {
        files: [spectrumFile("s", E1_PATH, E1)],
        selectedId: null,
        onSelect: () => {},
        onOpenSpectrum: () => {},
        onOpenTable: () => {},
        comparableX1d: new Set(["s"]),
      }),
    );
    expect(rowHeights(html).map((r) => r.height)).toEqual([72]);
  });

  it("windows rows of mixed heights by their real offsets", () => {
    const tops = rowTops([72, 92, 72, 72, 92, 72]);
    expect(tops).toEqual([0, 72, 164, 236, 308, 400, 472]);
    expect(rowWindow(tops, 0, 100, 0)).toEqual({ start: 0, end: 2 });
    expect(rowWindow(tops, 170, 140, 0)).toEqual({ start: 2, end: 5 });
    expect(rowWindow(tops, 400, 72, 0)).toEqual({ start: 5, end: 6 });
    expect(rowWindow(tops, 300, 100, 2)).toEqual({ start: 1, end: 6 });
    expect(rowWindow(rowTops([]), 0, 600, 4)).toEqual({ start: 0, end: 0 });
  });
});

function optionRow(html: string, id: string): string {
  const start = html.indexOf(`<div id="mfl-opt-${id}"`);
  if (start < 0) throw new Error(`no row ${id}`);
  const next = html.indexOf('<div id="mfl-opt-', start + 1);
  return next < 0 ? html.slice(start) : html.slice(start, next);
}

describe("MetadataFileList layer rows", () => {
  it("tags a layer file and leaves a science image without a tag", () => {
    const html = renderList([
      { ...doneFile("w", "legacysurvey-0532m280-invvar-r.fits.fz"), layer: { role: "inverse_variance", basis: "file name -invvar-", assumed: false } },
      { ...doneFile("s", "legacysurvey-0532m280-image-r.fits.fz"), layer: { role: "science", basis: "EXTNAME SCI", assumed: false } },
      doneFile("n", "m42.fits"),
    ]);
    expect(html.match(/>inverse variance</g)).toHaveLength(1);
    expect(optionRow(html, "w")).toContain('<span class="ab-mfl-chip ab-mfl-chip-neutral" title="inverse variance layer, from file name -invvar-">inverse variance</span>');
    expect(optionRow(html, "s")).not.toContain("ab-mfl-chip-neutral");
    expect(optionRow(html, "n")).not.toContain("ab-mfl-chip-neutral");
    expect(rowHeights(html)).toEqual([
      { id: "w", height: 72 },
      { id: "s", height: 72 },
      { id: "n", height: 72 },
    ]);
  });
});
