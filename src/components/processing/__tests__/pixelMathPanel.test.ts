import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider, fileKeyOf, useRenderActions } from "../../../context/PreviewContext";
import { EMPTY_CHAIN, withStep } from "../../../utils/processingChain";
import type { ChainStep, ProcessedResult } from "../../../shared/types/preview";
import type { ProcessedFile } from "../../../shared/types/fits.types";
import PixelMathPanel, { PixelMathRunOn } from "../PixelMathPanel";
import { initialPixelMathTarget, pixelMathTargetAfterKey } from "../pixelMathTarget";
import DebayerPanel from "../DebayerPanel";
import { chainOutputs, pixelMathFileOptions, retain } from "../pixelMathPanelState";
import { RESERVED_NAMES } from "../../../utils/pixelmathSlots";

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

function renderPixelMath(compositeMode: boolean, chainedFrom?: string): string {
  const panel = createElement(PixelMathPanel, {
    selectedFile: FILE,
    outputDir: "C:/out",
    fileKey: "m31_L",
    chainedFrom,
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

function radioOptions(html: string): { text: string; checked: string | undefined; tabIndex: string | undefined }[] {
  return [...html.matchAll(/<button([^>]*role="radio"[^>]*)>(.*?)<\/button>/g)].map((m) => ({
    text: m[2].replace(/<[^>]*>/g, ""),
    checked: /aria-checked="([^"]*)"/.exec(m[1])?.[1],
    tabIndex: /tabindex="([^"]*)"/.exec(m[1])?.[1],
  }));
}

function attributeOf(tag: string, name: string): string | undefined {
  return new RegExp(`${name}="([^"]*)"`).exec(tag)?.[1];
}

function renderRunOn(target: "file" | "composite", fileName: string): string {
  return renderToStaticMarkup(createElement(PixelMathRunOn, { target, fileName, disabled: false, onChange: NOOP }));
}

describe("PixelMathPanel target selector", () => {
  it("is hidden in file mode, with no composite notice and no output name: the history names the step", () => {
    const html = renderPixelMath(false);
    expect(html).not.toContain("Composite on screen");
    expect(html).not.toContain('role="radiogroup"');
    expect(html).not.toContain("the composite on screen does not change");
    expect(html).not.toContain("Output name");
    expect(html).toContain("$T = <span");
    expect(html).toContain("m31_L.fits</span>");
  });

  it("starts on the composite while one is on screen, and on the file otherwise", () => {
    expect(initialPixelMathTarget(true)).toBe("composite");
    expect(initialPixelMathTarget(false)).toBe("file");
  });

  it("is a labelled Run on radiogroup in composite mode with the composite checked and the composite notice", () => {
    const html = renderPixelMath(true);
    expect(radioOptions(html)).toEqual([
      { text: "Composite on screen (per channel)", checked: "true", tabIndex: "0" },
      { text: "File m31_L.fits", checked: "false", tabIndex: "-1" },
    ]);
    const group = /<div[^>]*role="radiogroup"[^>]*>/.exec(html)?.[0] ?? "";
    const labelId = attributeOf(group, "aria-labelledby");
    const noteId = attributeOf(group, "aria-describedby");
    expect(html).toMatch(new RegExp(`<span[^>]*id="${labelId}"[^>]*>Run on</span>`));
    expect(html).toMatch(new RegExp(`<div[^>]*id="${noteId}"[^>]*role="note"[^>]*>Composite mode: processes the three channels of the colour composite; m31_L.fits is not changed.</div>`));
    expect(html).not.toContain("the composite on screen does not change");
    expect(html).not.toContain(">Target<");
    expect(html).toContain("one channel at a time");
  });

  it("keeps File one click away, with the file-only notice when it is chosen", () => {
    const html = renderRunOn("file", "m31_L.fits");
    expect(radioOptions(html)).toEqual([
      { text: "Composite on screen (per channel)", checked: "false", tabIndex: "-1" },
      { text: "File m31_L.fits", checked: "true", tabIndex: "0" },
    ]);
    expect(html).toContain("Runs on m31_L.fits; the composite on screen does not change.");
    expect(html).not.toContain("Composite mode: processes");
  });

  it("names the file option File alone when no file is selected", () => {
    const html = renderRunOn("file", "");
    expect(radioOptions(html).map((o) => o.text)).toEqual(["Composite on screen (per channel)", "File"]);
    expect(html).toContain("Runs on the selected file; the composite on screen does not change.");
  });

  it("moves the choice with the arrow keys, wrapping at both ends, and ignores other keys", () => {
    expect(pixelMathTargetAfterKey("composite", "ArrowDown")).toBe("file");
    expect(pixelMathTargetAfterKey("composite", "ArrowRight")).toBe("file");
    expect(pixelMathTargetAfterKey("file", "ArrowDown")).toBe("composite");
    expect(pixelMathTargetAfterKey("file", "ArrowUp")).toBe("composite");
    expect(pixelMathTargetAfterKey("composite", "ArrowUp")).toBe("file");
    expect(pixelMathTargetAfterKey("composite", "ArrowLeft")).toBe("file");
    for (const key of ["Enter", " ", "Tab", "a"]) {
      expect(pixelMathTargetAfterKey("composite", key)).toBeNull();
    }
  });
});

