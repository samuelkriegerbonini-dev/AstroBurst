import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import MetadataFileList, { type MetadataFile } from "../MetadataFileList";

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
