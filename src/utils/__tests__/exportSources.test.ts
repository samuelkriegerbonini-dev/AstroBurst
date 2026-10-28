import { describe, it, expect } from "vitest";
import {
  assetUrlToPath,
  channelSourceLabel,
  cubeFrameExportName,
  exportSourceLabel,
  exportTimestamp,
  manifestLine,
  resolveRgbExportSource,
  rgbCubeUnavailableReason,
  rgbExportPaths,
  rgbFilePngNote,
  rgbFileSourceTitle,
  zipCandidates,
  zipEntryName,
} from "../exportSources";
import { rgbProductOf, type RgbProductInput } from "../rgbProduct";
import type { ProcessedResult } from "../../shared/types/preview";

const A = "C:/data/A.fits";
const A_PREVIEW = "http://asset.localhost/C%3A%2Fcache%2FA.png";
const A_CURVES = "C:/history/3f9c0a1e5b7d2c44/n0001.fits";

const CURVES_ON_A: ProcessedResult = {
  fitsPath: A_CURVES,
  previewUrl: "http://asset.localhost/n0001.png?v=3",
  dimensions: [128, 128],
  label: "Curves",
  kind: "processing",
  inputPath: A,
  planes: 3,
  domain: "display_referred",
};

type ScreenInput = Omit<RgbProductInput, "processed"> & {
  rgbChannels: { r: string | null; g: string | null; b: string | null } | null;
  processed?: RgbProductInput["processed"];
};

function exportSourceOf({ rgbChannels, processed = null, ...screen }: ScreenInput) {
  return resolveRgbExportSource({ rgbChannels, product: rgbProductOf({ ...screen, processed }) });
}

