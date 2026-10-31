import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf } from "../../../context/PreviewContext";
import ArcsinhStretchPanel from "../ArcsinhStretchPanel";
import type { ProcessedFile } from "../../../shared/types";

const FILE: ProcessedFile = {
  id: "arcsinh-slider",
  name: "673nmos.fits",
  path: "C:/data/673nmos.fits",
  sourcePath: "C:/data/673nmos.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: { png_path: "C:/out/a.png", previewUrl: "asset://a.png", dimensions: [1600, 1600], elapsed_ms: 1, is_rgb: false },
  error: null,
  startedAt: null,
  finishedAt: null,
};

function render(): string {
  const props = { selectedFile: FILE, outputDir: "C:/out", fileKey: fileKeyOf(FILE), compositeMode: false, compositeInput: null, onCompositeDone: () => {}, fileName: FILE.name };
  const preview = createElement(PreviewProvider, { file: FILE, doneFiles: [FILE], children: createElement(ArcsinhStretchPanel, props) });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function rangeAttributes(html: string): Record<string, string> {
  const tag = /<input[^>]*type="range"[^>]*>/.exec(html)?.[0];
  if (!tag) throw new Error("no range input rendered");
  return Object.fromEntries([...tag.matchAll(/([\w-]+)="([^"]*)"/g)].map(([, name, value]) => [name, value]));
}

describe("Arcsinh stretch factor slider", () => {
  it("puts the factor on the slider's own log axis, so the edit box holds the factor itself", () => {
    const range = rangeAttributes(render());
    expect(range.min).toBe("0");
    expect(range.max).toBe("1");
    expect(Math.abs(Number(range.value) - Math.log(50) / Math.log(500))).toBeLessThan(1e-6);
    expect(range["aria-valuetext"]).toBe("50.0");
  });

  it("shows the factor on the value button and in the run label", () => {
    const html = render();
    expect(html).toMatch(/aria-label="Edit Stretch Factor \(S\) value"[^>]*>50\.0<\/button>/);
    expect(html).toContain("Apply Stretch (S=50)");
  });
});
