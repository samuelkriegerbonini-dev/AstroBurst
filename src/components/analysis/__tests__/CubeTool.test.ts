import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ANALYSIS_SECTION, CUBE_TOOL_EMPTY_TITLE } from "../../../utils/analysisSections";
import type { RampInfo } from "../../../shared/types/ramp";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";
import type { SpectrumRequest } from "../../../services/spectrum1d";
import type { OpenSampleFrames, SampleDataStore, SampleEntry } from "../../../hooks/useSampleData";

const cubeState = vi.hoisted(() => ({
  isCube: true,
  isSpectrum: false,
  ramp: null as unknown,
  file: { id: "f1", path: "C:/data/cube.fits" } as Record<string, unknown> | null,
  cubeDims: null as unknown,
}));
const specState = vi.hoisted(() => ({
  spectrum: [] as number[],
  err: null as number[] | null,
  coord: null as { x: number; y: number } | null,
}));
const docState = vi.hoisted(() => ({
  doc: { loading: false, spectrum: null as unknown, error: null as string | null },
  requests: [] as (SpectrumRequest | null)[],
  csv: null as { path: string; over: string | null } | null,
}));
const panelProps = vi.hoisted(() => ({ current: null as Record<string, unknown> | null }));
const tsoProps = vi.hoisted(() => ({ current: null as Record<string, unknown> | null }));
const pickerProps = vi.hoisted(() => ({ current: null as Record<string, unknown> | null }));
const samples = vi.hoisted(() => ({
  entries: [] as Record<string, unknown>[],
  opens: [] as unknown[],
  started: [] as string[],
}));
const sectionTips = vi.hoisted(() => ({
  calls: [] as { sections: readonly { id: string }[]; tips: Readonly<Record<string, { title: string; body: string } | undefined>> | undefined }[],
}));

vi.mock("../useSectionTip", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../useSectionTip")>();
  return {
    ...actual,
    useSectionTip: (...args: Parameters<typeof actual.useSectionTip>) => {
      sectionTips.calls.push({ sections: args[0], tips: args[1] });
      return actual.useSectionTip(...args);
    },
  };
});

vi.mock("../../../context/PreviewContext", () => ({
  fileKeyOf: (file: { id: string; path: string } | null) => (file ? `${file.id}|${file.path}` : null),
  useFileContext: () => ({ file: cubeState.file }),
  useCubeContext: () => ({
    isCube: cubeState.isCube,
    isSpectrum: cubeState.isSpectrum,
    cubeDims: cubeState.cubeDims,
    ramp: cubeState.ramp,
    rampSource: cubeState.ramp ? "fits" : null,
  }),
  useRenderActions: () => ({ publishProcessed: () => {} }),
}));
vi.mock("../../../hooks/useSpectrumStore", () => ({
  useSpectrum: () => ({
    spectrum: specState.spectrum,
    err: specState.err,
    wavelengths: null,
    coord: specState.coord,
    loading: false,
    elapsed: 0,
    error: null,
    region: null,
  }),
}));
vi.mock("../../../hooks/useSpectrum1dStore", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../hooks/useSpectrum1dStore")>()),
  useSpectrum1dRequest: (request: SpectrumRequest | null) => {
    docState.requests.push(request);
    return request === null ? { loading: false, spectrum: null, error: null } : docState.doc;
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
vi.mock("../SpectrumSourcePicker", () => ({
  default: (props: Record<string, unknown>) => {
    pickerProps.current = props;
    return "[source-picker]";
  },
}));
vi.mock("../PvPanel", () => ({ default: () => "[pv-panel]" }));
vi.mock("../../../hooks/useSampleData", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../hooks/useSampleData")>();
  const entries = () => samples.entries as unknown as SampleEntry[];
  const store: SampleDataStore = { ...actual.sampleDataStore, subscribe: () => () => {}, get: entries };
  const sampleActions: typeof actual.sampleActions = (_store, id, onOpen) => {
    samples.opens.push(onOpen);
    return actual.sampleActions(store, id, onOpen);
  };
  return {
    ...actual,
    sampleDataStore: store,
    sampleActions,
    useSampleData: (onOpen: OpenSampleFrames, id: string) => {
      samples.started.push(id);
      return actual.sampleControls(entries().find((e) => e.id === id), sampleActions(store, id, onOpen));
    },
  };
});
vi.mock("../RampPanel", () => ({ default: () => "[ramp-panel]" }));
vi.mock("../TsoLightCurvesPanel", () => ({
  default: (props: Record<string, unknown>) => {
    tsoProps.current = props;
    return "[tso-panel]";
  },
}));

import CubeTool from "../CubeTool";
import { spectrumSelectionStore } from "../../../hooks/useActiveSpectrumDocument";

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
const X1DINTS_PATH = "C:/data/jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048_x1dints.fits";

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

const fileRequest = (path: string, hdu: number | null): SpectrumRequest => ({ kind: "file", path, hdu, wavelengthUnit: null });

function findElement(node: ReactNode, match: (props: Record<string, unknown>) => boolean): ReactElement<Record<string, unknown>> | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findElement(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Record<string, unknown>>(node)) return null;
  if (match(node.props)) return node;
  return findElement(node.props.children as ReactNode, match);
}

