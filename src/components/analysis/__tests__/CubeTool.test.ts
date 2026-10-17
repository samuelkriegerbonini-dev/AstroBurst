import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ANALYSIS_SECTION, CUBE_TOOL_EMPTY_TITLE } from "../../../utils/analysisSections";
import type { RampInfo } from "../../../shared/types/ramp";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";

const cubeState = vi.hoisted(() => ({
  isCube: true,
  isSpectrum: false,
  ramp: null as unknown,
  file: { id: "f1", path: "C:/data/cube.fits" } as Record<string, unknown> | null,
}));
const docState = vi.hoisted(() => ({
  doc: { loading: false, spectrum: null as unknown, error: null as string | null },
  requests: [] as [string | null, number | null][],
  csv: null as { path: string; over: string | null } | null,
}));
const panelProps = vi.hoisted(() => ({ current: null as Record<string, unknown> | null }));

vi.mock("../../../context/PreviewContext", () => ({
  fileKeyOf: (file: { id: string; path: string } | null) => (file ? `${file.id}|${file.path}` : null),
  useFileContext: () => ({ file: cubeState.file }),
  useCubeContext: () => ({
    isCube: cubeState.isCube,
    isSpectrum: cubeState.isSpectrum,
    cubeDims: null,
    ramp: cubeState.ramp,
    rampSource: cubeState.ramp ? "fits" : null,
  }),
  useRenderActions: () => ({ publishProcessed: () => {} }),
}));
vi.mock("../../../hooks/useSpectrumStore", () => ({
  useSpectrum: () => ({ spectrum: [], wavelengths: null, coord: null, loading: false, elapsed: 0, error: null }),
}));
vi.mock("../../../hooks/useSpectrum1dStore", () => ({
  useSpectrum1dDoc: (path: string | null, hdu: number | null) => {
    docState.requests.push([path, hdu]);
    return path === null ? { loading: false, spectrum: null, error: null } : docState.doc;
  },
  useOpenSpectrumCsv: () => docState.csv,
  openSpectrumCsv: vi.fn(async () => false),
  spectrumCsvStore: { close: vi.fn() },
}));
vi.mock("../../../infrastructure/tauri", () => ({ getPreviewUrl: async () => "" }));
vi.mock("../SpectroscopyPanel", () => ({
  default: (props: Record<string, unknown>) => {
    panelProps.current = props;
    return "[spectroscopy-panel]";
  },
}));
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

const X1D_PATH = "C:/data/jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits";

const SPECTRUM = { source: { kind: "x1d", path: X1D_PATH, hdu: 1, extver: 1 }, flux: [1, 2, 3] } as unknown as Spectrum1d;

async function render(): Promise<string> {
  renderToStaticMarkup(createElement(CubeTool));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return renderToStaticMarkup(createElement(CubeTool));
}

async function renderCubeTool(ramp: RampInfo | null): Promise<string> {
  cubeState.isCube = true;
  cubeState.ramp = ramp;
  return render();
}

const sectionMarkup = (id: string) => `id="${id}"`;

beforeEach(() => {
  cubeState.isCube = true;
  cubeState.isSpectrum = false;
  cubeState.ramp = null;
  cubeState.file = { id: "f1", path: "C:/data/cube.fits" };
  docState.doc = { loading: false, spectrum: null, error: null };
  docState.requests = [];
  docState.csv = null;
  panelProps.current = null;
});

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

describe("CubeTool empty state", () => {
  it("offers Open spectrum (CSV) next to the title when the selected file gives the Cube tool nothing", async () => {
    cubeState.isCube = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    const html = await render();
    expect(html).toContain(CUBE_TOOL_EMPTY_TITLE);
    expect(html).toContain("Open spectrum (CSV)");
    expect(html).not.toContain("[spectroscopy-panel]");
  });

  it("keeps [data-cube-empty] on the title alone and gives the CSV button its own attribute", async () => {
    cubeState.isCube = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    const html = await render();
    expect(html.match(/data-cube-empty(?:="[^"]*")?[^>]*>([^<]*)<\/[a-z]+>/)?.[1]).toBe(CUBE_TOOL_EMPTY_TITLE);
    expect(html).toMatch(/<button[^>]*data-open-spectrum-csv[^>]*>/);
  });

  it("drops the empty title and the CSV button for a cube", async () => {
    const html = await renderCubeTool(null);
    expect(html).not.toContain(CUBE_TOOL_EMPTY_TITLE);
    expect(html).not.toContain("Open spectrum (CSV)");
  });
});

describe("CubeTool spectrum documents", () => {
  it("plots a selected 1D spectrum file in document mode with the HDU the probe found", async () => {
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const html = await render();
    expect(html).not.toContain(CUBE_TOOL_EMPTY_TITLE);
    expect(html).toContain(sectionMarkup(ANALYSIS_SECTION.spectrum.id));
    expect(html).not.toContain(sectionMarkup(ANALYSIS_SECTION.pv.id));
    expect(docState.requests).toContainEqual([X1D_PATH, 1]);
    expect(panelProps.current).toMatchObject({
      documentMode: true,
      filePath: X1D_PATH,
      isLoading: false,
      error: null,
      spectrumDocument: { source: { kind: "file", hdu: 1 }, spectrum: SPECTRUM },
    });
  });

  it("passes the loading state and the backend refusal of a document that is not read yet", async () => {
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "wcs1d", hdu: 0 } };
    docState.doc = { loading: false, spectrum: null, error: "CTYPE1 = 'MULTISPE': IRAF multispec spectra are not supported" };
    await render();
    expect(panelProps.current).toMatchObject({
      documentMode: true,
      spectrumDocument: null,
      isLoading: false,
      error: "CTYPE1 = 'MULTISPE': IRAF multispec spectra are not supported",
    });
  });

  it("counts a spectrum document that is neither read nor refused as loading", async () => {
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
    docState.doc = { loading: false, spectrum: null, error: null };
    await render();
    expect(panelProps.current).toMatchObject({ documentMode: true, spectrumDocument: null, isLoading: true, error: null });
  });

  it("plots an open CSV in document mode with a Close button when nothing else is shown", async () => {
    cubeState.isCube = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    docState.csv = { path: "C:/data/ha_spectrum.csv", over: "other|C:/data/o.fits" };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const html = await render();
    expect(html).not.toContain(CUBE_TOOL_EMPTY_TITLE);
    expect(html).toContain("ha_spectrum.csv");
    expect(html).toContain("Close");
    expect(docState.requests).toContainEqual(["C:/data/ha_spectrum.csv", null]);
    expect(panelProps.current).toMatchObject({
      documentMode: true,
      filePath: "C:/data/ha_spectrum.csv",
      spectrumDocument: { source: { kind: "file", hdu: null } },
    });
  });

  it("asks for no document while a cube is shown", async () => {
    await renderCubeTool(null);
    expect(docState.requests.every(([path]) => path === null)).toBe(true);
    expect(panelProps.current?.documentMode).toBeFalsy();
  });
});
