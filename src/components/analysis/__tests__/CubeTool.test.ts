import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ANALYSIS_SECTION } from "../../../utils/analysisSections";
import type { RampInfo } from "../../../shared/types/ramp";

const cubeState = vi.hoisted(() => ({ isCube: true, ramp: null as unknown }));

vi.mock("../../../context/PreviewContext", () => ({
  fileKeyOf: () => "file-key",
  useFileContext: () => ({ file: { path: "C:/data/cube.fits" } }),
  useCubeContext: () => ({
    isCube: cubeState.isCube,
    cubeDims: null,
    ramp: cubeState.ramp,
    rampSource: cubeState.ramp ? "fits" : null,
  }),
  useRenderActions: () => ({ publishProcessed: () => {} }),
}));
vi.mock("../../../hooks/useSpectrumStore", () => ({
  useSpectrum: () => ({ spectrum: [], wavelengths: null, coord: null, loading: false, elapsed: 0, error: null }),
}));
vi.mock("../../../infrastructure/tauri", () => ({ getPreviewUrl: async () => "" }));
vi.mock("../SpectroscopyPanel", () => ({ default: () => "[spectroscopy-panel]" }));
vi.mock("../PvPanel", () => ({ default: () => "[pv-panel]" }));
vi.mock("../RampPanel", () => ({ default: () => "[ramp-panel]" }));

import CubeTool from "../CubeTool";

const RAMP: RampInfo = {
  kind: "jwst_groups",
  nints: 1,
  ngroups: 10,
  nframes: 1,
  groupgap: 0,
  tframe_s: 14.58889,
  tgroup_s: 14.589,
  tgroup_source: "tgroup",
  group_times_s: null,
  readpatt: "NRSIRS2RAPID",
  instrument: "NIRSPEC",
  detector: "NRS1",
  exp_type: "NRS_IFU",
  datamodl: "Level1bModel",
  irs2: null,
  frame_width: 2048,
  frame_height: 3200,
};

async function renderCubeTool(ramp: RampInfo | null): Promise<string> {
  cubeState.isCube = true;
  cubeState.ramp = ramp;
  renderToStaticMarkup(createElement(CubeTool));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return renderToStaticMarkup(createElement(CubeTool));
}

const sectionMarkup = (id: string) => `id="${id}"`;

describe("CubeTool sections", () => {
  it("drops the PV section for a ramp and keeps Ramp and Spectrum", async () => {
    const html = await renderCubeTool(RAMP);
    expect(html).toContain(sectionMarkup(ANALYSIS_SECTION.ramp.id));
    expect(html).toContain(sectionMarkup(ANALYSIS_SECTION.spectrum.id));
    expect(html).toContain("[spectroscopy-panel]");
    expect(html).not.toContain(sectionMarkup(ANALYSIS_SECTION.pv.id));
    expect(html).not.toContain("[pv-panel]");
  });

  it("keeps Spectrum and PV for a cube that is not a ramp", async () => {
    const html = await renderCubeTool(null);
    expect(html).toContain(sectionMarkup(ANALYSIS_SECTION.spectrum.id));
    expect(html).toContain(sectionMarkup(ANALYSIS_SECTION.pv.id));
    expect(html).toContain("[pv-panel]");
    expect(html).not.toContain(sectionMarkup(ANALYSIS_SECTION.ramp.id));
  });
});
