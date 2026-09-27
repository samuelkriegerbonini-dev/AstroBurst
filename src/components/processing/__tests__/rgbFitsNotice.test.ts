import { describe, expect, it } from "vitest";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CompositeProvider } from "../../../context/CompositeContext";
import { PreviewProvider } from "../../../context/PreviewContext";
import BackgroundPanel from "../BackgroundPanel";
import WaveletPanel from "../WaveletPanel";
import PsfPanel from "../PsfPanel";
import ArcsinhStretchPanel from "../ArcsinhStretchPanel";
import LocalContrastPanel from "../LocalContrastPanel";
import HdrPanel from "../HdrPanel";
import PixelMathPanel from "../PixelMathPanel";
import DebayerPanel from "../DebayerPanel";
import { RGB_FITS_NOTICE, rgbFitsDisabledReason } from "../rgbFitsNotice";
import type { ProcessedFile } from "../../../shared/types";

const FILE: ProcessedFile = {
  id: "f1",
  name: "drizzle_rgb.fits",
  path: "C:/data/drizzle_rgb.fits",
  sourcePath: "C:/data/drizzle_rgb.fits",
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
  runnableWithoutReason: boolean;
}

const PANELS: PanelCase[] = [
  { name: "BackgroundPanel", component: BackgroundPanel as ComponentType<never>, runnableWithoutReason: true },
  { name: "WaveletPanel", component: WaveletPanel as ComponentType<never>, runnableWithoutReason: true },
  { name: "PsfPanel", component: PsfPanel as ComponentType<never>, runnableWithoutReason: true },
  { name: "ArcsinhStretchPanel", component: ArcsinhStretchPanel as ComponentType<never>, runnableWithoutReason: true },
  { name: "LocalContrastPanel", component: LocalContrastPanel as ComponentType<never>, runnableWithoutReason: true },
  { name: "HdrPanel", component: HdrPanel as ComponentType<never>, runnableWithoutReason: true },
  { name: "PixelMathPanel", component: PixelMathPanel as ComponentType<never>, runnableWithoutReason: false },
  { name: "DebayerPanel", component: DebayerPanel as ComponentType<never>, runnableWithoutReason: true },
];

function render(component: ComponentType<never>, disabledReason: string | null): string {
  const props = {
    selectedFile: FILE,
    outputDir: "C:/out",
    fileKey: FILE.path,
    compositeMode: false,
    compositeInput: null,
    onCompositeDone: () => {},
    fileName: FILE.name,
    disabledReason,
  };
  const preview = createElement(PreviewProvider, { file: FILE, doneFiles: [FILE], children: createElement(component, props as never) });
  return renderToStaticMarkup(createElement(CompositeProvider, { children: preview }));
}

function runButtonDisabled(html: string): boolean {
  const match = /class="ab-run-btn[^"]*"[^>]*data-disabled="(true|false)"/.exec(html);
  if (!match) throw new Error("run button not found");
  return match[1] === "true";
}

describe("rgbFitsDisabledReason", () => {
  it("refuses the file tools on an RGB FITS outside composite mode", () => {
    expect(rgbFitsDisabledReason({ fileIsRgb: true, compositeMode: false })).toBe(RGB_FITS_NOTICE);
  });

  it("leaves mono files and composite mode alone", () => {
    expect(rgbFitsDisabledReason({ fileIsRgb: false, compositeMode: false })).toBeNull();
    expect(rgbFitsDisabledReason({ fileIsRgb: false, compositeMode: true })).toBeNull();
    expect(rgbFitsDisabledReason({ fileIsRgb: true, compositeMode: true })).toBeNull();
  });

  it("uses the agreed wording", () => {
    expect(RGB_FITS_NOTICE).toBe(
      "This is an RGB FITS. The Processing tools work on mono FITS files or on a colour composite built in Compose; they cannot process the planes of an RGB file yet.",
    );
  });
});

describe.each(PANELS)("$name disabledReason", ({ component, runnableWithoutReason }) => {
  it("disables the run button and gives the reason as its title", () => {
    const html = render(component, RGB_FITS_NOTICE);
    expect(runButtonDisabled(html)).toBe(true);
    expect(html).toContain(`title="${RGB_FITS_NOTICE}"`);
  });

  it("keeps today's run button without a reason", () => {
    const html = render(component, null);
    expect(runButtonDisabled(html)).toBe(!runnableWithoutReason);
    expect(html).not.toContain(RGB_FITS_NOTICE);
  });
});