function documentPanelKey(): string | null {
  let tree: ReactNode = null;
  const readTree = () => {
    tree = (CubeTool as unknown as { type: () => ReactNode }).type();
    return null;
  };
  renderToStaticMarkup(createElement(readTree));
  return findElement(tree, (props) => props.documentMode === true)?.key ?? null;
}

beforeEach(() => {
  spectrumSelectionStore.reset();
  cubeState.isCube = true;
  cubeState.isSpectrum = false;
  cubeState.ramp = null;
  cubeState.file = { id: "f1", path: "C:/data/cube.fits" };
  cubeState.cubeDims = null;
  specState.spectrum = [];
  specState.err = null;
  specState.coord = null;
  docState.doc = { loading: false, spectrum: null, error: null };
  docState.requests = [];
  docState.csv = null;
  panelProps.current = null;
  tsoProps.current = null;
  pickerProps.current = null;
  samples.entries = [];
  samples.opens = [];
  samples.started = [];
  sectionTips.calls = [];
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

describe("CubeTool section guidance", () => {
  const SPECTRUM_ID = ANALYSIS_SECTION.spectrum.id;
  const lastCall = () => sectionTips.calls.at(-1);
  const spectrumTip = () => lastCall()?.tips?.[SPECTRUM_ID];
  const insideSpectrumSection = (html: string, marker: string) => {
    const at = html.indexOf(sectionMarkup(SPECTRUM_ID));
    const end = html.indexOf("</section>", at);
    const found = html.indexOf(marker, at);
    return at >= 0 && found > at && found < end;
  };

  it("gives a cube Spectrum and PV chips with the cube extraction steps in the Spectrum section", async () => {
    const html = await renderCubeTool(null);
    expect(html).toContain('aria-label="Spectra panels"');
    expect(html).toContain(`data-section-help="${SPECTRUM_ID}"`);
    expect(html).toContain('aria-label="Help: Measure a spectral line"');
    expect(insideSpectrumSection(html, `data-section-tip="${SPECTRUM_ID}"`)).toBe(true);
    expect(lastCall()?.sections.map((s) => s.id)).toEqual([SPECTRUM_ID, ANALYSIS_SECTION.pv.id]);
    expect(spectrumTip()?.title).toBe("Measure a spectral line");
    expect(spectrumTip()?.body).toContain("click the image to extract a spectrum");
    expect(spectrumTip()?.body).toContain("Measure line");
  });

  it("gives a ramp its Ramp and Spectrum chips and no guidance", async () => {
    const html = await renderCubeTool(RAMP);
    expect(html).toContain('aria-label="Spectra panels"');
    expect(lastCall()?.sections.map((s) => s.id)).toEqual([ANALYSIS_SECTION.ramp.id, SPECTRUM_ID]);
    expect(lastCall()?.tips).toBeUndefined();
    expect(html).not.toContain("data-section-help");
    expect(html).not.toContain("data-section-tip");
  });

  it("gives a 1D spectrum document the line steps without the cube extraction step, with its help in the section", async () => {
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const html = await render();
    expect(html).not.toContain('aria-label="Spectra panels"');
    expect(lastCall()?.sections.map((s) => s.id)).toEqual([SPECTRUM_ID]);
    expect(insideSpectrumSection(html, `data-section-help="${SPECTRUM_ID}"`)).toBe(true);
    expect(spectrumTip()?.title).toBe("Measure a spectral line");
    expect(spectrumTip()?.body).toContain("Drag across the plot");
    expect(spectrumTip()?.body).not.toContain("Crosshair");
    expect(spectrumTip()?.body).not.toContain("click the image");
  });

  it("gives time-series spectra a Light curves section after the spectrum document", async () => {
    const TSO_ID = ANALYSIS_SECTION.tso.id;
    const insideTsoSection = (html: string, marker: string) => {
      const at = html.indexOf(sectionMarkup(TSO_ID));
      const found = html.indexOf(marker, at);
      return at >= 0 && found > at && found < html.indexOf("</section>", at);
    };
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = {
      id: "x2",
      path: X1DINTS_PATH,
      status: "spectrum",
      spectrum: { kind: "x1dints", layout: "rows", n_series: 1, n_integrations: 240, n_channels: 431 },
    };
    const html = await render();
    expect(lastCall()?.sections.map((s) => s.id)).toEqual([SPECTRUM_ID, TSO_ID]);
    expect(insideTsoSection(html, "[tso-panel]")).toBe(true);
    expect(html.indexOf(sectionMarkup(SPECTRUM_ID))).toBeLessThan(html.indexOf(sectionMarkup(TSO_ID)));
    expect(tsoProps.current).toEqual({ filePath: X1DINTS_PATH, fileKey: `x2|${X1DINTS_PATH}` });
    expect(docState.requests.every((request) => request === null)).toBe(true);

    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
    const x1d = await render();
    expect(lastCall()?.sections.map((s) => s.id)).toEqual([SPECTRUM_ID]);
    expect(x1d).not.toContain(sectionMarkup(TSO_ID));
  });

  it("gives an open CSV spectrum the same document guidance", async () => {
    cubeState.isCube = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    docState.csv = { path: "C:/data/ha_spectrum.csv", over: "other|C:/data/o.fits" };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const html = await render();
    expect(lastCall()?.sections.map((s) => s.id)).toEqual([SPECTRUM_ID]);
    expect(insideSpectrumSection(html, `data-section-help="${SPECTRUM_ID}"`)).toBe(true);
    expect(spectrumTip()?.body).not.toContain("click the image");
  });

  it("offers no section chips or guidance when the Cube tool has nothing to show", async () => {
    cubeState.isCube = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    const html = await render();
    expect(lastCall()?.sections).toEqual([]);
    expect(html).not.toContain('aria-label="Spectra panels"');
    expect(html).not.toContain("data-section-help");
  });

  it("tells the line steps with the rest line before the measurement", async () => {
    const lineSteps =
      "Drag across the plot to select a line range, set continuum windows A and B beside the line, click a rest-line marker or type the rest wavelength to get velocities, choose a model, then click Measure line.";
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    await render();
    const body = spectrumTip()?.body ?? "";
    expect(body).toBe(lineSteps);
    expect(body).toContain("Drag across the plot");
    expect(body).toContain("Measure line");
    expect(body.indexOf("rest-line marker")).toBeGreaterThanOrEqual(0);
    expect(body.indexOf("rest-line marker")).toBeLessThan(body.indexOf("Measure line"));

    await renderCubeTool(null);
    expect(spectrumTip()?.body).toContain("click the image to extract a spectrum");
    expect(spectrumTip()?.body).toContain(lineSteps);
  });
});

describe("CubeTool SED", () => {
  const plane = (filter: string, pivot_um: number) => ({
    filter,
    dataset: "JWST",
    instrument: "NIRCam",
    pivot_um,
    pixscale_arcsec: 0.04,
    bunit_in: "Jy",
    fluxconv: 1,
    psf_fwhm_arcsec: null,
    kernel: "",
  });
  const photometricDims = {
    width: 6,
    height: 5,
    frames: 3,
    bunit: "Jy",
    photometric_planes: [plane("F444W", 4.404315), plane("F150W", 1.500744), plane("F277W", 2.761)],
    spectral_classification: { is_spectral: false, is_photometric: false, reason: "NAXIS3=3 with no spectral keywords: likely RGB/RGBA composition" },
  };
  const count = (html: string, attribute: string) => html.split(attribute).length - 1;

  it("plots the picked pixel and its error against the pivot wavelengths of a filter cube", async () => {
    cubeState.cubeDims = photometricDims;
    specState.spectrum = [3, 1, 2];
    specState.err = [0.3, 0.1, 0.2];
    specState.coord = { x: 2, y: 3 };
    const html = await renderCubeTool(null);
    expect(html).toContain("data-sed-panel");
    expect(html).toContain("pixel (2, 3)");
    expect(html).toContain("Pivot wavelength (um)");
    expect(html).toContain("Flux (Jy)");
    expect(count(html, "data-sed-point")).toBe(3);
    expect(count(html, "data-sed-err")).toBe(3);
    expect(html.indexOf("F150W")).toBeLessThan(html.indexOf("F277W"));
    expect(html.indexOf("F277W")).toBeLessThan(html.indexOf("F444W"));
  });

  it("asks for a pixel or a region before it has values to plot", async () => {
    cubeState.cubeDims = photometricDims;
    const html = await renderCubeTool(null);
    expect(html).toContain("data-sed-panel");
    expect(html).toContain("Click a pixel or extract a region spectrum");
    expect(count(html, "data-sed-point")).toBe(0);
  });

  it("wraps the SED of a filter cube in its own section between Spectrum and PV", async () => {
    cubeState.cubeDims = photometricDims;
    const html = await renderCubeTool(null);
    expect(sectionTips.calls.at(-1)?.sections.map((s) => s.id)).toEqual([ANALYSIS_SECTION.spectrum.id, ANALYSIS_SECTION.sed.id, ANALYSIS_SECTION.pv.id]);
    const at = html.indexOf(sectionMarkup(ANALYSIS_SECTION.sed.id));
    expect(at).toBeGreaterThanOrEqual(0);
    expect(html.slice(at, html.indexOf("</section>", at))).toContain("data-sed-panel");
  });

  it("gives the SED of a filter cube its own tip", async () => {
    const SED_ID = ANALYSIS_SECTION.sed.id;
    cubeState.cubeDims = photometricDims;
    const html = await renderCubeTool(null);
    const tip = sectionTips.calls.at(-1)?.tips?.[SED_ID];
    expect(tip?.title).toBe("Plot the SED of a filter cube");
    expect(tip?.body).toContain("each filter plane with a pivot wavelength gives one point of the SED");
    expect(html).toContain(`data-section-help="${SED_ID}"`);
    expect(html).toContain('aria-label="Help: Plot the SED of a filter cube"');
    const at = html.indexOf(sectionMarkup(SED_ID));
    expect(at).toBeGreaterThanOrEqual(0);
    const section = html.slice(at, html.indexOf("</section>", at));
    const tipAt = section.indexOf(`data-section-tip="${SED_ID}"`);
    expect(tipAt).toBeGreaterThanOrEqual(0);
    expect(tipAt).toBeLessThan(section.indexOf("data-sed-panel"));
  });

  it("leaves the SED out of a cube without filter planes", async () => {
    cubeState.cubeDims = { width: 6, height: 5, frames: 3, photometric_planes: null };
    specState.spectrum = [1, 2, 3];
    specState.coord = { x: 1, y: 1 };
    const html = await renderCubeTool(null);
    expect(html).not.toContain("data-sed-panel");
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
    expect(docState.requests).toContainEqual(fileRequest(X1D_PATH, 1));
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
    expect(docState.requests).toContainEqual(fileRequest("C:/data/ha_spectrum.csv", null));
    expect(panelProps.current).toMatchObject({
      documentMode: true,
      filePath: "C:/data/ha_spectrum.csv",
      spectrumDocument: { source: { kind: "file", hdu: null } },
    });
  });

  it("passes the document's view key to the panel", async () => {
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    await render();
    expect(panelProps.current?.viewKey).toBe("1|" + X1D_PATH);

    cubeState.isSpectrum = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    docState.csv = { path: "C:/data/ha_spectrum.csv", over: "other|C:/data/o.fits" };
    await render();
    expect(panelProps.current?.viewKey).toBe("|C:/data/ha_spectrum.csv");

    docState.csv = null;
    cubeState.file = { id: "f1", path: "C:/data/cube.fits" };
    await renderCubeTool(null);
    expect(panelProps.current).toMatchObject({ filePath: "C:/data/cube.fits" });
    expect(panelProps.current?.documentMode).toBeFalsy();
    expect(panelProps.current?.viewKey).toBeUndefined();
  });

  it("asks for no document while a cube is shown", async () => {
    await renderCubeTool(null);
    expect(docState.requests.every((request) => request === null)).toBe(true);
    expect(panelProps.current?.documentMode).toBeFalsy();
  });

  it("remounts the panel when the chosen table or pick changes", async () => {
    const path = "C:/data/jw01345-o063_t001_nirspec_g395m-f290lp_x1d.fits";
    const fileKey = `m1|${path}`;
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "m1", path, status: "spectrum", spectrum: { kind: "x1d", hdu: 2, n_tables: 3 } };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    const afterPanelKey = (key: string | null) => key?.slice(key.indexOf("|spectrum|"));
    const probeTable = documentPanelKey();
    expect(afterPanelKey(probeTable)).toBe(`|spectrum|2|${path}`);

    spectrumSelectionStore.set(fileKey, { kind: "table", hdu: 3, wavelengthUnit: null });
    const otherTable = documentPanelKey();
    expect(afterPanelKey(otherTable)).toBe(`|spectrum|3|${path}`);

    spectrumSelectionStore.set(fileKey, { kind: "table", hdu: 3, wavelengthUnit: "nm" });
    const statedUnit = documentPanelKey();
    expect(afterPanelKey(statedUnit)).toBe(`|spectrum|3@nm|${path}`);
    expect(new Set([probeTable, otherTable, statedUnit]).size).toBe(3);

    await render();
    expect(panelProps.current?.viewKey).toBe("3@nm|" + path);
    expect(docState.requests).toContainEqual({ kind: "file", path, hdu: 3, wavelengthUnit: "nm" });
  });

  it("shows the Light curves section for a time-series file only", async () => {
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = {
      id: "x2",
      path: X1DINTS_PATH,
      status: "spectrum",
      spectrum: { kind: "x1dints", layout: "rows", n_series: 1, n_integrations: 240, n_channels: 431 },
    };
    const series = await render();
    expect(series).toContain(sectionMarkup(ANALYSIS_SECTION.tso.id));
    expect(panelProps.current).toMatchObject({ documentMode: true, spectrumDocument: null, isLoading: true, error: null });
    expect(docState.requests.every((request) => request === null)).toBe(true);

    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1, n_tables: 1 } };
    const single = await render();
    expect(single).not.toContain(sectionMarkup(ANALYSIS_SECTION.tso.id));
    expect(single).toContain(sectionMarkup(ANALYSIS_SECTION.spectrum.id));
  });

  it("puts the picker above the panel of a spectrum file", async () => {
    const pickerBeforePanel = (html: string) => {
      const section = html.indexOf(sectionMarkup(ANALYSIS_SECTION.spectrum.id));
      const picker = html.indexOf("[source-picker]");
      const panel = html.indexOf("[spectroscopy-panel]");
      return section >= 0 && section < picker && picker < panel;
    };
    cubeState.isCube = false;
    cubeState.isSpectrum = true;
    cubeState.file = { id: "x1", path: X1D_PATH, status: "spectrum", spectrum: { kind: "x1d", hdu: 1 } };
    docState.doc = { loading: false, spectrum: SPECTRUM, error: null };
    expect(pickerBeforePanel(await render())).toBe(true);
    expect(pickerProps.current?.active).toMatchObject({ content: "spectrum", path: X1D_PATH, kind: { kind: "x1d", hdu: 1 } });

    cubeState.isSpectrum = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    docState.csv = { path: "C:/data/ha_spectrum.csv", over: "other|C:/data/o.fits" };
    expect(pickerBeforePanel(await render())).toBe(true);
    expect(pickerProps.current?.active).toMatchObject({ content: "csv", path: "C:/data/ha_spectrum.csv" });

    docState.csv = null;
    cubeState.file = { id: "f1", path: "C:/data/cube.fits" };
    const cube = await renderCubeTool(null);
    expect(cube).toContain("[spectroscopy-panel]");
    expect(cube).not.toContain("[source-picker]");
  });
});

