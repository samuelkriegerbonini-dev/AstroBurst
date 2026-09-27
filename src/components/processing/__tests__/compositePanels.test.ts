import { describe, expect, it } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import BackgroundPanel from "../BackgroundPanel";
import WaveletPanel from "../WaveletPanel";
import PsfPanel from "../PsfPanel";
import DeconvolutionPanel from "../DeconvolutionPanel";
import { compositeModeNotice, fileOnlyNotice, unchangedFileClause } from "../compositeProps";
import type { CompositeInputView, CompositePanelProps } from "../compositeProps";
import type { ProcessedFile } from "../../../shared/types";

const FILE_NAME = "656nmos.fits";
const COMPOSITE_INPUT: CompositeInputView = { input: "base", previewUrl: "asset://out/blend.png", label: "Composite" };

const FILE: ProcessedFile = {
  id: "f1",
  name: FILE_NAME,
  path: "C:/data/656nmos.fits",
  sourcePath: "C:/data/656nmos.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: null,
  error: null,
  startedAt: null,
  finishedAt: null,
};

interface PanelCase {
  name: string;
  component: ComponentType<never>;
  hint: string;
}

const PANELS: PanelCase[] = [
  { name: "BackgroundPanel", component: BackgroundPanel as ComponentType<never>, hint: "Select a FITS file to enable background extraction." },
  { name: "WaveletPanel", component: WaveletPanel as ComponentType<never>, hint: "Select a FITS file to enable noise reduction." },
  { name: "PsfPanel", component: PsfPanel as ComponentType<never>, hint: "Select a FITS file to estimate PSF." },
  { name: "DeconvolutionPanel", component: DeconvolutionPanel as ComponentType<never>, hint: "Select a FITS file to enable deconvolution." },
];

function render(panel: PanelCase, selectedFile: ProcessedFile | null, composite: Partial<CompositePanelProps>): string {
  const props = {
    selectedFile,
    fileKey: selectedFile?.path ?? null,
    compositeMode: false,
    compositeInput: null,
    onCompositeDone: () => {},
    fileName: selectedFile?.name ?? "",
    ...composite,
  };
  const element = createElement(panel.component, props as never);
  const preview = createElement(PreviewProvider, { file: selectedFile, doneFiles: [], children: element });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function runButtonDisabled(html: string): boolean {
  const match = /class="ab-run-btn[^"]*"[^>]*data-disabled="(true|false)"/.exec(html);
  if (!match) throw new Error("run button not found");
  return match[1] === "true";
}

const NOTICE = compositeModeNotice(FILE_NAME);

describe.each(PANELS)("$name composite mode", (panel) => {
  it("shows the composite notice and enables the run button without a selected file", () => {
    const html = render(panel, null, { compositeMode: true, compositeInput: COMPOSITE_INPUT, fileName: FILE_NAME });
    expect(html).toContain(NOTICE);
    expect(html).not.toContain(panel.hint);
    expect(runButtonDisabled(html)).toBe(false);
  });

  it("disables the run button when the composite input is missing", () => {
    const html = render(panel, FILE, { compositeMode: true, compositeInput: null, fileName: FILE_NAME });
    expect(runButtonDisabled(html)).toBe(true);
  });

  it("keeps the file-mode hint and gating without a composite", () => {
    const html = render(panel, null, {});
    expect(html).toContain(panel.hint);
    expect(html).not.toContain("Composite mode:");
    expect(runButtonDisabled(html)).toBe(true);
  });

  it("enables the run button in file mode with a selected file", () => {
    const html = render(panel, FILE, {});
    expect(html).not.toContain("Composite mode:");
    expect(runButtonDisabled(html)).toBe(false);
  });
});

describe("composite notices without a selected file", () => {
  it("drop the file clause instead of naming an empty file", () => {
    expect(compositeModeNotice("")).toBe("Composite mode: processes the three channels of the colour composite.");
    expect(compositeModeNotice(FILE_NAME)).toBe(`Composite mode: processes the three channels of the colour composite; ${FILE_NAME} is not changed.`);
    expect(fileOnlyNotice("Runs on", "")).toBe("Runs on the selected file; the composite on screen does not change.");
    expect(unchangedFileClause("")).toBe(".");
    expect(unchangedFileClause(FILE_NAME)).toBe(`; ${FILE_NAME} is not changed.`);
  });

  it("render in the panels without a dangling semicolon", () => {
    for (const panel of PANELS) {
      const html = render(panel, null, { compositeMode: true, compositeInput: COMPOSITE_INPUT, fileName: "" });
      expect(html).toContain("Composite mode: processes the three channels of the colour composite.");
      expect(html).not.toContain(";  is not changed");
    }
  });
});
