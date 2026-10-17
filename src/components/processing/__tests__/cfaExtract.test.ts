import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { invoked } = vi.hoisted(() => ({
  invoked: [] as { via: string; cmd: string; outputDir?: string; args: Record<string, unknown> }[],
}));

vi.mock("../../../infrastructure/tauri", () => ({
  getOutputDir: vi.fn(async () => "C:/default-out"),
  typedInvoke: vi.fn(async (cmd: string, args: Record<string, unknown>) => {
    invoked.push({ via: "invoke", cmd, args });
    return { results: [], succeeded: 0, failed: 0, mode: args.mode, elapsed_ms: 1 };
  }),
  withPreview: vi.fn(async (cmd: string, outputDir: string | undefined, args: Record<string, unknown>) => {
    invoked.push({ via: "preview", cmd, outputDir, args });
    return { previewUrl: "asset://plane.png", ...args };
  }),
}));

import { CfaExtractResultView } from "../CfaExtractResultView";
import { EXTRACT_MODES, isExtractMode, planeRows, runPanelBatch, runPanelMode } from "../debayerModes";
import type { CfaExtractResult } from "../../../services/processing";
import { writtenFitsPaths } from "../../../utils/overwrittenFiles";

const HA_OIII: CfaExtractResult = {
  png_path: "C:/out/M42_ha_oiii_extract.png",
  path: "C:/data/M42.fits",
  pattern: "RGGB",
  mode: "ha_oiii",
  paths: ["C:/out/M42_ha_oiii_Ha.fits", "C:/out/M42_ha_oiii_OIII.fits"],
  planes: [
    { tag: "Ha", filter: "Ha", provenance: "ha", fits_path: "C:/out/M42_ha_oiii_Ha.fits" },
    { tag: "OIII", filter: "OIII", provenance: "oiii", fits_path: "C:/out/M42_ha_oiii_OIII.fits" },
  ],
  dimensions: [3000, 2000],
  trimmed: false,
  oiii_scale: { scale: 1.25, offset: -12.5 },
  elapsed_ms: 42,
};

describe("CFA extraction in the Debayer panel", () => {
  beforeEach(() => {
    invoked.length = 0;
  });

  it("offers the four extraction modes next to the two reconstructions", () => {
    expect(EXTRACT_MODES.map((mode) => mode.value)).toEqual(["green", "ha", "ha_oiii", "split"]);
    expect(isExtractMode("ha_oiii")).toBe(true);
    expect(isExtractMode("superpixel")).toBe(false);
  });

  it("runs Hα + OIII through extract_cfa_cmd with mode ha_oiii", async () => {
    await runPanelMode("ha_oiii", "C:/data/M42.fits", "C:/out", "");
    expect(invoked).toEqual([
      { via: "preview", cmd: "extract_cfa_cmd", outputDir: "C:/out", args: { path: "C:/data/M42.fits", mode: "ha_oiii", pattern: null } },
    ]);
  });

  it("keeps the reconstruction methods on debayer_fits_cmd", async () => {
    await runPanelMode("superpixel", "C:/data/M42.fits", "C:/out", "GBRG");
    expect(invoked).toEqual([
      { via: "preview", cmd: "debayer_fits_cmd", outputDir: "C:/out", args: { path: "C:/data/M42.fits", method: "superpixel", pattern: "GBRG" } },
    ]);
  });

  it("sends a batch extraction to extract_cfa_batch_cmd", async () => {
    await runPanelBatch("green", ["C:/data/a.fits", "C:/data/b.fits"], "C:/out", "RGGB");
    expect(invoked).toEqual([
      {
        via: "invoke",
        cmd: "extract_cfa_batch_cmd",
        args: { paths: ["C:/data/a.fits", "C:/data/b.fits"], outputDir: "C:/out", mode: "green", pattern: "RGGB" },
      },
    ]);
  });

  it("lists the written planes by tag and file name", () => {
    expect(planeRows(HA_OIII)).toEqual([
      { tag: "Ha", name: "M42_ha_oiii_Ha.fits", path: "C:/out/M42_ha_oiii_Ha.fits" },
      { tag: "OIII", name: "M42_ha_oiii_OIII.fits", path: "C:/out/M42_ha_oiii_OIII.fits" },
    ]);
  });

  it("renders the two plane paths and the OIII scale of a Hα + OIII result", () => {
    const html = renderToStaticMarkup(createElement(CfaExtractResultView, { result: HA_OIII }));
    expect(html).toContain("M42_ha_oiii_Ha.fits");
    expect(html).toContain("M42_ha_oiii_OIII.fits");
    expect(html).toContain("k = 1.250, c = -12.50");
    expect(html).toContain("RGGB");
  });

  it("names trimmed odd rows or columns", () => {
    const html = renderToStaticMarkup(createElement(CfaExtractResultView, { result: { ...HA_OIII, trimmed: true, oiii_scale: null } }));
    expect(html).toContain("odd last row or column was trimmed");
    expect(html).not.toContain("k = ");
  });

  it("passes the backend refusal of an X-Trans mosaic through verbatim", async () => {
    const refusal =
      "CFA extraction works on 2x2 Bayer mosaics; this is an X-Trans mosaic (36-character BAYERPAT), and X-Trans has no extraction mode.";
    const { withPreview } = await import("../../../infrastructure/tauri");
    vi.mocked(withPreview).mockRejectedValueOnce(new Error(refusal));
    await expect(runPanelMode("green", "C:/data/raf.fits", "C:/out", "")).rejects.toThrow(refusal);
  });

  it("announces every extracted plane as a written FITS file", () => {
    expect(writtenFitsPaths(HA_OIII)).toEqual(["C:/out/M42_ha_oiii_Ha.fits", "C:/out/M42_ha_oiii_OIII.fits"]);
    expect(writtenFitsPaths({ results: [HA_OIII, { path: "x", error: "no BAYERPAT" }] })).toEqual([
      "C:/out/M42_ha_oiii_Ha.fits",
      "C:/out/M42_ha_oiii_OIII.fits",
    ]);
  });
});
