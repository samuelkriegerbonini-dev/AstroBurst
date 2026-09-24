import { describe, it, expect } from "vitest";
import {
  assetUrlToPath,
  channelSourceLabel,
  exportSourceLabel,
  manifestLine,
  resolveRgbExportSource,
  rgbCubeUnavailableReason,
  rgbExportPaths,
  zipCandidates,
  zipEntryName,
} from "../exportSources";

const A = "C:/data/A.fits";
const A_PREVIEW = "http://asset.localhost/C%3A%2Fcache%2FA.png";

describe("resolveRgbExportSource", () => {
  it("uses the RGB file path, not null channels, when the screen shows that file's RGB view", () => {
    const src = resolveRgbExportSource({
      rgbChannels: null,
      compositePreviewUrl: A_PREVIEW,
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    expect(src).toEqual({ kind: "file", path: A });
  });

  it("treats all-null channels on an RGB file as the file, not the global composite", () => {
    const src = resolveRgbExportSource({
      rgbChannels: { r: null, g: null, b: null },
      compositePreviewUrl: A_PREVIEW,
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    expect(src).toEqual({ kind: "file", path: A });
  });

  it("uses Headers-tab channels when no RGB image is on screen", () => {
    const channels = { r: "r.fits", g: "g.fits", b: null };
    const src = resolveRgbExportSource({
      rgbChannels: channels,
      compositePreviewUrl: null,
      filePath: "C:/data/L.fits",
      fileIsRgb: false,
      filePreviewUrl: "http://asset.localhost/L.png",
    });
    expect(src).toEqual({ kind: "channels", channels });
  });

  it("prefers the RGB file on screen over channels assigned on other files", () => {
    const channels = { r: "C:/data/L1.fits", g: "C:/data/L2.fits", b: "C:/data/L3.fits" };
    const ownView = resolveRgbExportSource({
      rgbChannels: channels,
      compositePreviewUrl: A_PREVIEW,
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    const compositeReset = resolveRgbExportSource({
      rgbChannels: channels,
      compositePreviewUrl: null,
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    expect(ownView).toEqual({ kind: "file", path: A });
    expect(compositeReset).toEqual({ kind: "file", path: A });
  });

  it("prefers a composite on screen over channels assigned on other files", () => {
    const src = resolveRgbExportSource({
      rgbChannels: { r: "C:/data/L1.fits", g: "C:/data/L2.fits", b: "C:/data/L3.fits" },
      compositePreviewUrl: "http://asset.localhost/blend.png",
      filePath: "C:/data/L3.fits",
      fileIsRgb: false,
      filePreviewUrl: "http://asset.localhost/L3.png",
    });
    expect(src).toEqual({ kind: "composite" });
  });

  it("uses the composite cache when a wizard or composite result is on screen", () => {
    const onRgbFile = resolveRgbExportSource({
      rgbChannels: null,
      compositePreviewUrl: "http://asset.localhost/blend.png",
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    const onMonoFile = resolveRgbExportSource({
      rgbChannels: null,
      compositePreviewUrl: "http://asset.localhost/blend.png",
      filePath: "C:/data/L.fits",
      fileIsRgb: false,
      filePreviewUrl: "http://asset.localhost/L.png",
    });
    expect(onRgbFile).toEqual({ kind: "composite" });
    expect(onMonoFile).toEqual({ kind: "composite" });
  });

  it("returns null for a mono file with no composite and no channels", () => {
    expect(
      resolveRgbExportSource({
        rgbChannels: null,
        compositePreviewUrl: null,
        filePath: "C:/data/L.fits",
        fileIsRgb: false,
        filePreviewUrl: null,
      }),
    ).toBeNull();
  });
});

describe("channelSourceLabel", () => {
  it("names each assigned channel file and the missing ones", () => {
    expect(channelSourceLabel({ r: "C:\\data\\L1.fits", g: "/data/L2.fits", b: null })).toBe(
      "Source: Headers-tab channels R=L1.fits, G=L2.fits, B=none",
    );
  });
});

describe("rgbCubeUnavailableReason", () => {
  it("allows a single RGB FITS file, whose planes the backend reads by path", () => {
    expect(rgbCubeUnavailableReason(A, null)).toBeNull();
  });
  it("blocks partial channel assignments, which the backend would fill from the global composite cache", () => {
    expect(rgbCubeUnavailableReason(null, { r: "r.fits", g: null, b: null })).toMatch(/Assign R, G and B/);
    expect(rgbCubeUnavailableReason(null, { r: "r.fits", g: "g.fits", b: null })).toMatch(/Assign R, G and B/);
  });
  it("allows three channels or the composite on screen", () => {
    expect(rgbCubeUnavailableReason(null, { r: "r.fits", g: "g.fits", b: "b.fits" })).toBeNull();
    expect(rgbCubeUnavailableReason(null, { r: null, g: null, b: null })).toBeNull();
    expect(rgbCubeUnavailableReason(null, null)).toBeNull();
  });
});

describe("rgbExportPaths", () => {
  it("sends the RGB file as all three channels so the export reads that file, never the global composite slot", () => {
    expect(rgbExportPaths(A, null)).toEqual({ r: A, g: A, b: A });
    expect(rgbExportPaths(A, { r: null, g: null, b: null })).toEqual({ r: A, g: A, b: A });
  });
  it("passes Headers-tab channels through and sends nulls for the composite on screen", () => {
    expect(rgbExportPaths(null, { r: "r.fits", g: "g.fits", b: "b.fits" })).toEqual({ r: "r.fits", g: "g.fits", b: "b.fits" });
    expect(rgbExportPaths(null, null)).toEqual({ r: null, g: null, b: null });
  });
});

describe("exportSourceLabel", () => {
  it("names the processed result", () => {
    expect(exportSourceLabel({ isProcessed: true, label: "Background", previewOnly: false })).toBe("processed: Background");
  });
  it("says the original is exported for a PNG-only result", () => {
    expect(exportSourceLabel({ isProcessed: true, label: "Debayer", previewOnly: true })).toBe("original file (Debayer is PNG-only)");
  });
  it("says original when nothing is processed", () => {
    expect(exportSourceLabel({ isProcessed: false, label: null, previewOnly: false })).toBe("original file");
  });
});

describe("assetUrlToPath", () => {
  it("decodes a Windows asset URL and drops the cache-bust query", () => {
    expect(assetUrlToPath("http://asset.localhost/C%3A%5CUsers%5Cme%5Cout%5Cx.png?v=3")).toBe("C:\\Users\\me\\out\\x.png");
  });
  it("decodes an asset:// URL", () => {
    expect(assetUrlToPath("asset://localhost/%2Fhome%2Fme%2Fx%20y.png?v=2#f")).toBe("/home/me/x y.png");
  });
  it("rejects non-asset URLs", () => {
    expect(assetUrlToPath("data:image/png;base64,AAAA")).toBeNull();
    expect(assetUrlToPath("blob:http://localhost/abc")).toBeNull();
    expect(assetUrlToPath("C:/out/x.png")).toBeNull();
    expect(assetUrlToPath("http://asset.localhost/%E0%A4%A")).toBeNull();
  });
});

describe("zipEntryName", () => {
  it("renames every source extension to .png", () => {
    const taken = new Set<string>();
    expect(zipEntryName("a.fits", taken)).toBe("a.png");
    expect(zipEntryName("b.fts", taken)).toBe("b.png");
    expect(zipEntryName("c.asdf", taken)).toBe("c.png");
    expect(zipEntryName("d.fits.fz", taken)).toBe("d.png");
  });
  it("never lets two files share one entry", () => {
    const taken = new Set<string>();
    expect(zipEntryName("a.fits", taken)).toBe("a.png");
    expect(zipEntryName("a.fits", taken)).toBe("a_2.png");
    expect(zipEntryName("A.fit", taken)).toBe("A_3.png");
  });
});

describe("zipCandidates", () => {
  it("prefers the processed preview, falls back to the ingest PNG", () => {
    const c = zipCandidates({
      pngPath: "C:/cache/a.png",
      processed: { previewUrl: "http://asset.localhost/C%3A%2Fout%2Fa_bg.png?v=4", label: "Background" },
    });
    expect(c).toEqual([
      { path: "C:/out/a_bg.png", source: "processed (Background)" },
      { path: "C:/cache/a.png", source: "original preview (processed result Background not included)" },
    ]);
  });
  it("uses the ingest PNG when nothing is processed", () => {
    expect(zipCandidates({ pngPath: "C:/cache/a.png", processed: null })).toEqual([
      { path: "C:/cache/a.png", source: "original preview" },
    ]);
  });
  it("says the processed result is missing when it has no PNG", () => {
    expect(zipCandidates({ pngPath: "C:/cache/a.png", processed: { previewUrl: null, label: "Deconvolution" } })).toEqual([
      { path: "C:/cache/a.png", source: "original preview (processed result Deconvolution not included)" },
    ]);
  });
});

describe("manifestLine", () => {
  it("writes entry, source and file path separated by tabs", () => {
    expect(manifestLine("a.png", "original preview", "C:/data/a.fits")).toBe("a.png\toriginal preview\tC:/data/a.fits");
    expect(manifestLine(null, "skipped", "C:/data/b.fits")).toBe("-\tskipped\tC:/data/b.fits");
  });
});
