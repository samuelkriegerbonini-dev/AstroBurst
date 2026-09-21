import { describe, it, expect } from "vitest";
import { parseImageRef, formatImageRef, hduRef, arrayRef, planeLabel, exportStem } from "../imageRef";

describe("parseImageRef", () => {
  it("returns auto for a plain path", () => {
    expect(parseImageRef("C:/data/a.fits")).toEqual({ path: "C:/data/a.fits", plane: { kind: "auto" } });
  });

  it("parses #hdu=<n>", () => {
    expect(parseImageRef("a.fits#hdu=3")).toEqual({ path: "a.fits", plane: { kind: "hdu", index: 3 } });
    expect(parseImageRef("a.fits#hdu=0")).toEqual({ path: "a.fits", plane: { kind: "hdu", index: 0 } });
  });

  it("parses #array=<key> with dotted keys", () => {
    expect(parseImageRef("r0000.asdf#array=roman.dq")).toEqual({
      path: "r0000.asdf",
      plane: { kind: "array", key: "roman.dq" },
    });
  });

  it("keeps a directory containing # as part of the path", () => {
    expect(parseImageRef("C:/data/run#7/a.fits")).toEqual({ path: "C:/data/run#7/a.fits", plane: { kind: "auto" } });
  });

  it("uses the last # as the fragment", () => {
    expect(parseImageRef("C:/run#7/a.fits#hdu=2")).toEqual({ path: "C:/run#7/a.fits", plane: { kind: "hdu", index: 2 } });
  });

  it("treats invalid fragments as part of the path", () => {
    for (const s of ["a.fits#hdu=x", "a.fits#foo=1", "a.fits#hdu=", "a.fits#hdu=-1", "a.fits#hdu= 1", "a.fits#array=", "a.fits#array=a b", "#hdu=1"]) {
      expect(parseImageRef(s)).toEqual({ path: s, plane: { kind: "auto" } });
    }
  });

  it("never splits synthetic keys", () => {
    for (const s of ["__composite_r", "__wizard_ch_ha_aligned#hdu=1", "__star_mask#array=dq"]) {
      expect(parseImageRef(s)).toEqual({ path: s, plane: { kind: "auto" } });
    }
  });

  it("enforces the 128-char array key limit", () => {
    const ok = "a".repeat(128);
    expect(parseImageRef(`a.asdf#array=${ok}`).plane.kind).toBe("array");
    const tooLong = "a".repeat(129);
    expect(parseImageRef(`a.asdf#array=${tooLong}`).plane.kind).toBe("auto");
  });
});

describe("formatImageRef", () => {
  it("round-trips canonical inputs", () => {
    for (const s of ["a.fits", "a.fits#hdu=0", "a.fits#hdu=12", "b.asdf#array=dq", "b.asdf#array=roman.err", "C:/x/y#z/a.fits#array=var_poisson"]) {
      expect(formatImageRef(parseImageRef(s))).toBe(s);
    }
  });
});

describe("hduRef / arrayRef", () => {
  it("build canonical refs", () => {
    expect(hduRef("a.fits", 3)).toBe("a.fits#hdu=3");
    expect(arrayRef("r.asdf", "roman.dq")).toBe("r.asdf#array=roman.dq");
  });
});

describe("exportStem", () => {
  it("strips directories and the FITS extension", () => {
    expect(exportStem("C:\\data\\jw01234_i2d.fits")).toBe("jw01234_i2d");
    expect(exportStem("/home/u/obs/m51.fit")).toBe("m51");
    expect(exportStem("C:/data/cube.fts")).toBe("cube");
  });

  it("drops the image-ref fragment before building the stem", () => {
    expect(exportStem("C:\\data\\jw01234_i2d.fits#hdu=2")).toBe("jw01234_i2d");
    expect(exportStem("C:/data/r0000.asdf#array=roman.dq")).toBe("r0000");
    expect(exportStem("C:/data/a.fits#foo=1")).toBe("a");
  });

  it("covers the other accepted source extensions", () => {
    expect(exportStem("bundle.zip")).toBe("bundle");
    expect(exportStem("obs.asdf")).toBe("obs");
    expect(exportStem("obs.fits.gz")).toBe("obs");
    expect(exportStem("obs.fits.fz")).toBe("obs");
  });

  it("falls back when nothing usable remains", () => {
    expect(exportStem("")).toBe("output");
    expect(exportStem(".fits")).toBe("output");
    expect(exportStem("", "image")).toBe("image");
  });
});

describe("planeLabel", () => {
  it("returns null for auto", () => {
    expect(planeLabel(parseImageRef("a.fits"))).toBeNull();
    expect(planeLabel(parseImageRef("a.fits"), "SCI")).toBeNull();
  });

  it("labels HDUs with and without extname", () => {
    expect(planeLabel(parseImageRef("a.fits#hdu=3"))).toBe("HDU 3");
    expect(planeLabel(parseImageRef("a.fits#hdu=3"), null)).toBe("HDU 3");
    expect(planeLabel(parseImageRef("a.fits#hdu=3"), "SCI")).toBe("HDU 3 · SCI");
  });

  it("labels arrays by key", () => {
    expect(planeLabel(parseImageRef("r.asdf#array=dq"))).toBe("array dq");
    expect(planeLabel(parseImageRef("r.asdf#array=roman.dq"), "roman.dq")).toBe("array roman.dq");
  });
});