describe("resolveRgbExportSource", () => {
  it("uses the RGB file path, not null channels, when the screen shows that file's RGB view", () => {
    const src = exportSourceOf({
      rgbChannels: null,
      compositePreviewUrl: A_PREVIEW,
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    expect(src).toEqual({ kind: "file", path: A });
  });

  it("treats all-null channels on an RGB file as the file, not the global composite", () => {
    const src = exportSourceOf({
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
    const src = exportSourceOf({
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
    const ownView = exportSourceOf({
      rgbChannels: channels,
      compositePreviewUrl: A_PREVIEW,
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    const compositeReset = exportSourceOf({
      rgbChannels: channels,
      compositePreviewUrl: null,
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    expect(ownView).toEqual({ kind: "file", path: A });
    expect(compositeReset).toEqual({ kind: "file", path: A });
  });

  it("exports the planes of a processed RGB result instead of the original file, whether or not the file view was reset yet", () => {
    for (const compositePreviewUrl of [null, A_PREVIEW]) {
      const src = exportSourceOf({ rgbChannels: null, compositePreviewUrl, filePath: A, fileIsRgb: true, filePreviewUrl: A_PREVIEW, processed: CURVES_ON_A });
      expect(src).toEqual({ kind: "file", path: A_CURVES });
    }
  });

  it("offers no RGB file export when a mono or PNG-only result is shown over an RGB file", () => {
    const base = { rgbChannels: null, compositePreviewUrl: null, filePath: A, fileIsRgb: true, filePreviewUrl: A_PREVIEW };
    expect(exportSourceOf({ ...base, processed: { ...CURVES_ON_A, planes: 1 } })).toBeNull();
    expect(exportSourceOf({ ...base, processed: { ...CURVES_ON_A, planes: undefined } })).toBeNull();
    expect(exportSourceOf({ ...base, processed: { ...CURVES_ON_A, fitsPath: null } })).toBeNull();
    const channels = { r: "C:/data/L1.fits", g: "C:/data/L2.fits", b: "C:/data/L3.fits" };
    expect(exportSourceOf({ ...base, rgbChannels: channels, processed: { ...CURVES_ON_A, planes: 1 } })).toEqual({ kind: "channels", channels });
  });

  it("prefers a composite on screen over channels assigned on other files", () => {
    const src = exportSourceOf({
      rgbChannels: { r: "C:/data/L1.fits", g: "C:/data/L2.fits", b: "C:/data/L3.fits" },
      compositePreviewUrl: "http://asset.localhost/blend.png",
      filePath: "C:/data/L3.fits",
      fileIsRgb: false,
      filePreviewUrl: "http://asset.localhost/L3.png",
    });
    expect(src).toEqual({ kind: "composite" });
  });

  it("uses the composite cache when a wizard or composite result is on screen", () => {
    const onRgbFile = exportSourceOf({
      rgbChannels: null,
      compositePreviewUrl: "http://asset.localhost/blend.png",
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
    });
    const onMonoFile = exportSourceOf({
      rgbChannels: null,
      compositePreviewUrl: "http://asset.localhost/blend.png",
      filePath: "C:/data/L.fits",
      fileIsRgb: false,
      filePreviewUrl: "http://asset.localhost/L.png",
    });
    const overCurves = exportSourceOf({
      rgbChannels: null,
      compositePreviewUrl: "http://asset.localhost/blend.png",
      filePath: A,
      fileIsRgb: true,
      filePreviewUrl: A_PREVIEW,
      processed: CURVES_ON_A,
    });
    expect(onRgbFile).toEqual({ kind: "composite" });
    expect(onMonoFile).toEqual({ kind: "composite" });
    expect(overCurves).toEqual({ kind: "composite" });
  });

  it("returns null for a mono file with no composite and no channels", () => {
    expect(
      exportSourceOf({
        rgbChannels: null,
        compositePreviewUrl: null,
        filePath: "C:/data/L.fits",
        fileIsRgb: false,
        filePreviewUrl: null,
      }),
    ).toBeNull();
  });
});

describe("RGB file source notes", () => {
  const curves = { label: "Curves", displayReferred: true };

  it("names the processed result when its planes are exported, and the file otherwise", () => {
    expect(rgbFileSourceTitle(curves)).toBe("Source: the R, G and B planes and header of the processed result (Curves)");
    expect(rgbFileSourceTitle(null)).toBe("Source: this RGB file's own planes and header");
  });

  it("says a display-referred result is written as shown and a file follows the STF on screen", () => {
    expect(rgbFilePngNote(curves, null)).toBe("Source: the processed result (Curves) as shown; its stretch is baked in, so no STF is applied");
    expect(rgbFilePngNote({ ...curves, displayReferred: false }, null)).toBe("Source: the processed result (Curves) with a per-channel auto STF");
    expect(rgbFilePngNote(null, { linked: true })).toBe("Source: this RGB file with the linked STF on screen");
    expect(rgbFilePngNote(null, { linked: false })).toBe("Source: this RGB file with the per-channel STF on screen");
    expect(rgbFilePngNote(null, null)).toBe("Source: this RGB file with a per-channel auto STF");
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

describe("cubeFrameExportName", () => {
  const STAMP = exportTimestamp(new Date(Date.UTC(2026, 9, 10, 12, 0, 0, 5)));

  it("tags the HDU so the SCI and ERR frames of one cube never share a file", () => {
    const sci = cubeFrameExportName("D:/c/cube_s3d.fits#hdu=1", 0, "png", STAMP);
    const err = cubeFrameExportName("D:/c/cube_s3d.fits#hdu=2", 0, "png", STAMP);
    expect(sci).toBe(`cube_s3d_hdu1_frame_0000_${STAMP}.png`);
    expect(err).toBe(`cube_s3d_hdu2_frame_0000_${STAMP}.png`);
  });

  it("gives two export runs of same-named cubes different names", () => {
    const first = cubeFrameExportName("D:/n1/cube.fits", 12, "fits", exportTimestamp(new Date(1000)));
    const second = cubeFrameExportName("D:/n2/cube.fits", 12, "fits", exportTimestamp(new Date(1001)));
    expect(first).toContain("_frame_0012_");
    expect(first).not.toBe(second);
  });
});