describe("PixelMathPanel reference text", () => {
  it("lists every function name in a collapsed Functions disclosure next to the editor", () => {
    const html = renderPixelMath(false);
    const details = /<details([^>]*)>(.*?)<\/details>/.exec(html);
    expect(details).not.toBeNull();
    expect(details?.[1] ?? "").not.toContain("open");
    expect(details?.[2] ?? "").toMatch(/^<summary[^>]*>Functions<\/summary>/);
    const names = [...(details?.[2] ?? "").matchAll(/<code[^>]*>([^<]*)<\/code>/g)].map((m) => m[1]);
    expect(names).toContain("med");
    expect(names).toContain("mtf");
    expect(names).toEqual([...RESERVED_NAMES]);
    expect(html.indexOf("<details")).toBeGreaterThan(html.indexOf("<textarea"));
    expect(html.indexOf("<details")).toBeLessThan(html.indexOf("Image slots"));
  });

  it("explains Truncate and Rescale together right under the two toggles", () => {
    const html = renderPixelMath(false);
    const hint = html.indexOf("Both on: rescale first; Truncate then only maps infinities to 0 or 1");
    expect(hint).toBeGreaterThan(html.indexOf("Rescale result to [0, 1]"));
    expect(hint).toBeLessThan(html.indexOf("Run PixelMath"));
  });

  it("joins the chained-input notice with a colon, with no em dash in the panel", () => {
    const html = renderPixelMath(false, "Background");
    expect(html).toContain("result is not part of this expression: bind it to a slot if you want it.");
    expect(html).not.toContain("—");
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
    retain(key, { expression: "$T - A", slots: [{ name: "A", path: BG_PATH }], truncate: false, rescale: false, slotsTouched: true });
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

describe("pixelMathFileOptions", () => {
  const target = "C:/data/502nmos.fits";
  const other = "C:/data/656nmos.fits";
  const bg = "C:/out/502nmos_bg_corrected.fits";
  const tone = "C:/out/502nmos_curves.fits";
  const loaded = [
    { path: target, name: "502nmos.fits" },
    { path: other, name: "656nmos.fits" },
  ];

  it("offers chain outputs that are not loaded as '<step> output' after the loaded files", () => {
    const chain = withStep(withStep(EMPTY_CHAIN, "background", { fitsPath: bg, previewUrl: null, dimensions: null }), "tone", { fitsPath: tone, previewUrl: null, dimensions: null });
    expect(pixelMathFileOptions(target, loaded, [], chainOutputs(chain))).toEqual([
      { value: target, label: "502nmos.fits ($T)" },
      { value: other, label: "656nmos.fits" },
      { value: bg, label: "Background output" },
      { value: tone, label: "Curves output" },
    ]);
  });

  it("lists every path once, whatever its spelling, and keeps slot paths nothing else offers", () => {
    const chain = withStep(EMPTY_CHAIN, "background", { fitsPath: "c:\\OUT\\502NMOS_bg_corrected.fits", previewUrl: null, dimensions: null });
    const options = pixelMathFileOptions(target, [...loaded, { path: bg, name: "502nmos_bg_corrected.fits" }], [{ path: other }, { path: "C:/x/elsewhere.fits" }], chainOutputs(chain));
    expect(options.map((o) => o.label)).toEqual(["502nmos.fits ($T)", "656nmos.fits", "502nmos_bg_corrected.fits", "elsewhere.fits"]);
  });

  it("has no target option without a target and no chain options for an empty chain", () => {
    expect(pixelMathFileOptions(null, loaded, [], chainOutputs(EMPTY_CHAIN))).toEqual([
      { value: target, label: "502nmos.fits" },
      { value: other, label: "656nmos.fits" },
    ]);
  });
});

describe("PixelMathPanel slot image options with chain outputs", () => {
  it("offers the Background output of the file on screen in the slot picker", () => {
    const file = loadedFile("pm-slot-chain-output");
    const denoised = "C:/out/f444w_i2d_bg_denoised.fits";
    seedRecords(file, [
      ["background", shownResult("processing", BG_PATH, RAW_PATH, "Background")],
      ["denoise", shownResult("processing", denoised, BG_PATH, "Denoise")],
    ]);
    const key = fileKeyOf(file);
    retain(key, { expression: "$T - A", slots: [{ name: "A", path: RAW_PATH }], truncate: false, rescale: false, slotsTouched: true });
    const panel = createElement(PixelMathPanel, {
      selectedFile: file,
      outputDir: "C:/out",
      fileKey: key,
      compositeMode: false,
      compositeInput: null,
      onCompositeDone: NOOP,
      fileName: file.name,
    });
    const preview = createElement(PreviewProvider, { file, doneFiles: [file], children: panel });
    const html = renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
    const slotSelect = /<select[^>]*aria-label="Slot 1 image"[^>]*>(.*?)<\/select>/.exec(html)?.[1] ?? "";
    const options = [...slotSelect.matchAll(/<option value="([^"]*)"[^>]*>([^<]*)<\/option>/g)].map((m) => [m[1], m[2]]);
    expect(options).toEqual([
      [denoised, "f444w_i2d_bg_denoised.fits ($T)"],
      [RAW_PATH, "f444w_i2d.fits"],
      [BG_PATH, "Background output"],
    ]);
  });
});
