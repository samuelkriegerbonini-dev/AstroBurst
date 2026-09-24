import { describe, it, expect } from "vitest";
import {
  COMPOSITE_EXPORT_STEM,
  compositeExportName,
  compositeRgbPngStf,
  exportFileName,
  exportTimestamp,
  fileRgbPngStf,
  mefHduSummary,
  planeTag,
} from "../exportSources";

const STAMP = exportTimestamp(new Date(Date.UTC(2026, 8, 24, 2, 11, 14, 123)));
const NEUTRAL = { shadow: 0, midtone: 0.5, highlight: 1 };
const STRETCHED = { shadow: 0.01, midtone: 0.2, highlight: 1 };

describe("exportTimestamp", () => {
  it("is filename-safe and resolves milliseconds", () => {
    expect(STAMP).toBe("20260924-021114-123");
  });
});

describe("exportFileName", () => {
  it("gives two HDUs of the same file different names", () => {
    const sci = exportFileName("D:/n1/X.fits#hdu=1", "_proc", "fits", STAMP);
    const err = exportFileName("D:/n1/X.fits#hdu=2", "_proc", "fits", STAMP);
    expect(sci).toBe(`X_hdu1_proc_${STAMP}.fits`);
    expect(err).toBe(`X_hdu2_proc_${STAMP}.fits`);
    expect(sci).not.toBe(err);
  });

  it("gives same-named files exported at different times different names", () => {
    const first = exportFileName("D:/n1/X.fits", "_stf", "png", exportTimestamp(new Date(1000)));
    const second = exportFileName("D:/n2/X.fits", "_stf", "png", exportTimestamp(new Date(1001)));
    expect(first).not.toBe(second);
  });

  it("tags ASDF arrays and leaves auto refs untagged", () => {
    expect(planeTag("D:/a/f.asdf#array=err")).toBe("_err");
    expect(planeTag("D:/a/f.fits")).toBe("");
    expect(planeTag("D:/a/f.asdf#array=a%2Fb%20c")).toBe("_a_b_c");
    expect(exportFileName("D:/a/img.fits.fz", "_compressed", "fits", STAMP)).toBe(`img_compressed_${STAMP}.fits`);
  });
});

describe("compositeExportName", () => {
  it("never reuses the rgb_composite prefix that composite rendering deletes", () => {
    const name = compositeExportName("_stf_16bit", "png", STAMP);
    expect(name.startsWith("rgb_composite")).toBe(false);
    expect(name).toBe(`${COMPOSITE_EXPORT_STEM}_stf_16bit_${STAMP}.png`);
  });
});

describe("fileRgbPngStf", () => {
  it("sends the per-channel STF on screen for an RGB FITS instead of dropping it for a linked auto STF", () => {
    const args = fileRgbPngStf({ r: STRETCHED, g: { ...STRETCHED, midtone: 0.3 }, b: NEUTRAL, linked: false });
    expect(args.applyStfStretch).toBe(true);
    expect(args.linked).toBe(false);
    expect(args.midtoneR).toBe(0.2);
    expect(args.midtoneG).toBe(0.3);
    expect(args.midtoneB).toBe(0.5);
  });

  it("falls back to a per-channel auto STF, like the ingest preview, when no RGB view state exists", () => {
    expect(fileRgbPngStf(null)).toEqual({ applyStfStretch: false, linked: false });
  });
});

describe("compositeRgbPngStf", () => {
  it("sends the on-screen per-channel STF when only some channels are stretched", () => {
    const args = compositeRgbPngStf({ r: NEUTRAL, g: STRETCHED, b: STRETCHED }, false, false);
    expect(args.applyStfStretch).toBe(true);
    expect(args.linked).toBe(false);
    expect(args.midtoneR).toBe(0.5);
    expect(args.midtoneG).toBe(0.2);
    expect(args.shadowB).toBe(0.01);
  });

  it("sends a shadow/highlight-only STF with neutral midtones instead of replacing it by an auto STF", () => {
    const clip = { shadow: 0.2, midtone: 0.5, highlight: 0.9 };
    const args = compositeRgbPngStf({ r: clip, g: clip, b: clip }, false, true);
    expect(args.applyStfStretch).toBe(true);
    expect(args.linked).toBe(true);
    expect([args.shadowR, args.midtoneR, args.highlightR]).toEqual([0.2, 0.5, 0.9]);
    expect([args.shadowB, args.midtoneB, args.highlightB]).toEqual([0.2, 0.5, 0.9]);
  });

  it("sends the neutral STF on screen too, so the PNG is not auto-stretched unlike the preview", () => {
    const args = compositeRgbPngStf({ r: NEUTRAL, g: NEUTRAL, b: NEUTRAL }, false, false);
    expect(args.applyStfStretch).toBe(true);
    expect([args.shadowG, args.midtoneG, args.highlightG]).toEqual([0, 0.5, 1]);
  });

  it("passes the toggle through when there is no composite STF", () => {
    expect(compositeRgbPngStf(null, true, true)).toEqual({ applyStfStretch: true, linked: true });
    expect(compositeRgbPngStf(null, false, true)).toEqual({ applyStfStretch: false, linked: true });
  });
});

describe("mefHduSummary", () => {
  it("names the image HDUs the codec left uncompressed, which compress_mef_cmd now reports", () => {
    expect(mefHduSummary({ dropped: ["DQ"], kept_raw: [], uncompressed: ["TIME"] })).toEqual([
      "dropped: DQ",
      "kept raw: none",
      "copied uncompressed (codec cannot take them): TIME",
    ]);
  });

  it("adds no line when every image HDU was compressed", () => {
    expect(mefHduSummary({ dropped: [], kept_raw: ["ERR"], uncompressed: [] })).toEqual(["dropped: none", "kept raw: ERR"]);
  });
});
