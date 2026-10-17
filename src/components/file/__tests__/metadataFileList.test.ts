import { describe, expect, it } from "vitest";
import { createElement, type ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import MetadataFileList, { type MetadataFile } from "../MetadataFileList";
import { rowTops, rowWindow } from "../../../utils/fileListRows";
import type { Spectrum1dKind } from "../../../shared/types/spectrum1d";

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

const X1D_REJECTION =
  "Failed to load C:/d/jw01_nrs1_x1d.fits: HDU 1 (EXTRACT1D): BINTABLE extension, holds no image pixels; " +
  "no HDU in this file holds a 2D image. " +
  "HDUs: [0] EXTNAME=(none) XTENSION=PRIMARY NAXIS=0 -- header only, no pixel data; " +
  "[1] EXTNAME=EXTRACT1D XTENSION=BINTABLE NAXIS=2 [232x1024] -- BINTABLE extension, holds no image pixels";

const X1D: Spectrum1dKind = { kind: "x1d", hdu: 1 };
const WCS1D: Spectrum1dKind = { kind: "wcs1d", hdu: 0 };

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

  it("does not offer a spectrum row for dragging into a composite", () => {
    const html = renderRow(spectrumFile("x", "C:/d/jw01_nrs1_x1d.fits", X1D));
    expect(html).toContain('draggable="false"');
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
