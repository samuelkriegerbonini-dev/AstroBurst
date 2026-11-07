import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { current } = vi.hoisted(() => ({
  current: {
    header: {} as Record<string, string>,
    dimensions: undefined as [number, number] | undefined,
    histData: null as { mean: number; median: number; sigma: number } | null,
    compositePreviewUrl: null as string | null,
    compositeDims: null as [number, number] | null,
    linked: true,
  },
}));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({
    file: { id: "f", name: "x.fits", path: "C:/heavy/x.fits", result: { header: current.header, dimensions: current.dimensions } },
  }),
  useHistContext: () => ({ histData: current.histData, stfParams: { shadow: 0.0011, midtone: 0.5, highlight: 1 } }),
  useDoneFilesContext: () => ({ doneFiles: [] }),
  useNarrowbandContext: () => ({ narrowbandPalette: null, narrowbandFilters: [] }),
  useRenderActions: () => ({ forgetOutputs: () => {} }),
}));

vi.mock("../../../context/CompositeContext", async (importOriginal) => {
  const r = { shadow: 0.0123, midtone: 0.2345, highlight: 0.9876 };
  const g = { shadow: 0.0456, midtone: 0.3456, highlight: 0.9765 };
  const b = { shadow: 0.0789, midtone: 0.4567, highlight: 0.9654 };
  return {
    ...(await importOriginal<typeof import("../../../context/CompositeContext")>()),
    useCompositePreview: () => ({
      compositePreviewUrl: current.compositePreviewUrl,
      isShowingComposite: current.compositePreviewUrl !== null,
      compositeVersion: 1,
      canShowParked: false,
    }),
    useCompositeStf: () => ({
      compositeStfR: r,
      compositeStfG: g,
      compositeStfB: b,
      compositeStfLinked: current.linked,
      compositeAutoStfR: null,
      compositeAutoStfG: null,
      compositeAutoStfB: null,
      wizardStfR: r,
      wizardStfG: g,
      wizardStfB: b,
      wizardStfLinked: current.linked,
    }),
  };
});

vi.mock("../../../context/ComposeWizardContext", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../context/ComposeWizardContext")>()),
  useWizardCompositeDims: () => current.compositeDims,
}));

import { MiniInfoBar } from "../ComposeWizard";

function infoBarText(header: Record<string, string>): string {
  current.header = header;
  return renderToStaticMarkup(createElement(MiniInfoBar));
}

function compositeBar(compositeReady: boolean): string {
  return renderToStaticMarkup(createElement(MiniInfoBar, { compositeReady }));
}

function expectFileStats(html: string): void {
  expect(html).not.toContain("Composite");
  expect(html).toContain("S=0.0011");
  expect(html).toContain("mean=1.50");
  expect(html).toContain("100×80");
}

beforeEach(() => {
  current.header = { INSTRUME: "ASI2600", FILTER: "Red" };
  current.dimensions = [100, 80];
  current.histData = { mean: 1.5, median: 1.25, sigma: 0.5 };
  current.compositePreviewUrl = null;
  current.compositeDims = null;
  current.linked = true;
});

describe("Compose wizard info bar filter", () => {
  it("names the NIRCam pupil-wheel filter like the file list chip", () => {
    const html = infoBarText({ TELESCOP: "JWST", INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "F470N" });
    expect(html).toContain("JWST NIRCAM F470N");
    expect(html).not.toContain("F444W");
  });

  it("keeps the filter wheel when the pupil slot is empty", () => {
    expect(infoBarText({ TELESCOP: "JWST", INSTRUME: "NIRCAM", FILTER: "F444W", PUPIL: "CLEAR" })).toContain("JWST NIRCAM F444W");
  });

  it("names the WFPC2 filter instead of leaving the filter out", () => {
    const html = infoBarText({ TELESCOP: "HST", INSTRUME: "WFPC2", FILTNAM1: "F656N", FILTER1: "31", FILTER2: "0" });
    expect(html).toContain("HST WFPC2 F656N");
  });

  it("keeps an amateur filter name", () => {
    expect(infoBarText({ INSTRUME: "ASI2600", FILTER: "Red" })).toContain("ASI2600 Red");
  });
});

describe("Compose wizard info bar with the composite on screen", () => {
  it("shows the composite size and its linked STF, not the selected file's", () => {
    current.compositePreviewUrl = "asset://localhost/rgb_composite_1.png";
    current.compositeDims = [4000, 3000];
    const html = compositeBar(true);
    expect(html).toContain("Composite");
    expect(html).toContain("4000×3000");
    expect(html).toContain("S=0.0123");
    expect(html).toContain("M=0.2345");
    expect(html).toContain("H=0.9876");
    expect(html).not.toContain("S=0.0011");
    expect(html).not.toContain("mean=");
    expect(html).not.toContain("100×80");
    expect(html).not.toContain("ASI2600 Red");
  });

  it("says per-channel instead of one channel's STF when the channels are unlinked", () => {
    current.compositePreviewUrl = "asset://localhost/rgb_composite_1.png";
    current.linked = false;
    const html = compositeBar(true);
    expect(html).toContain("Composite");
    expect(html).toContain("per-channel STF");
    expect(html).not.toContain("S=0.0123");
  });

  it("keeps the selected file's stats when the composite is built but not on screen", () => {
    expectFileStats(compositeBar(true));
  });

  it("keeps the selected file's stats when the wizard has not built the composite", () => {
    current.compositePreviewUrl = "asset://localhost/rgb_composite_1.png";
    expectFileStats(compositeBar(false));
  });
});
