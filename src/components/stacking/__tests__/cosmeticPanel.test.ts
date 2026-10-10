import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProcessedFile } from "../../../shared/types/fits.types";

vi.mock("../../../context/PreviewContext", () => ({
  useDoneFilesContext: () => ({ doneFiles: [] }),
}));
vi.mock("../../../services/cosmetic", () => ({ cosmeticCorrect: vi.fn(), cosmeticCorrectBatch: vi.fn() }));

import CosmeticPanel from "../CosmeticPanel";

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
