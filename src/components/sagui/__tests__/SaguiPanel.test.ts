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

const render = () => renderToStaticMarkup(createElement(SaguiPanel));
const liveRegion = (html: string) => /<div aria-live="polite"[^>]*>(.*?)<\/div>/.exec(html)?.[1] ?? "";
const runButton = (html: string) => /<button[^>]*class="ab-run-btn[^"]*"[^>]*>(?:(?!<\/button>).)*<\/button>/.exec(html)?.[0] ?? "";
function withState(change: Partial<SaguiState>, check: () => void) {
  const saved = { ...state };
  Object.assign(state, change);
  try { check(); } finally { Object.assign(state, saved); }
}
const RUN = "C:\\work\\sagui-20261106-one";
const RESULT: SaguiResult = { run_id: "one", unit: "Jy", output_dir: RUN, labels_path: `${RUN}\\labels.fits`, mask_path: `${RUN}\\support.fits`, table_path: `${RUN}\\regions.csv`, manifest_path: `${RUN}\\manifest.json`, width: 1, height: 1, labels: [1], warnings: [], regions: [{ region: 1, n_pixels: 1, bands: [{ filter: "F200W", pivot_um: 2, flux: 0.5, error: 0.1 }, { filter: "F444W", pivot_um: 4.4, flux: 0.7, error: 0.1 }] }] };
const INPUT = { width: 40, height: 30, bands: [{ filter: "F200W", pivot_um: 2 }, { filter: "F444W", pivot_um: 4.4 }], unit: "Jy/pixel", has_err: true, has_dq: true, has_noise_correlation: false, dq_flags: [], estimated_memory_bytes: 2 * 1024 ** 3, warnings: ["NOISCORR extension is absent: regional errors assume independent pixels and error_correlated stays empty; resampling and PSF matching create covariance that is not stored"] };

