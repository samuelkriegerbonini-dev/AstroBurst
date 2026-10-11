import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../shared/types/fits.types";

vi.mock("../../../context/PreviewContext", () => ({
  useDoneFilesContext: () => ({ doneFiles: [] }),
}));
vi.mock("../../../services/cosmetic", () => ({ cosmeticCorrect: vi.fn(), cosmeticCorrectBatch: vi.fn(), cosmeticDryRun: vi.fn() }));

import CosmeticPanel, { DryRunPreview } from "../CosmeticPanel";
import type { CosmeticDryRunResult } from "../../../services/cosmetic";

function frame(name: string, header: Record<string, string>): ProcessedFile {
  return {
    id: name,
    name,
    path: `C:/r6/a1/${name}`,
    sourcePath: `C:/r6/a1/${name}`,
    imageRef: null,
    size: 0,
    status: "done",
    result: { header } as ProcessedFile["result"],
    error: null,
    startedAt: null,
    finishedAt: null,
  };
}

function render(selectedFile: ProcessedFile | null): string {
  return renderToStaticMarkup(createElement(CosmeticPanel, { selectedFile }));
}

function badgeText(html: string): string | null {
  return /<[^>]*data-testid="cosmetic-cfa-badge"[^>]*>([^<]*)</.exec(html)?.[1] ?? null;
}

function cfaToggleChecked(html: string): string | null {
  const tag = /<button[^>]*aria-label="CFA \(Bayer\) data"[^>]*>/.exec(html)?.[0];
  return tag ? /aria-checked="(true|false)"/.exec(tag)?.[1] ?? null : null;
}

describe("CosmeticPanel CFA toggle", () => {
  it("is checked and names the pattern when the selected frame carries BAYERPAT", () => {
    const html = render(frame("cfa_rggb_12.fits", { BAYERPAT: "'RGGB'" }));
    expect(badgeText(html)).toBe("RGGB from header");
    expect(cfaToggleChecked(html)).toBe("true");
  });

  it("is unchecked and says so when the frame has no Bayer card", () => {
    const html = render(frame("mono_12.fits", { INSTRUME: "ASI2600MM" }));
    expect(badgeText(html)).toBe("no Bayer card");
    expect(cfaToggleChecked(html)).toBe("false");
  });

  it("is unchecked with no frame selected", () => {
    const html = render(null);
    expect(badgeText(html)).toBe("no Bayer card");
    expect(cfaToggleChecked(html)).toBe("false");
  });
});

describe("CosmeticPanel dry run", () => {
  it("offers a Dry run button next to the correction", () => {
    const html = render(frame("light_120s.fits", {}));
    expect(html).toMatch(/<button[^>]*>(?:(?!<\/button>).)*Dry run(?:(?!<\/button>).)*<\/button>/);
  });

  it("shows the defect preview and says which pixels would be replaced", () => {
    const result: CosmeticDryRunResult = {
      dry_run: true,
      path: "C:/r7/b3/cosmetic_stars_hot.fits",
      png_path: "C:/out/cosmetic_stars_hot_defects.png",
      previewUrl: "asset://localhost/C%3A/out/cosmetic_stars_hot_defects.png?v=1",
      dimensions: [256, 256],
      counts: { flagged: 93, replaced: 0, hot: 88, cold: 5, listed: 0 },
      defects: [[10, 20, "hot"]],
      defects_total: 93,
      dq_present: false,
      warning: null,
      cfa_applied: false,
      cfa_source: "none",
      elapsed_ms: 12,
    };
    const html = renderToStaticMarkup(createElement(DryRunPreview, { result }));
    expect(html).toContain('src="asset://localhost/C%3A/out/cosmetic_stars_hot_defects.png?v=1"');
    expect(html).toMatch(/<img[^>]*data-testid="cosmetic-dry-run-preview"/);
    expect(html).toContain("Red marks: pixels that would be replaced (88 hot, 5 cold)");
    const listed = renderToStaticMarkup(
      createElement(DryRunPreview, { result: { ...result, counts: { ...result.counts, listed: 2 } } }),
    );
    expect(listed).toContain("Red marks: pixels that would be replaced (88 hot, 5 cold, 2 listed)");
  });
});
