import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import type { ProcessedFile } from "../../../shared/types/fits.types";
import PixelMathPanel from "../PixelMathPanel";
import DebayerPanel from "../DebayerPanel";

const FILE: ProcessedFile = {
  id: "1",
  name: "m31_L.fits",
  path: "C:/data/m31_L.fits",
  sourcePath: "C:/data/m31_L.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: null,
  error: null,
  startedAt: null,
  finishedAt: null,
};

const NOOP = () => {};

function renderPixelMath(compositeMode: boolean): string {
  const panel = createElement(PixelMathPanel, {
    selectedFile: FILE,
    outputDir: "C:/out",
    fileKey: "m31_L",
    compositeMode,
    compositeInput: compositeMode ? { input: "base", previewUrl: "asset://blend.png", label: "Composite" } : null,
    onCompositeDone: NOOP,
    fileName: FILE.name,
  });
  const preview = createElement(PreviewProvider, { file: FILE, doneFiles: [FILE], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function renderDebayer(compositeMode: boolean): string {
  const panel = createElement(DebayerPanel, {
    selectedFile: FILE,
    outputDir: "C:/out",
    fileKey: "m31_L",
    compositeMode,
    fileName: FILE.name,
  });
  const preview = createElement(PreviewProvider, { file: FILE, doneFiles: [FILE], children: panel });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

describe("PixelMathPanel target selector", () => {
  it("is hidden in file mode, with no composite notice", () => {
    const html = renderPixelMath(false);
    expect(html).not.toContain("Composite (per channel)");
    expect(html).not.toContain("the composite on screen does not change");
    expect(html).toContain("Output name");
  });

  it("appears in composite mode with File selected by default and the file-only notice", () => {
    const html = renderPixelMath(true);
    expect(html).toContain("Composite (per channel)");
    expect(html).toContain('<option value="file"');
    expect(html).toContain("Runs on m31_L.fits; the composite on screen does not change.");
    expect(html).not.toContain("Composite mode: processes the three channels");
    expect(html).toContain("$T = <span");
    expect(html).toContain("m31_L.fits</span>");
  });
});

describe("DebayerPanel composite notice", () => {
  it("names the file only in composite mode", () => {
    expect(renderDebayer(true)).toContain("Debayer acts on m31_L.fits; the composite on screen does not change.");
    expect(renderDebayer(false)).not.toContain("Debayer acts on");
  });
});
