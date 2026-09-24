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

  it("accepts array keys of any length, as the backend no longer caps them", () => {
    const long = "a".repeat(200);
    expect(parseImageRef(`a.asdf#array=${long}`).plane).toEqual({ kind: "array", key: long });
  });

  it("decodes the %XX escapes Rust writes for array keys outside [A-Za-z0-9_.-]", () => {
    expect(parseImageRef("x.asdf#array=sci%20image")).toEqual({ path: "x.asdf", plane: { kind: "array", key: "sci image" } });
    expect(parseImageRef("x.asdf#array=data%282%29").plane).toEqual({ kind: "array", key: "data(2)" });
    expect(parseImageRef("x.asdf#array=a%23b").plane).toEqual({ kind: "array", key: "a#b" });
    expect(parseImageRef("x.asdf#array=%C3%A9t%C3%A9").plane).toEqual({ kind: "array", key: "été" });
  });

  it("refuses the escapes Rust parse_fragment refuses", () => {
    for (const s of ["a.fits#array=a%2", "a.fits#array=a%2x", "a.fits#array=a%2f", "a.fits#array=%41", "a.fits#array=%FF", "a.fits#array=é"]) {
      expect(parseImageRef(s)).toEqual({ path: s, plane: { kind: "auto" } });
    }
  });
});

describe("formatImageRef", () => {
  it("round-trips canonical inputs", () => {
    for (const s of ["a.fits", "a.fits#hdu=0", "a.fits#hdu=12", "b.asdf#array=dq", "b.asdf#array=roman.err", "C:/x/y#z/a.fits#array=var_poisson", "x.asdf#array=sci%20image", "x.asdf#array=%C3%A9t%C3%A9"]) {
      expect(formatImageRef(parseImageRef(s))).toBe(s);
    }
  });

  it("encodes array keys the way Rust ImageRef::cache_key does", () => {
    expect(arrayRef("x.asdf", "sci image")).toBe("x.asdf#array=sci%20image");
    expect(arrayRef("x.asdf", "a#b")).toBe("x.asdf#array=a%23b");
    expect(arrayRef("x.asdf", "a/b")).toBe("x.asdf#array=a%2Fb");
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

  it("labels an encoded array ref with its decoded key", () => {
    expect(planeLabel(parseImageRef("x.asdf#array=sci%20image"))).toBe("array sci image");
  });
});