describe("SAGUI desktop entry and scientific viewer", () => {
  it("opens without a file and explains external runtime and input requirements", () => {
    expect(DOCK_TOOL_META.sagui.needsFile).toBe(false);
    expect(DEFAULT_DOCK_LAYOUT.anchors["right-top"]).toContain("sagui");
    const html = renderToStaticMarkup(createElement(SaguiPanel));
    for (const text of ["Rscript executable", "Private R package library", "Use Cube Prep result", "Starlet scales", "Sparse Ward (approximation)", "DQ bits to exclude", "Working folder", "Install / repair SAGUI library", "Library installation folder", "Open previous SAGUI result"]) expect(html).toContain(text);
    expect(html).not.toContain("Segmentation completed");
  });

  it("states that R 4.6.2 installs beside other R versions and links its installer in the CRAN archive", () => {
    const html = renderToStaticMarkup(createElement(SaguiPanel));
    expect(html).toContain("Required: R 4.6.2 (installs alongside other R versions;");
    expect(html).toContain("https://cran.r-project.org/bin/windows/base/old/4.6.2/");
    expect(html).toContain("SAGUI commit 56fd3ea776d7c00e056368498da9412bd58b84bb");
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

describe("SAGUI panel status, actions and products", () => {
  it("names the folder and the cancellation after the operation that made them (SCIENCE-18)", () => {
    withState({ status: "installed", operation: "install", outputDir: "C:\\R\\install-1" }, () => {
      const html = render();
      expect(html).toContain(">Installation folder<");
      expect(html).not.toContain("Run folder");
    });
    withState({ status: "cancelled", operation: "install", outputDir: "C:\\R\\install-1" }, () => {
      const live = liveRegion(render());
      expect(live).toContain("Installation cancelled. Any partial files and logs are in the installation folder.");
      expect(live).not.toContain("run folder");
    });
    withState({ status: "cancelled", operation: "segment", outputDir: null }, () => {
      const html = render();
      expect(liveRegion(html)).toContain("Cancelled before SAGUI started; no run folder was created.");
      expect(html).not.toContain("Run folder");
    });
    withState({ status: "cancelled", operation: "segment", outputDir: RUN }, () => {
      const html = render();
      expect(liveRegion(html)).toContain("Cancelled. Any partial files and logs remain in the run folder.");
      expect(html).toContain(">Run folder<");
    });
  });

  it("says that a reopened result keeps the current R installation and working folder", () => {
    withState({ status: "restored", result: RESULT, outputDir: RUN }, () => {
      const live = liveRegion(render());
      expect(live).toContain("Previous result opened and verified. Its input and analysis settings are shown above; your Rscript, library and working folder were kept.");
      expect(live).not.toContain("Its saved settings are shown above");
    });
  });

  it("announces only the stage and keeps the bar and its counters outside the live region (SCIENCE-24)", () => {
    withState({ status: "running", operation: "segment", outputDir: RUN, progress: { stage: "Running official SAGUI in isolated R - R running 12 s - Clustering", current: 1, total: 5, percent: 20 } }, () => {
      const html = render();
      const live = liveRegion(html);
      expect(live).toContain("Running official SAGUI in isolated R");
      expect(live).not.toContain("R running 12 s");
      expect(live).not.toContain("%");
      expect(live).not.toContain("<progress");
      expect(html).toMatch(/<progress[^>]*aria-label="SAGUI segmentation progress"/);
      expect(html).toContain("20% · R running 12 s - Clustering");
    });
    withState({ status: "running", operation: "segment", outputDir: RUN, progress: { stage: "Measuring correlated regional error: region 2 of 6, F200W", current: 3, total: 5, percent: 60 } }, () => {
      const html = render();
      expect(liveRegion(html)).toContain("Measuring correlated regional error");
      expect(liveRegion(html)).not.toContain("region 2 of 6");
      expect(html).toContain("60% · region 2 of 6, F200W");
    });
    withState({ status: "installing", operation: "install", outputDir: "C:\\R\\install-1", progress: { stage: "Preparing private SAGUI packages and checking exact, starlet and sparse modes - R running 30 s", current: 1, total: 4, percent: 25 } }, () => {
      const html = render();
      expect(html).toMatch(/<progress[^>]*aria-label="SAGUI installation progress"/);
      expect(liveRegion(html)).toContain("Preparing private SAGUI packages and checking exact, starlet and sparse modes");
      expect(liveRegion(html)).not.toContain("R running 30 s");
    });
  });

  it("names every Browse button after its field (SCIENCE-25)", () => {
    const html = render();
    for (const name of ["Browse prepared multiband FITS cube", "Browse Rscript executable", "Browse private R package library", "Browse library installation folder", "Browse working folder"]) expect(html).toContain(`aria-label="${name}"`);
    expect(html.match(/>Browse<\/button>/g)).toHaveLength(5);
  });

  it("lists the run folder and products by name, with the full path on hover and a Show in folder action each (SCIENCE-27)", () => {
    withState({ status: "complete", operation: "segment", result: RESULT, outputDir: RUN, unit: "Jy/pixel" }, () => {
      const html = render();
      for (const path of [RUN, RESULT.labels_path, RESULT.mask_path, RESULT.table_path, RESULT.manifest_path]) {
        const name = path.split("\\").at(-1)!;
        expect(html).toContain(`title="${path}">${name}</span>`);
        expect(html).toContain(`aria-label="Show in folder: ${name}"`);
        expect(html).not.toContain(`>${path}<`);
      }
      expect(html.match(/>Show in folder<\/button>/g)).toHaveLength(5);
      expect(html).not.toContain("Product paths");
      expect(html).not.toContain("Run folder: ");
      for (const action of ["Open segmentation FITS", "Open mask FITS", "Save photometry as…"]) expect(html).toContain(action);
    });
  });

  it("uses the shared run button with a running state for Validate & segment (SCIENCE-29)", () => {
    const idle = runButton(render());
    expect(idle).toContain("Validate &amp; segment");
    expect(idle).toContain('data-accent="violet"');
    expect(idle).toContain('data-running="false"');
    withState({ status: "running", operation: "segment", outputDir: RUN }, () => {
      const running = runButton(render());
      expect(running).toContain('aria-busy="true"');
      expect(running).toContain('data-running="true"');
      expect(running).toContain("Segmenting…");
    });
    withState({ status: "checking", operation: "segment" }, () => expect(runButton(render())).toContain("Checking…"));
    withState({ status: "checking", operation: "install" }, () => {
      const blocked = runButton(render());
      expect(blocked).toContain('data-running="false"');
      expect(blocked).toContain("disabled");
    });
  });

  it("offers the SED scaling as an advanced field with the SAGUI default selected", () => {
    const html = render();
    expect(html).toContain("Advanced clustering");
    expect(html).toContain("SED scaling");
    expect(html).toContain('<option value="median_centre" selected="">Median-centered (SAGUI default)</option>');
    expect(html).toContain('<option value="median_mad">Median-centered, divided by MAD</option>');
    withState({ config: { ...state.config, sed_scaling: "median_mad" } }, () => expect(render()).toContain('<option value="median_mad" selected="">'));
  });

  it("states whether the cube carries NOISCORR, shows its warning and the PSF-size warning, and where memory is checked", () => {
    const warning = "Support of 185 pixels over 20 regions gives 9.2 pixels per region, below the PSF noise-equivalent area of 27.3 pixels (F444W FWHM 0.1388 arcsec at 0.0400 arcsec per pixel); regions smaller than the PSF are not resolved, so request fewer regions";
    withState({ input: INPUT, result: { ...RESULT, warnings: [INPUT.warnings[0], warning] }, outputDir: RUN, status: "complete", operation: "segment" }, () => {
      const html = render();
      expect(html).toContain("ERR present · DQ present · NOISCORR absent");
      expect(html).toContain(INPUT.warnings[0]);
      expect(html).toContain(warning);
      expect(html).not.toContain("The runner checks memory for the selected mode before launching R");
      expect(html).toContain("Before R starts, only the cube and the sparse neighbor graph are checked against free memory; exact Ward checks its own memory in R on the clustered support and refuses more than 65536 pixels.");
    });
    withState({ input: { ...INPUT, has_noise_correlation: true, warnings: [] } }, () => expect(render()).toContain("NOISCORR present"));
  });

  it("explains the default DQ mask per mission", () => {
    expect(render()).toContain("The default 1 excludes DO_NOT_USE on JWST and Roman planes (REED_SOLOMON on HST planes). 4294967295 excludes all flagged bits; 0 ignores DQ flags. Non-finite samples remain excluded.");
  });
});