const IFU_ID = "jwst-gc-nirspec-ifu";
const LRS_ID = "jwst-miri-lrs";
const IFU_CREDIT = "JWST/NIRSpec IFU cube and 1-D spectrum of the Galactic Center (GC-NIRSPEC-1): NASA, ESA, CSA, GTO programme 1266 (PI M. Garcia Marin), data via MAST at STScI";
const LRS_CREDIT = "JWST/MIRI LRS spectrum of SN 2021aefx: NASA, ESA, CSA, GO programme 2072 (PI S. W. Jha), data via MAST at STScI";
const LICENCE = "Public JWST archive data; acknowledgement requested by STScI";
const IFU_SHOWS = "Cube: spectra with error bands, moment maps, line fits and line measurement, next to the pipeline's 1-D spectrum";

function offer(id: string, over: Record<string, unknown> = {}): Record<string, unknown> {
  const ifu = id === IFU_ID;
  return {
    id,
    title: ifu ? "JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum" : "JWST/MIRI LRS spectrum of SN 2021aefx",
    credit: ifu ? IFU_CREDIT : LRS_CREDIT,
    licence: LICENCE,
    demonstrates: ifu ? IFU_SHOWS : "",
    total_bytes: ifu ? 175_680_000 : 35_164_800,
    file_count: ifu ? 2 : 3,
    state: "absent",
    received_bytes: 0,
    folder: `C:/Users/me/AstroBurst/samples/${id}`,
    file: null,
    error: null,
    ...over,
  };
}

