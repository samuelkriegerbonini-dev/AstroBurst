import { describe, it, expect } from "vitest";
import { outputKeepList, urlToLocalPath, type KeepFile, type KeepWizard } from "../outputKeep";
import { EMPTY_CHAIN, withStep } from "../processingChain";
import type { CompositeChain } from "../../shared/types/compositeChain";
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
      historyId: null,
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

  it("keeps the composite chain's base, step and background model PNGs", () => {
    const stf = { shadow: 0, midtone: 0.5, highlight: 1 };
    const compositeChain: CompositeChain = {
      generation: 3,
      base: { previewUrl: asset(`${OUT}/composite_chain_base_1.png`), stf: { r: stf, g: stf, b: stf, linked: true } },
      steps: {
        background: {
          previewUrl: `${asset(`${OUT}/composite_chain_background_2.png`)}?v=2`,
          label: "Background",
          displayed: "linear",
          modelPreviewUrl: asset(`${OUT}/composite_chain_bgmodel_2.png`),
        },
        stretch: { previewUrl: asset(`${OUT}/composite_chain_stretch_3.png`), label: "Stretch", displayed: "stretched" },
      },
      psfKernel: null,
    };
    const keep = outputKeepList({ files: [], records: [], compositeChain });
    expect(keep).toEqual(expect.arrayContaining([
      `${OUT}/composite_chain_base_1.png`,
      `${OUT}/composite_chain_background_2.png`,
      `${OUT}/composite_chain_bgmodel_2.png`,
      `${OUT}/composite_chain_stretch_3.png`,
    ]));
    expect(outputKeepList({ files: [], records: [], compositeChain: null })).toEqual([]);
  });

  it("output_keep_list_skips_history_paths", () => {
    const HISTORY = "C:\\Users\\u\\AppData\\astroburst\\history";
    const node = (id: string) => `C:/Users/u/AppData/astroburst/history/0123456789abcdef/${id}`;
    const chain = withStep(
      withStep(EMPTY_CHAIN, "background", { fitsPath: node("n0001.fits"), previewUrl: asset(node("n0001.png")), dimensions: null }),
      "denoise",
      { fitsPath: `${OUT}/A_dn.fits`, previewUrl: asset(`${OUT}/A_dn.png`), dimensions: null },
    );
    const record: FileRenderState = {
      processed: {
        fitsPath: node("n0002.fits"),
        previewUrl: `${asset(node("n0002.png"))}?v=4`,
        dimensions: null,
        label: "Arcsinh",
        kind: "processing",
        inputPath: node("n0001.fits"),
      },
      chain,
      version: 4,
      historyId: "0123456789abcdef",
    };
    const keep = outputKeepList({ files: [file("A")], records: [record], historyDir: HISTORY });
    expect(keep.filter((p) => p.toLowerCase().replace(/\\/g, "/").includes("/history/"))).toEqual([]);
    expect(keep).toEqual(expect.arrayContaining([`${OUT}/A_dn.fits`, `${OUT}/A_dn.png`, `${OUT}/A.png`, "D:/data/A.fits"]));

    const sibling = outputKeepList({ files: [], records: [record], historyDir: `${HISTORY}-old` });
    expect(sibling).toContain(node("n0002.fits"));
    expect(outputKeepList({ files: [], records: [record] })).toContain(node("n0001.fits"));
  });

  it("strips plane fragments and collapses duplicates that differ only by case or slashes", () => {
    const f: KeepFile = { path: "D:/data/X.fits#hdu=2", sourcePath: "D:\\data\\X.fits", result: null };
    const keep = outputKeepList({ files: [f], records: [], previewUrls: [asset(`${OUT}/composite.png`)] });
    expect(keep.filter((p) => p.toLowerCase().replace(/\\/g, "/") === "d:/data/x.fits")).toHaveLength(1);
    expect(keep).toContain(`${OUT}/composite.png`);
  });
});
