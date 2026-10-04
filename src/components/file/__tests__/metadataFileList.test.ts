import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import MetadataFileList, { type MetadataFile } from "../MetadataFileList";

function doneFile(id: string, name: string): MetadataFile {
  return { id, name, path: `C:/data/${name}`, size: 1, status: "done" };
}

function shownNames(names: string[]): { title: string; text: string }[] {
  const files = names.map((name, i) => doneFile(`f${i}`, name));
  const html = renderToStaticMarkup(createElement(MetadataFileList, { files, selectedId: null, onSelect: () => {} }));
  return [...html.matchAll(/<span class="ab-mfl-filename" title="([^"]*)">([^<]*)<\/span>/g)].map(([, title, text]) => ({ title, text }));
}

describe("MetadataFileList file names", () => {
  it("shows clear-f444w and f444w-f470n as different rows and keeps the full name in the title", () => {
    const clearF444w = "jw02739-o001_t001_nircam_clear-f444w_i2d.fits";
    const f444wF470n = "jw02739-o001_t001_nircam_f444w-f470n_i2d.fits";
    expect(shownNames([clearF444w, f444wF470n])).toEqual([
      { title: clearF444w, text: "...clear-f444w_i2d" },
      { title: f444wF470n, text: "...f444w-f470n_i2d" },
    ]);
  });

  it("names each row against the whole list, so rows that share the short form get their differing token", () => {
    const names = [
      "jw06565-o002_t001_nircam_clear-f090w_i2d.fits",
      "jw06565-o003_t001_nircam_clear-f090w_i2d.fits",
    ];
    expect(shownNames(names)).toEqual([
      { title: names[0], text: "o002...clear-f090w_i2d" },
      { title: names[1], text: "o003...clear-f090w_i2d" },
    ]);
  });
});