function listSets(...sets: Record<string, unknown>[]): void {
  samples.entries = sets;
}

function offerBlock(html: string, id: string): string {
  const at = html.indexOf(`data-cube-sample="${id}"`);
  if (at < 0) return "";
  const next = html.indexOf("data-cube-sample=", at + 1);
  return html.slice(at, next < 0 ? undefined : next);
}

describe("CubeTool sample sets (UX-34b)", () => {
  it("offers the NIRSpec IFU and MIRI LRS sample sets with their size, credit and licence when the Cube tool has nothing to show", async () => {
    cubeState.isCube = false;
    cubeState.file = { id: "img", path: "C:/data/m31.fits" };
    listSets(offer(IFU_ID), offer(LRS_ID));
    const html = await render();
    expect(html).toContain(CUBE_TOOL_EMPTY_TITLE);
    const ifu = offerBlock(html, IFU_ID);
    const lrs = offerBlock(html, LRS_ID);
    expect(ifu).toContain("JWST/NIRSpec IFU cube of the Galactic Center");
    expect(ifu).toContain("Download sample data (176 MB)");
    expect(ifu).toContain(IFU_CREDIT);
    expect(ifu).toContain(LICENCE);
    expect(ifu).toContain(IFU_SHOWS.replace("'", "&#x27;"));
    expect(lrs).toContain("Download sample data (35 MB)");
    expect(lrs).toContain(LRS_CREDIT);
    expect(ifu).toMatch(/data-cube-sample-terms[^>]*>Used under the MAST data use terms \(https:\/\/archive\.stsci\.edu\/publishing\/data-use\); publications acknowledge JWST program #1266 and STScI\.</);
    expect(lrs).toMatch(/data-cube-sample-terms[^>]*>Used under the MAST data use terms [^<]*JWST program #2072 and STScI\.</);
    expect(html.indexOf(`data-cube-sample="${IFU_ID}"`)).toBeLessThan(html.indexOf(`data-cube-sample="${LRS_ID}"`));
  });

  it("opens a kept set and resumes a partial one", async () => {
    cubeState.isCube = false;
    listSets(offer(IFU_ID, { state: "ready", received_bytes: 175_680_000 }), offer(LRS_ID, { state: "partial", received_bytes: 12_000_000 }));
    const html = await render();
    expect(offerBlock(html, IFU_ID)).toContain("Open sample data");
    expect(offerBlock(html, LRS_ID)).toContain("Resume sample download (12 MB of 35 MB)");
  });

  it("shows a failed download with Retry next to its error", async () => {
    cubeState.isCube = false;
    listSets(offer(IFU_ID, { state: "partial", received_bytes: 1_000, error: "the sample host answered 404" }));
    const block = offerBlock(await render(), IFU_ID);
    expect(block).toContain("Retry sample download");
    expect(block).toMatch(/role="alert"[^>]*>the sample host answered 404</);
  });

  it("keeps a running sample download visible with its bar and Cancel beside a cube, and hides the idle set", async () => {
    listSets(offer(IFU_ID, { state: "downloading", received_bytes: 52_000_000 }), offer(LRS_ID));
    const html = await renderCubeTool(null);
    expect(html).toContain("[spectroscopy-panel]");
    const ifu = offerBlock(html, IFU_ID);
    expect(ifu).toMatch(/role="progressbar"[^>]*aria-valuenow="29"/);
    expect(ifu).toContain("Cancel");
    expect(offerBlock(html, LRS_ID)).toBe("");
  });

  it("keeps a failed sample download visible with its error and Retry beside a cube", async () => {
    listSets(offer(IFU_ID, { state: "partial", received_bytes: 1_000, error: "the sample host answered 404" }), offer(LRS_ID));
    const html = await renderCubeTool(null);
    const ifu = offerBlock(html, IFU_ID);
    expect(ifu).toMatch(/role="alert"[^>]*>the sample host answered 404</);
    expect(ifu).toContain("Retry sample download");
    expect(offerBlock(html, LRS_ID)).toBe("");
  });

  it("offers no set beside a cube when nothing downloads, nor outside the desktop app where no set is listed", async () => {
    listSets(offer(IFU_ID), offer(LRS_ID));
    expect(await renderCubeTool(null)).not.toContain("data-cube-sample=");
    cubeState.isCube = false;
    listSets();
    const html = await render();
    expect(html).not.toContain("data-cube-sample=");
    expect(html).toContain("Open spectrum (CSV)");
  });

  it("opens a set's files through the sample opener that shows its cube or spectrum", async () => {
    const { openSampleProducts } = await import("../../../utils/sampleOffers");
    cubeState.isCube = false;
    listSets(offer(IFU_ID), offer(LRS_ID));
    await render();
    expect(samples.opens.length).toBeGreaterThan(0);
    expect(samples.opens.every((onOpen) => onOpen === openSampleProducts)).toBe(true);
  });

  it("reads the sets from the store App starts, without starting the store again for each offer", async () => {
    cubeState.isCube = false;
    listSets(offer(IFU_ID), offer(LRS_ID));
    const html = await render();
    expect(offerBlock(html, IFU_ID)).toContain("Download sample data (176 MB)");
    expect(offerBlock(html, LRS_ID)).toContain("Download sample data (35 MB)");
    expect(samples.started).toEqual([]);
  });
});
