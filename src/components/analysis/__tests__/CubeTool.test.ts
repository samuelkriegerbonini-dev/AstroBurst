import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ANALYSIS_SECTION, CUBE_TOOL_EMPTY_TITLE } from "../../../utils/analysisSections";
import type { RampInfo } from "../../../shared/types/ramp";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";
import type { OpenSampleFrames, SampleDataStore, SampleEntry } from "../../../hooks/useSampleData";

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
const samples = vi.hoisted(() => ({
  entries: [] as Record<string, unknown>[],
  opens: [] as unknown[],
  started: [] as string[],
}));

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
  samples.entries = [];
  samples.opens = [];
  samples.started = [];
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
