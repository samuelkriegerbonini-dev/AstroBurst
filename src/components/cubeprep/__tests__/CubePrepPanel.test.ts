import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { createCubePrepDraft } from "../cubePrepModel";
import type { CubePrepState } from "../cubePrepStore";
import { DOCK_TOOL_META, DEFAULT_DOCK_LAYOUT } from "../../../utils/dockLayout";

const state: CubePrepState = { draft: createCubePrepDraft(), status: "idle", validation: null, progress: null, report: null, outputDir: null, error: null };
vi.mock("../cubePrepStore", () => ({ useCubePrepState: () => state, cubePrepStore: { edit: vi.fn(), validate: vi.fn(), run: vi.fn(), cancel: vi.fn() } }));
vi.mock("../../../hooks/useFileStore", () => ({ fileStore: { getFiles: () => [] } }));
vi.mock("../../../hooks/useFileIngest", () => ({ showWhenLoaded: vi.fn() }));
import CubePrepPanel from "../CubePrepPanel";

describe("Cube Prep desktop panel", () => {
  it("is available without an open file and exposes the entire recipe workflow", () => {
    expect(DOCK_TOOL_META.cubeprep.needsFile).toBe(false);
    expect(DEFAULT_DOCK_LAYOUT.anchors["right-top"]).toContain("cubeprep");
    const html = renderToStaticMarkup(createElement(CubePrepPanel));
    for (const label of ["Inputs", "Crop", "Units", "Align", "Cube", "Edit", "PSFs", "Kernels", "Match", "Export", "LIGHTSTACK 0.4.0", "Scientific MEF", "Validate &amp; run", "Working folder"]) expect(html).toContain(label);
  });

  it("shows retained output and explicit cancellation without a success claim", () => {
    state.status = "cancelled";
    state.outputDir = "/data/cubeprep-partial";
    const html = renderToStaticMarkup(createElement(CubePrepPanel));
    expect(html).toContain("Cancelled");
    expect(html).toContain("partial");
    expect(html).not.toContain("Run completed");
    state.status = "idle";
    state.outputDir = null;
  });
});
