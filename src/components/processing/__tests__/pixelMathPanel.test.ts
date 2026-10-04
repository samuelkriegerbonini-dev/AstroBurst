import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { withStep } from "../../../utils/processingChain";
import type { ChainStep, ProcessedResult } from "../../../shared/types/preview";
import type { ProcessedFile } from "../../../shared/types/fits.types";
import PixelMathPanel from "../PixelMathPanel";
import DebayerPanel from "../DebayerPanel";
import { retain } from "../pixelMathPanelState";

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

const RAW_PATH = "C:/data/f444w_i2d.fits";
const BG_PATH = "C:/out/f444w_i2d_bg.fits";
const PM_PATH = "C:/out/f444w_i2d_bg_pixelmath.fits";

function loadedFile(id: string): ProcessedFile {
  return { ...FILE, id, name: "f444w_i2d.fits", path: RAW_PATH, sourcePath: RAW_PATH };
}

function shownResult(kind: ProcessedResult["kind"], fitsPath: string, inputPath: string, label: string): ProcessedResult {
  return { fitsPath, previewUrl: `asset://${label}.png`, dimensions: [8, 8], label, kind, inputPath };
}

function seedRecords(file: ProcessedFile, records: [ChainStep, ProcessedResult][]): void {
  const key = fileKeyOf(file) as string;
  function Seeder() {
    const { publishProcessed } = useRenderActions();
    for (const [step, result] of records) {
      publishProcessed(key, result, (c) => withStep(c, step, { fitsPath: result.fitsPath as string, previewUrl: result.previewUrl, dimensions: result.dimensions }));
    }
    return null;
  }
  const preview = createElement(PreviewProvider, { file: null, doneFiles: [], children: createElement(Seeder) });
  renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function renderedTarget(file: ProcessedFile): string | undefined {
  const panel = createElement(PixelMathPanel, {
    selectedFile: file,
    outputDir: "C:/out",
    fileKey: fileKeyOf(file),
    compositeMode: false,
    compositeInput: null,
    onCompositeDone: NOOP,
    fileName: file.name,
  });
  const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
  const html = renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
  return /\$T = <span[^>]*>([^<]*)<\/span>/.exec(html)?.[1];
}

describe("PixelMathPanel $T after a run", () => {
  it("binds $T to the input of the PixelMath result on screen, not to the result", () => {
    const file = loadedFile("pm-target-after-run");
    seedRecords(file, [
      ["background", shownResult("processing", BG_PATH, RAW_PATH, "Background")],
      ["pixelMath", shownResult("pixelmath", PM_PATH, BG_PATH, "PixelMath")],
    ]);
    expect(renderedTarget(file)).toBe("f444w_i2d_bg.fits");
  });

  it("binds $T to the displayed chain output when it is not a PixelMath result", () => {
    const file = loadedFile("pm-target-displayed-bg");
    seedRecords(file, [["background", shownResult("processing", BG_PATH, RAW_PATH, "Background")]]);
    expect(renderedTarget(file)).toBe("f444w_i2d_bg.fits");
  });
});

describe("PixelMathPanel slot image options after a run", () => {
  it("names the $T option after $T and still offers the PixelMath result as a loaded file", () => {
    const file = loadedFile("pm-slot-options");
    const resultRow: ProcessedFile = { ...FILE, id: "pm-slot-options-result", name: "f444w_i2d_bg_pixelmath.fits", path: PM_PATH, sourcePath: PM_PATH };
    seedRecords(file, [
      ["background", shownResult("processing", BG_PATH, RAW_PATH, "Background")],
      ["pixelMath", shownResult("pixelmath", PM_PATH, BG_PATH, "PixelMath")],
    ]);
    const key = fileKeyOf(file);
    retain(key, { expression: "$T - A", slots: [{ name: "A", path: BG_PATH }], truncate: false, rescale: false, outputName: "", slotsTouched: true });
    const panel = createElement(PixelMathPanel, {
      selectedFile: file,
      outputDir: "C:/out",
      fileKey: key,
      compositeMode: false,
      compositeInput: null,
      onCompositeDone: NOOP,
      fileName: file.name,
    });
    const preview = createElement(PreviewProvider, { file, doneFiles: [file, resultRow], children: panel });
    const html = renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
    const slotSelect = /<select[^>]*aria-label="Slot 1 image"[^>]*>(.*?)<\/select>/.exec(html)?.[1] ?? "";
    const labels = [...slotSelect.matchAll(/<option[^>]*>([^<]*)<\/option>/g)].map((m) => m[1]);
    expect(labels).toEqual(["f444w_i2d_bg.fits ($T)", "f444w_i2d.fits", "f444w_i2d_bg_pixelmath.fits"]);
  });
});
