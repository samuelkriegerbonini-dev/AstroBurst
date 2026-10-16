import { describe, expect, it } from "vitest";
import { CLASS_LABELS, copyDownloadUrls, copyMastUris, exactBytes, formatBytes, opensParts, productView, sectionExpanded, shortFilename, visibleProducts } from "../logic/productView";
import { productList } from "./fixtures";

describe("formatBytes", () => {
  it.each([
    [0, "0 B"],
    [999, "999 B"],
    [2_400, "2.4 KB"],
    [27_000_000, "27 MB"],
    [5_800_000, "5.8 MB"],
    [71_000_000, "71 MB"],
    [1_980_000_000, "1.98 GB"],
    [32_870_000_000, "32.87 GB"],
    [1_500_000_000_000, "1.50 TB"],
  ])("formats %d as %s with decimal units", (n, text) => {
    expect(formatBytes(n)).toBe(text);
  });

  it("gives the exact byte count for the tooltip", () => {
    expect(exactBytes(32_870_000_000)).toBe("32,870,000,000 bytes");
    expect(exactBytes(1)).toBe("1 byte");
  });
});

describe("productView", () => {
  const list = productList();

  it("shows what the app opens and the catalogues, with the other groups collapsed", () => {
    const view = productView(list, "opens", false);
    expect(view.sections.map((s) => [s.key, s.count, s.expanded])).toEqual([
      ["opens", 2, true],
      ["catalog", 1, true],
      ["preview", 1, false],
      ["association", 1, false],
      ["auxiliary", 1, false],
      ["other", 1, false],
    ]);
    expect(view.sections[0]).toMatchObject({ title: "Opens in the app", detail: "2 images, 10.84 GB" });
    expect(view.sections[1]).toMatchObject({ title: "Catalogues", detail: "1 file, 990 MB" });
    expect(view.sections[2]).toMatchObject({ title: "Previews", detail: "1 file, 14 MB (see Previews)" });
  });

  it("counts hidden CRDS reference and guide-star files", () => {
    const view = productView(list, "opens", false);
    expect(view.hidden).toEqual({
      count: 3,
      bytes: 11_770_000_000,
      reference: 2,
      guidestar: 1,
      text: "3 files, 11.77 GB: 2 CRDS reference files, 1 guide-star file",
    });
  });

  it("copies only the visible groups", () => {
    const view = productView(list, "opens", false);
    expect(view.visible.map((p) => p.filename)).toEqual(["a_i2d.fits", "b_i2d.fits", "a_cat.ecsv"]);
    expect(copyMastUris(view.visible)).toBe(["mast:JWST/product/a_i2d.fits", "mast:JWST/product/b_i2d.fits", "mast:JWST/product/a_cat.ecsv"].join("\n"));
    expect(copyDownloadUrls(view.visible).split("\n")[0]).toBe("https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:JWST/product/a_i2d.fits");
  });

  it("expands every group in Everything and adds hidden files only when asked", () => {
    const everything = productView(list, "everything", false);
    expect(everything.sections.every((s) => s.expanded)).toBe(true);
    expect(everything.visible).toHaveLength(7);
    const withHidden = productView(list, "everything", true);
    expect(withHidden.visible).toHaveLength(10);
    expect(withHidden.sections.at(-1)).toMatchObject({ key: "hidden", title: "Hidden", expanded: true, count: 3 });
  });

  it("summarises totals, timing and the cache date", () => {
    expect(productView(list, "opens", false).summary).toBe("10 files, 23.61 GB at MAST · listed in 29.5 s · cached until 2026-10-17 04:10 UTC");
    expect(productView({ ...list, fromCache: true, cachedUntil: null }, "opens", false).summary).toBe("10 files, 23.61 GB at MAST · from the cache");
  });

  it("names mixed classes in the opens section", () => {
    const mixed = {
      ...list,
      products: [
        { ...list.products[0], cls: "image" as const },
        { ...list.products[1], cls: "cube" as const, opensAs: "cube" as const },
        { ...list.products[1], filename: "s.fits", cls: "spectrum" as const, opensAs: "spectrum" as const, sizeBytes: 1000 },
      ],
    };
    expect(productView(mixed, "opens", false).sections[0].detail).toBe("1 image, 1 cube, 1 spectrum, 10.84 GB");
  });
});

describe("sections on screen", () => {
  const list = productList();

  it("names the classes each section holds", () => {
    expect(productView(list, "opens", true).sections.map((s) => [s.key, s.classes])).toEqual([
      ["opens", ["image", "cube", "spectrum", "ramp"]],
      ["catalog", ["catalog"]],
      ["preview", ["preview"]],
      ["association", ["association"]],
      ["auxiliary", ["auxiliary"]],
      ["other", ["other"]],
      ["hidden", ["reference", "guidestar"]],
    ]);
  });

  it("lets a section be opened or closed by hand and copies what is open", () => {
    const view = productView(list, "opens", false);
    expect(sectionExpanded(view.sections[2], {})).toBe(false);
    expect(sectionExpanded(view.sections[2], { preview: true })).toBe(true);
    expect(visibleProducts(view, {}).map((p) => p.filename)).toEqual(view.visible.map((p) => p.filename));
    expect(visibleProducts(view, { preview: true, catalog: false }).map((p) => p.filename)).toEqual(["a_i2d.fits", "b_i2d.fits", "a_thumb.jpg"]);
  });

  it("splits the opens section by class for its group markers", () => {
    expect(opensParts(productView(list, "opens", false).sections[0])).toEqual([{ cls: "image", count: 2, text: "2 images" }]);
  });

  it("shortens a file name that repeats the dataset id so its distinguishing part stays visible", () => {
    expect(shortFilename("jw02739-o001_t001_nircam_clear-f187n_i2d.fits", "jw02739-o001_t001_nircam")).toBe("…_clear-f187n_i2d.fits");
    expect(shortFilename("ick909030_drc.fits", "ICK909030")).toBe("…_drc.fits");
    expect(shortFilename("jw02739001002_02101_00001_nrca1_cal.fits", "jw02739-o001_t001_nircam")).toBe("jw02739001002_02101_00001_nrca1_cal.fits");
    expect(shortFilename("jw02739-o001_t001_nircam", "jw02739-o001_t001_nircam")).toBe("jw02739-o001_t001_nircam");
  });

  it("labels every class in words", () => {
    expect(CLASS_LABELS).toEqual({
      image: "Image",
      cube: "Cube",
      spectrum: "Spectrum",
      ramp: "Ramp",
      catalog: "Catalogue",
      preview: "Preview",
      association: "Association",
      auxiliary: "Auxiliary",
      reference: "CRDS reference",
      guidestar: "Guide star",
      other: "Other",
    });
  });
});
