import { describe, expect, it } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import DeconvolutionPanel from "../DeconvolutionPanel";
import MaskedStretchPanel from "../MaskedStretchPanel";
import type { ProcessedFile } from "../../../shared/types";

const REASON = "This is an RGB FITS.";

const FILE: ProcessedFile = {
  id: "f1",
  name: "rgb.fits",
  path: "C:/data/rgb.fits",
  sourcePath: "C:/data/rgb.fits",
  imageRef: null,
  size: 1,
  status: "done",
  result: null,
  error: null,
  startedAt: null,
  finishedAt: null,
};

const PANELS = [
  { name: "DeconvolutionPanel", component: DeconvolutionPanel as ComponentType<never> },
  { name: "MaskedStretchPanel", component: MaskedStretchPanel as ComponentType<never> },
];

function render(component: ComponentType<never>, disabledReason: string | null): string {
  const props = { selectedFile: FILE, fileKey: FILE.path, compositeMode: false, compositeInput: null, onCompositeDone: () => {}, fileName: FILE.name, disabledReason };
  const preview = createElement(PreviewProvider, { file: FILE, doneFiles: [], children: createElement(component, props as never) });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function runButtonDisabled(html: string): boolean {
  const match = /class="ab-run-btn[^"]*"[^>]*data-disabled="(true|false)"/.exec(html);
  if (!match) throw new Error("run button not found");
  return match[1] === "true";
}

describe.each(PANELS)("$name disabledReason", ({ component }) => {
  it("disables the run button and explains why", () => {
    const html = render(component, REASON);
    expect(runButtonDisabled(html)).toBe(true);
    expect(html).toContain(`title="${REASON}"`);
  });

  it("keeps the run button enabled without a reason", () => {
    const html = render(component, null);
    expect(runButtonDisabled(html)).toBe(false);
    expect(html).not.toContain(`title="${REASON}"`);
  });
});
