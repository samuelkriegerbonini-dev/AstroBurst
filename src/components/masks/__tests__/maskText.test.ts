import { describe, expect, it } from "vitest";
import { deletedNotice, formatCoverage, maskLabel, maskResultLine, recipeSummary, regionsSummary } from "../maskText";
import type { HistoryDoc, HistoryNode } from "../../../shared/types/ops";

const DOC = {
  nodes: [
    { id: "n0002", op: { op: "arcsinh", factor: 40 } },
    { id: "n0004", op: { op: "tone" } },
  ] as unknown as HistoryNode[],
} as Pick<HistoryDoc, "nodes">;

describe("mask copy", () => {
  it("mask_text_formats_coverage_without_rounding_a_sliver_to_zero", () => {
    expect(formatCoverage(0)).toBe("0%");
    expect(formatCoverage(0.00004)).toBe("< 0.01%");
    expect(formatCoverage(0.0342)).toBe("3.42%");
    expect(formatCoverage(1)).toBe("100.00%");
  });

  it("mask_text_names_every_history_step_that_keeps_a_deleted_mask", () => {
    expect(deletedNotice({ mask_id: "m03", recipe: { kind: "luminance" }, used_by: ["n0002", "n0004", "n0009"], kept_for_replay: true }, DOC)).toBe(
      "m03 is no longer listed. n0002 (Arcsinh Stretch), n0004 (Curves) and n0009 still use it, so its recipe stays in the history for replay.",
    );
    expect(deletedNotice({ mask_id: "m03", recipe: { kind: "luminance" }, used_by: [], kept_for_replay: false }, DOC)).toBe("Deleted m03. No History step used it.");
    expect(deletedNotice({ mask_id: "m01", recipe: { kind: "luminance" }, used_by: [], kept_for_replay: false }, null)).toBe("Deleted m01.");
  });

  it("mask_text_labels_a_mask_by_id_and_recipe_for_the_run_selectors", () => {
    expect(maskLabel({ id: "m02", recipe: { kind: "file", path: "D:\\masks\\cloud.fits", mark: "range", lo: 1, hi: 5, feather_px: 2.5, invert: false } })).toBe(
      "m02 · File cloud.fits, range 1 to 5, feather 2.5 px",
    );
    expect(recipeSummary({ kind: "stars", sigma: 4, config: { luminance_protect: true }, feather_px: 0, invert: true })).toBe("Stars at 4σ, luminance protection, inverted");
    expect(recipeSummary({ kind: "regions", shapes: [{ shape: "circle", x: 1, y: 1, r: 1 }], exclude_shapes: [] })).toBe("Regions, 1 included");
    expect(maskResultLine({ mask_id: "m07", kind: "stars", coverage: 0.031, dims: [2048, 2048], stars: 1, recipe: { kind: "stars" } })).toBe(
      "m07 · Stars · coverage 3.10% · 2048×2048 · 1 star",
    );
    expect(regionsSummary(2, 0, 2)).toBe(
      "2 included regions and 0 excluded regions from Image › Regions. Excluded regions cut holes in the mask. 2 line or point regions have no area and are left out.",
    );
  });
});
