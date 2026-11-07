import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { createCubePrepDraft } from "../cubePrepModel";
import type { CubePrepState } from "../cubePrepStore";
import { DOCK_TOOL_META, DEFAULT_DOCK_LAYOUT } from "../../../utils/dockLayout";

const state: CubePrepState = { draft: createCubePrepDraft(), status: "idle", validation: null, progress: null, report: null, outputDir: null, error: null, startedAt: null };
vi.mock("../cubePrepStore", () => ({ useCubePrepState: () => state, cubePrepStore: { edit: vi.fn(), validate: vi.fn(), run: vi.fn(), cancel: vi.fn(), isJobActive: () => true } }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getFiles: () => [] } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
import CubePrepPanel from "../CubePrepPanel";

const render = () => renderToStaticMarkup(createElement(CubePrepPanel));
const liveRegion = (html: string) => /<div aria-live="polite"[^>]*>(.*?)<\/div>/.exec(html)?.[1] ?? "";
function withState(change: Partial<CubePrepState>, check: () => void) {
  const saved = { ...state };
  Object.assign(state, change);
  try { check(); } finally { Object.assign(state, saved); }
}

describe("Cube Prep desktop panel", () => {
  it("is available without an open file and exposes the entire recipe workflow", () => {
    expect(DOCK_TOOL_META.cubeprep.needsFile).toBe(false);
    expect(DEFAULT_DOCK_LAYOUT.anchors["right-top"]).toContain("cubeprep");
    const html = render();
    for (const label of ["Inputs", "Crop", "Units", "Align", "Cube", "Edit", "PSFs", "Kernels", "Match", "Export", "LIGHTSTACK 0.4.0", "Scientific MEF", "Validate &amp; run", "Working folder"]) expect(html).toContain(label);
  });

  it("shows retained output and explicit cancellation without a success claim", () => {
    withState({ status: "cancelled", outputDir: "/data/cubeprep-partial" }, () => {
      const html = render();
      expect(html).toContain("Cancelled");
      expect(html).toContain("partial");
      expect(html).not.toContain("Run completed");
    });
  });

  it("says when a cancelled run never created its folder", () => {
    withState({ status: "cancelled", outputDir: null }, () => {
      const html = render();
      expect(html).toContain("Cancelled before the run started; no run folder was created.");
      expect(html).not.toContain("partial products remain");
    });
  });

  it("words the stage in the live region and keeps the bar and counters outside it (SCIENCE-07, SCIENCE-24)", () => {
    const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
    try {
      withState({ status: "running", startedAt: 1_000_000 - 65_000, progress: { stage: "01_crop", current: 0, total: 3, percent: 12 } }, () => {
        const html = render();
        const live = liveRegion(html);
        expect(live).toContain("Step 1 of 3: Crop");
        expect(live).not.toContain("<progress");
        expect(live).not.toContain("%");
        expect(html).toMatch(/<progress[^>]*aria-label="Cube preparation progress"/);
        expect(html).toContain("12% · 1m 5s elapsed");
      });
      withState({ status: "running", startedAt: 1_000_000 - 4_000, progress: { stage: "hashing_inputs", current: 3, total: 12, percent: 4 } }, () => {
        const html = render();
        expect(liveRegion(html)).toContain("Hashing inputs");
        expect(html).toContain("4% · 3 of 12 files · 4s elapsed");
      });
      withState({ status: "running", startedAt: 1_000_000, progress: { stage: "publishing", current: 3, total: 3, percent: 100 } }, () => {
        expect(liveRegion(render())).toContain("Publishing");
      });
    } finally { now.mockRestore(); }
  });

  it("disables Validate while the recipe has local problems (SCIENCE-17)", () => {
    const html = render();
    expect(html).toContain("Add at least one input image.");
    expect(html).toMatch(/<button[^>]*disabled=""[^>]*>Validate<\/button>/);
    const draft = createCubePrepDraft();
    draft.recipe.inputs = [{ path: "a.fits", dataset: "JWST", filter: "F200W" }];
    draft.recipe.steps = [{ step: "align", reference: "F200W" }, { step: "cube", output: "cube.fits" }];
    withState({ draft }, () => expect(render()).toMatch(/<button class="[^"]*">Validate<\/button>/));
  });

  it("explains what Cancel keeps (SCIENCE-19)", () => {
    withState({ status: "running" }, () => {
      const html = render();
      const note = /<p id="([^"]+)"[^>]*>Stops after the current step; partial products stay in the run folder\.<\/p>/.exec(html);
      expect(note).not.toBeNull();
      const id = note![1].replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      expect(html).toMatch(new RegExp(`<button[^>]*aria-describedby="${id}"[^>]*>Cancel</button>`));
    });
  });

  it("names the folder Browse buttons (SCIENCE-25)", () => {
    const html = render();
    expect(html).toContain('aria-label="Browse input folder"');
    expect(html).toContain('aria-label="Browse working folder"');
  });

  it("marks enabled steps without reading the check glyph aloud (SCIENCE-26)", () => {
    const html = render();
    expect(html.match(/<span aria-hidden="true"> ✓<\/span><span class="sr-only">, enabled<\/span>/g)).toHaveLength(3);
    expect(html).not.toMatch(/[^>] ✓/);
  });

  it("shows the run folder and final cube by name, with the full path on hover and Show in folder (SCIENCE-27)", () => {
    const folder = "C:\\data\\cubeprep-field-20261106";
    const report = { recipe_version: 1, name: "field", output_dir: folder, cube: "cube.fits", steps: [], files: [{ path: "cube.fits", bytes: 1_000 }], warnings: [], elapsed_ms: 1 };
    withState({ status: "complete", outputDir: folder, report }, () => {
      const html = render();
      expect(html).toContain(">Run folder<");
      expect(html).toContain(`title="${folder}">cubeprep-field-20261106</span>`);
      expect(html).toContain(">Final cube<");
      expect(html).toContain(`title="${folder}\\cube.fits">cube.fits</span>`);
      expect(html).toContain('aria-label="Show in folder: cubeprep-field-20261106"');
      expect(html).toContain('aria-label="Show in folder: cube.fits"');
      expect(html).not.toContain(`Run folder: ${folder}`);
      expect(html).not.toContain("Show run folder");
    });
  });

  it("uses the shared run button with a running state for Validate & run (SCIENCE-29)", () => {
    const runButton = (html: string) => /<button[^>]*class="ab-run-btn[^"]*"[^>]*>(?:(?!<\/button>).)*<\/button>/.exec(html)?.[0] ?? "";
    const idle = runButton(render());
    expect(idle).toContain("Validate &amp; run");
    expect(idle).toContain('data-accent="cyan"');
    expect(idle).toContain('data-running="false"');
    withState({ status: "validating", startedAt: 1 }, () => {
      const validating = runButton(render());
      expect(validating).toContain('aria-busy="true"');
      expect(validating).toContain("Validating…");
    });
    withState({ status: "running", startedAt: 1 }, () => {
      const running = runButton(render());
      expect(running).toContain('data-running="true"');
      expect(running).toContain("Running…");
      expect(render()).toMatch(/<button[^>]*aria-describedby="[^"]+"[^>]*>Cancel<\/button>/);
    });
  });

  it("shows product sizes in decimal units and counts the checked files (SCIENCE-28)", () => {
    const report = { recipe_version: 1, name: "field", output_dir: "/out", cube: null, steps: [], files: [{ path: "cube.fits", bytes: 12_345_678 }], warnings: [], elapsed_ms: 1 };
    withState({ validation: { recipe: state.draft.recipe, dependencies: ["/a.fits", "/b.fits"], warnings: [] }, report }, () => {
      const html = render();
      expect(html).toContain("Validated recipe · 2 files checked");
      expect(html).toContain("cube.fits · 12.3 MB");
      expect(html).not.toContain(" bytes");
    });
  });
});
