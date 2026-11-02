import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { SaguiState } from "../saguiStore";
import type { SaguiResult } from "../../../shared/types/sagui";
import { DOCK_TOOL_META, DEFAULT_DOCK_LAYOUT } from "../../../utils/dockLayout";

const state: SaguiState = { config: { input_path: "", output_dir: "", rscript_path: "", library_path: "", n_regions: 20, starlet: true, starlet_scales: [1, 2, 3], starlet_threshold: 3, dq_exclude_bits: 4294967295, psf_target: "", mode: "ward", knn_k: 40 }, input: null, runtime: null, status: "idle", operation: null, installation: null, installParent: "", progress: null, result: null, outputDir: null, unit: null, error: null };
vi.mock("../saguiStore", async (original) => ({ ...await original<typeof import("../saguiStore")>(), useSaguiState: () => state }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getSelectedFile: () => null } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
import SaguiPanel from "../SaguiPanel";
import SaguiViewer from "../SaguiViewer";

describe("SAGUI desktop entry and scientific viewer", () => {
  it("opens without a file and explains external runtime and input requirements", () => {
    expect(DOCK_TOOL_META.sagui.needsFile).toBe(false);
    expect(DEFAULT_DOCK_LAYOUT.anchors["right-top"]).toContain("sagui");
    const html = renderToStaticMarkup(createElement(SaguiPanel));
    for (const text of ["Rscript executable", "Private R package library", "Use Cube Prep result", "Starlet scales", "Sparse Ward (approximation)", "DQ bits to exclude", "Working folder", "Install / repair SAGUI library", "Library installation folder", "Open previous SAGUI result"]) expect(html).toContain(text);
    expect(html).not.toContain("Segmentation completed");
  });

  it("shows missing-runtime issues without fabricated segmentations", () => {
    state.runtime = { available: false, ready: false, r_version: null, sagui_version: null, sagui_commit: null, issues: ["SAGUI dependency missing"] };
    const html = renderToStaticMarkup(createElement(SaguiPanel));
    expect(html).toContain("Runtime unavailable or incompatible");
    expect(html).toContain("SAGUI dependency missing");
    expect(html).not.toContain("Regions and SEDs");
    state.runtime = null;
  });

  it("identifies restored results without claiming a new segmentation ran", () => {
    state.status = "restored";
    const html = renderToStaticMarkup(createElement(SaguiPanel));
    expect(html).toContain("Previous result opened and verified");
    expect(html).not.toContain("Segmentation completed by the official R runtime");
    state.status = "idle";
  });

  it("renders real negative flux and leaves absent uncertainty unknown", () => {
    const result: SaguiResult = { run_id: "one", unit: "Jy", output_dir: "/out", labels_path: "labels.fits", mask_path: "mask.fits", table_path: "regions.csv", manifest_path: "manifest.json", width: 2, height: 1, labels: [0, 1], warnings: [], regions: [{ region: 1, n_pixels: 1, bands: [{ filter: "F200W", pivot_um: 2, flux: -2, error: null }] }] };
    const html = renderToStaticMarkup(createElement(SaguiViewer, { result }));
    expect(html).toContain("unknown");
    expect(html).toContain("-2.0000e+0");
    expect(html).toContain("Summed flux (Jy)");
    expect(html).toContain("Region 1 · 1 pixels");
  });

  it("labels regional flux with the result unit when the cube used millijanskys per pixel", () => {
    state.unit = "mJy/pixel";
    state.result = { run_id: "converted", unit: "Jy", output_dir: "/out", labels_path: "labels.fits", mask_path: "mask.fits", table_path: "regions.csv", manifest_path: "manifest.json", width: 1, height: 1, labels: [1], warnings: [], regions: [{ region: 1, n_pixels: 1, bands: [{ filter: "F200W", pivot_um: 2, flux: 0.011, error: null }] }] };
    try {
      const html = renderToStaticMarkup(createElement(SaguiPanel));
      expect(html).toContain("Summed flux (Jy)");
      expect(html).toContain("1.1000e-2");
      expect(html).not.toContain("Summed flux (mJy)");
    } finally {
      state.result = null;
      state.unit = null;
    }
  });
});
