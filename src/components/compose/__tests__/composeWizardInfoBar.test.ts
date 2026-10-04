import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { current } = vi.hoisted(() => ({
  current: { header: {} as Record<string, string> },
}));

vi.mock("../../../context/PreviewContext", () => ({
  useFileContext: () => ({ file: { id: "f", name: "x.fits", path: "C:/heavy/x.fits", result: { header: current.header } } }),
  useHistContext: () => ({ histData: null, stfParams: { shadow: 0, midtone: 0.5, highlight: 1 } }),
  useDoneFilesContext: () => ({ doneFiles: [] }),
  useNarrowbandContext: () => ({ narrowbandPalette: null, narrowbandFilters: [] }),
  useRenderActions: () => ({ forgetOutputs: () => {} }),
}));

import { MiniInfoBar } from "../ComposeWizard";

function infoBarText(header: Record<string, string>): string {
  current.header = header;
  return renderToStaticMarkup(createElement(MiniInfoBar));
}

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
