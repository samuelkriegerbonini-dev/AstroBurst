import { describe, it, expect } from "vitest";
import { outputKeepList, urlToLocalPath, type KeepFile, type KeepWizard } from "../outputKeep";
import { EMPTY_CHAIN, withStep } from "../processingChain";
import type { FileRenderState } from "../../shared/types/preview";

const OUT = "C:/Users/u/AppData/astroburst/output";
const asset = (p: string) => `http://asset.localhost/${encodeURIComponent(p)}`;

function file(stem: string, extra: Partial<KeepFile["result"]> = {}): KeepFile {
  return {
    path: `D:/data/${stem}.fits`,
    sourcePath: `D:/data/${stem}.fits`,
    result: { png_path: `${OUT}/${stem}.png`, previewUrl: asset(`${OUT}/${stem}.png`), ...extra },
  };
}

describe("urlToLocalPath", () => {
  it("decodes asset URLs and drops cache-busting queries", () => {
    expect(urlToLocalPath(`${asset(`${OUT}/a.png`)}?v=3`)).toBe(`${OUT}/a.png`);
    expect(urlToLocalPath(`${OUT}/a.png?t=1`)).toBe(`${OUT}/a.png`);
    expect(urlToLocalPath("data:image/png;base64,AAAA")).toBeNull();
    expect(urlToLocalPath(null)).toBeNull();
  });
});

describe("outputKeepList", () => {
  it("keeps the auto-resampled stacking inputs written at load time", () => {
    const f = file("M31_L", {
      resampledPath: `${OUT}/M31_L_resampled.fits`,
      resampled: {
        png_path: `${OUT}/M31_L_resampled.png`,
        fits_path: `${OUT}/M31_L_resampled.fits`,
        dimensions: [100, 100],
        original_dimensions: [200, 200],
        wcs_updates: {},
        stats: { min: 0, max: 1, mean: 0.5, sigma: 0.1 },
      },
    });
    const keep = outputKeepList({ files: [f], records: [] });
    expect(keep).toContain(`${OUT}/M31_L_resampled.fits`);
    expect(keep).toContain(`${OUT}/M31_L_resampled.png`);
  });

  it("keeps every loaded file's ingest preview", () => {
    const keep = outputKeepList({ files: [file("A"), file("B")], records: [] });
    expect(keep).toContain(`${OUT}/A.png`);
    expect(keep).toContain(`${OUT}/B.png`);
  });

  it("keeps the processed result and every chain entry of every render record", () => {
    const chain = withStep(
      withStep(EMPTY_CHAIN, "background", { fitsPath: `${OUT}/A_bg.fits`, previewUrl: `${asset(`${OUT}/A_bg.png`)}?v=2`, dimensions: null }),
      "denoise",
      { fitsPath: `${OUT}/A_dn.fits`, previewUrl: null, dimensions: null },
    );
    const record: FileRenderState = {
      processed: {
        fitsPath: `${OUT}/A_dn.fits`,
        previewUrl: `${asset(`${OUT}/A_dn.png`)}?v=5`,
        dimensions: null,
        label: "Denoise",
        kind: "processing",
        inputPath: `${OUT}/A_bg.fits`,
      },
      chain,
      version: 5,
    };
    const keep = outputKeepList({ files: [], records: [record] });
    expect(keep).toEqual(expect.arrayContaining([
      `${OUT}/A_bg.fits`,
      `${OUT}/A_bg.png`,
      `${OUT}/A_dn.fits`,
      `${OUT}/A_dn.png`,
    ]));
  });

  it("keeps the wizard's on-disk intermediate outputs and skips cache keys", () => {
    const wizard: KeepWizard = {
      stackedPaths: { r: `${OUT}/stack_r.fits` },
      backgroundPaths: { r: `${OUT}/r_bg.fits`, g: "__wizard_ch_g_aligned" },
      channelResults: { r: { starless: { path: `${OUT}/r_starless.fits`, note: "" }, stretched: null } },
      starMaskPath: null,
      segmPath: null,
      resultPng: null,
      resultFits: null,
    };
    const keep = outputKeepList({ files: [], records: [], wizard });
    expect(keep).toEqual(expect.arrayContaining([`${OUT}/stack_r.fits`, `${OUT}/r_bg.fits`, `${OUT}/r_starless.fits`]));
    expect(keep.some((p) => p.startsWith("__"))).toBe(false);
  });

  it("strips plane fragments and collapses duplicates that differ only by case or slashes", () => {
    const f: KeepFile = { path: "D:/data/X.fits#hdu=2", sourcePath: "D:\\data\\X.fits", result: null };
    const keep = outputKeepList({ files: [f], records: [], previewUrls: [asset(`${OUT}/composite.png`)] });
    expect(keep.filter((p) => p.toLowerCase().replace(/\\/g, "/") === "d:/data/x.fits")).toHaveLength(1);
    expect(keep).toContain(`${OUT}/composite.png`);
  });
});
