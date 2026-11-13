import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { FRAME_KEY_HINT, createFramePublishGate } from "../../../utils/cubeNavigation";
import { framePublishers } from "../../../utils/rampLabels";
import type { CollapseRangeResult, CubeDims, LineFitResult, MomentMapsResult, NoiseFactor, RegionSpectrum } from "../../../shared/types/cube";
import type { Region, RegionShape } from "../../../shared/types/regions";
import { AUTO_WINDOWS_RULE, guardedContinuumWindows } from "../../../utils/lineWindows";
import type { RampInfo } from "../../../shared/types/ramp";
import type { LineMeasurement, SpectralAxisInfo } from "../../../shared/types/spectral";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";
import type { SpectrumDocument } from "../../../utils/spectrumDocument";

const cubeState = vi.hoisted(() => ({ ramp: null as unknown }));
const fileState = vi.hoisted(() => ({ id: "cube-1", imageRef: "C:/data/cube.fits", reloads: 0, finishedAt: 1 }));


const spectralMock = vi.hoisted(() => ({
  measure: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  axis: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
}));
const logMock = vi.hoisted(() => ({ append: vi.fn<(entry: unknown) => void>() }));
const overlayAxes = vi.hoisted(() => ({ calls: [] as number[][] }));
const backend = vi.hoisted(() => ({
  cubeSpectrum: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  pixelToWorld: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  regionSpectrum: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  moments: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  collapseRange: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  fit: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
  inspect: vi.fn<(...args: unknown[]) => Promise<unknown>>(),
}));
const spectrumState = vi.hoisted(() => ({
  region: null as unknown,
  regionSource: null as unknown,
  regionContinuum: null as unknown,
  regionError: null as string | null,
}));
const spectrumActions = vi.hoisted(() => ({ begin: vi.fn(() => 41), commit: vi.fn(), fail: vi.fn(), clear: vi.fn() }));
const regionDocState = vi.hoisted(() => ({ regions: [] as unknown[] }));
const bandCalls = vi.hoisted(() => ({ calls: [] as { y: readonly number[]; err: readonly number[] | null }[] }));
const renderState = vi.hoisted(() => ({ processed: null as unknown }));
const csvInputs = vi.hoisted(() => ({ calls: [] as unknown[] }));
const saveDialog = vi.hoisted(() => ({ save: vi.fn<(...args: unknown[]) => Promise<string | null>>() }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../services/spectral", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/spectral")>()),
  measureSpectralLine: spectralMock.measure,
  getSpectralAxis: spectralMock.axis,
}));
vi.mock("../../../services/cube", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/cube")>()),
  getCubeSpectrum: backend.cubeSpectrum,
  getCubeSpectrumRegion: backend.regionSpectrum,
  computeMomentMaps: backend.moments,
  collapseCubeRange: backend.collapseRange,
  fitCubeLines: backend.fit,
  inspectLineFitSpaxel: backend.inspect,
}));
vi.mock("../../../infrastructure/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../infrastructure/tauri")>()),
  getOutputDir: async () => "C:/out",
}));
vi.mock("../../../utils/regionErrors", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../utils/regionErrors")>();
  return {
    ...actual,
    errorBandRuns: (...args: Parameters<typeof actual.errorBandRuns>) => {
      bandCalls.calls.push({ y: args[1], err: args[2] });
      return actual.errorBandRuns(...args);
    },
  };
});
vi.mock("../../../services/astrometry", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../services/astrometry")>()),
  pixelToWorld: backend.pixelToWorld,
}));
vi.mock("../../../utils/measurementLog", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../utils/measurementLog")>()),
  measurementLog: { append: logMock.append },
}));
vi.mock("../../../utils/lineMeasure", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../utils/lineMeasure")>();
  return {
    ...actual,
    lineOverlayPolylines: (...args: Parameters<typeof actual.lineOverlayPolylines>) => {
      overlayAxes.calls.push(args[1]);
      return actual.lineOverlayPolylines(...args);
    },
  };
});
vi.mock("../../../utils/spectrumExport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../utils/spectrumExport")>();
  return {
    ...actual,
    spectrumCsv: (...args: Parameters<typeof actual.spectrumCsv>) => {
      csvInputs.calls.push(args[0]);
      return actual.spectrumCsv(...args);
    },
    saveCsvDialog: (...args: unknown[]) => saveDialog.save(...args),
  };
});

vi.mock("../../../context/PreviewContext", () => ({
  useCubeContext: () => ({ isCube: true, ramp: cubeState.ramp, rampSource: cubeState.ramp ? "fits" : null }),
  useFileContext: () => ({ file: { ...fileState, path: "C:/data/cube.fits", result: { header: { BUNIT: "DN" } } } }),
  useRenderContext: () => ({ processed: renderState.processed }),
}));
vi.mock("../../../hooks/useSpectrumStore", () => ({
  useSpectrum: () => ({
    region: spectrumState.region,
    regionSource: spectrumState.regionSource,
    regionContinuum: spectrumState.regionContinuum,
    regionLoading: false,
    regionError: spectrumState.regionError,
  }),
  beginRegionSpectrum: spectrumActions.begin,
  clearRegionSpectrum: spectrumActions.clear,
  commitRegionSpectrum: (region: unknown, source: unknown, continuum: unknown = null, request: unknown = undefined) => {
    spectrumActions.commit(region, source, continuum, request);
    spectrumState.region = region;
    spectrumState.regionSource = source;
    spectrumState.regionContinuum = continuum;
  },
  failRegionSpectrum: spectrumActions.fail,
}));
vi.mock("../../../hooks/useRampStore", () => ({ useRampIntegration: () => 0, setIntegration: () => {} }));
vi.mock("../../../hooks/useRegionKey", () => ({ useRegionKey: () => null }));
vi.mock("../../../hooks/useRegionStore", () => ({ useRegionDoc: () => ({ regions: regionDocState.regions, selectedId: null, version: 0 }) }));
vi.mock("../SpectralAxisControls", () => ({ default: () => "[axis-controls]" }));
vi.mock("../LineListControls", () => ({ default: () => "[line-list]" }));
vi.mock("../SpectrumComparisonSection", () => ({ default: () => "[compare]" }));
vi.mock("../LineFitSection", () => ({ default: () => "[line-fit]" }));
vi.mock("../LineMeasurementSection", () => ({ default: () => "[line-measure]" }));
vi.mock("../../CubeFrameNav", () => ({ default: () => "[frame-nav]" }));

import { hooks } from "../../../test/hookRuntime";
import SpectroscopyPanel from "../SpectroscopyPanel";
import SpectrumPlot from "../SpectrumPlot";
import SpectrumViewControls from "../SpectrumViewControls";
import RangeEdgeField from "../RangeEdgeField";
import { SPECTRUM_PLOT_HEIGHT, SPECTRUM_PLOT_PAD, documentPlot, type SpectrumPlotModel } from "../../../utils/spectrumPlotModel";
import { cubeViewKey, documentViewKey, spectrumViewStore } from "../../../hooks/useSpectrumViewStore";
import OutputNote from "../OutputNote";
import StaleResultNote, { ReopenedProductNote } from "../StaleResultNote";
import { fluxUnitLabel } from "../../../utils/spectrumExport";
import { lineFitCubeResult, type LineFitRun } from "../../../utils/lineFit";
import { bandById, pickedBandLabel, type BandMark } from "../../../utils/lineList";
import ExportNotice from "../ExportNotice";
import SpectrumComparisonSection from "../SpectrumComparisonSection";
import spectroscopyPanelSource from "../SpectroscopyPanel.tsx?raw";
import { adHocErrorBoxLines } from "../../../test/panelRules";
import { SMALL_BUTTON_CLASS } from "../../ui";

beforeEach(() => spectrumViewStore.reset());

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

const NON_SPECTRAL_DIMS: CubeDims = {
  width: 2048,
  height: 3200,
  frames: 10,
  spectral_axis: null,
  spectral_classification: { is_spectral: false, reason: "ramp: 10 groups x 1 integrations (NRSIRS2RAPID)" },
};

function renderPanel(ramp: RampInfo | null, spectrum: number[]): string {
  cubeState.ramp = ramp;
  return renderToStaticMarkup(
    createElement(SpectroscopyPanel, {
      spectrum,
      wavelengths: null,
      pixelCoord: spectrum.length > 0 ? { x: 1023, y: 2446 } : null,
      cubeDims: NON_SPECTRAL_DIMS,
      filePath: "C:/data/cube.fits",
      publishGate: createFramePublishGate(),
    }),
  );
}

function renderClickedPixel(ramp: RampInfo | null): string {
  return renderPanel(
    ramp,
    Array.from({ length: 10 }, (_, i) => 100 + i * 5),
  );
}

const SPECTRAL_ONLY_TEXT = ["Continuum", "continuum A", "SNR", "Moments", "[line-fit]", "[line-measure]", "[compare]"];

describe("SpectroscopyPanel spectral-axis widgets", () => {
  it("hides the spectral-axis controls and the line list for a ramp and keeps the group navigator", () => {
    const html = renderClickedPixel(RAMP);
    expect(html).toContain("[frame-nav]");
    expect(html).not.toContain("[axis-controls]");
    expect(html).not.toContain("[line-list]");
  });

  it("keeps the spectral-axis controls and the line list for a cube that is not a ramp", () => {
    const html = renderClickedPixel(null);
    expect(html).toContain("[frame-nav]");
    expect(html).toContain("[axis-controls]");
    expect(html).toContain("[line-list]");
  });
});

describe("SpectroscopyPanel spectral-only sections", () => {
  it("hides the continuum, Moments, Line fit, line measurement and comparison sections for a ramp", () => {
    const html = renderClickedPixel(RAMP);
    for (const text of SPECTRAL_ONLY_TEXT) expect(html).not.toContain(text);
    expect(html).toContain("Collapse Mean");
    expect(html).toContain("Collapse range");
  });

  it("hides the comparison section for a ramp before any spectrum is extracted", () => {
    expect(renderPanel(RAMP, [])).not.toContain("[compare]");
    expect(renderPanel(null, [])).toContain("[compare]");
  });

  it("keeps every spectral section for a cube that is not a ramp", () => {
    const html = renderClickedPixel(null);
    for (const text of SPECTRAL_ONLY_TEXT) expect(html).toContain(text);
  });
});

describe("SpectroscopyPanel channel and SNR boxes", () => {
  it("renders the range, continuum and SNR boxes as decimal text fields, none invalid before an edit", () => {
    const html = renderClickedPixel(null);
    expect(html).not.toContain('type="number"');
    expect(html.match(/inputMode="decimal"/g)?.length).toBe(7);
    expect(html).not.toContain("aria-invalid");
  });

  it("keeps only the line range boxes on a ramp", () => {
    const html = renderClickedPixel(RAMP);
    expect(html.match(/inputMode="decimal"/g)?.length).toBe(2);
    expect(html).toContain('aria-label="Range first channel z0"');
    expect(html).toContain('aria-label="Range last channel z1"');
  });
});

describe("SpectroscopyPanel grey text", () => {
  it("colours the idle export buttons through the text token instead of a grey literal", () => {
    const html = renderClickedPixel(null);
    expect(html).not.toContain("#71717a");
    expect(html).toContain("color:var(--ab-text-3)");
  });
});

const DOC_PATH = "C:/data/ha_bess.fits";
const N_DOC = 40;
const AIR_VALUES = Array.from({ length: N_DOC }, (_, i) => 0.6555 + i * 1e-4);
const DOC_VIEW_KEY = documentViewKey(DOC_PATH, "0");
const CUBE_VIEW_KEY = cubeViewKey("C:/data/cube.fits");

function airAxis(kind: SpectralAxisInfo["kind"] = "awav"): SpectralAxisInfo {
  return {
    kind,
    ctype: "WAVE",
    unit: "um",
    header_unit: "Angstrom",
    header_scale: 1e-4,
    values: AIR_VALUES,
    crval: AIR_VALUES[0],
    cdelt: 1e-4,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
  };
}

const SPECTRAL_CUBE_DIMS: CubeDims = {
  width: 8,
  height: 8,
  frames: N_DOC,
  spectral_axis: airAxis("wave"),
  spectral_classification: { is_spectral: true, reason: "WAVE axis" },
};

function documentOf(extra: Partial<Spectrum1d> = {}): SpectrumDocument {
  return {
    source: { kind: "file", hdu: 0 },
    spectrum: {
      source: { kind: "wcs1d", path: DOC_PATH, hdu: 0 },
      axis: airAxis(),
      wavelength_frame: "air",
      precision: "f64",
      flux: AIR_VALUES.map((_, i) => 1 + Math.exp(-((i - 20) ** 2) / 8)),
      err: AIR_VALUES.map(() => 0.05),
      dq: null,
      dq_table: "unknown",
      flux_unit: "erg/s/cm2/A",
      n_rows: N_DOC,
      cards: {},
      notes: ["spectrum file, 1D array in HDU 0: air wavelengths"],
      ...extra,
    },
  };
}

function renderDocument(doc: SpectrumDocument | null = documentOf(), extra: Record<string, unknown> = {}): string {
  cubeState.ramp = null;
  return renderToStaticMarkup(
    createElement(SpectroscopyPanel, {
      documentMode: true,
      spectrumDocument: doc,
      filePath: DOC_PATH,
      publishGate: createFramePublishGate(),
      ...extra,
    }),
  );
}

const CUBE_ONLY_TEXT = [
  "Collapse Mean",
  "Collapse Median",
  "Collapse range",
  "Range collapse mode",
  "Moments",
  "SNR",
  "mask M0",
  "[line-fit]",
  "[frame-nav]",
  "Region spectrum",
  "Cube:",
  "Click on the preview image",
];

describe("SpectroscopyPanel document mode", () => {
  it("hides collapse, moments, line-fit maps, frame navigation and region controls", () => {
    const html = renderDocument();
    for (const text of CUBE_ONLY_TEXT) expect(html).not.toContain(text);
  });

  it("keeps the axis controls, the line list, the brush, the range and continuum boxes and the line measurement", () => {
    const html = renderDocument();
    expect(html).toContain("[axis-controls]");
    expect(html).toContain("[line-list]");
    expect(html).toContain("continuum A");
    expect(html).toContain('aria-label="Range first channel, in μm"');
    expect(html).toContain('aria-label="Continuum B last channel, in μm"');
    expect(html.match(/inputMode="decimal"/g)?.length).toBe(7);
    expect(html).toContain("[line-measure]");
  });

  it("names the spectrum file and its layout instead of a pixel or a region", () => {
    const html = renderDocument();
    expect(html).toContain("spectrum file, HDU 0");
    expect(html).toContain("40 channels, 1D array in HDU 0, flux in erg/s/cm2/A");
  });

  it("states the air frame of a ground-based file and offers the vacuum frame", () => {
    const html = renderDocument();
    expect(html).toContain("air wavelengths, as the file reads");
    expect(html).toContain('data-frame-choice="air"');
    expect(html).toContain('data-frame-choice="vacuum"');
    expect(html).toContain("Morton (2000)");
  });

  it("states the vacuum frame without the air formula note", () => {
    const html = renderDocument(documentOf({ wavelength_frame: "vacuum", axis: airAxis("wave") }));
    expect(html).toContain("vacuum wavelengths, as the file reads");
    expect(html).not.toContain("Morton (2000)");
  });

  it("offers no frame choice for an axis that is not a wavelength", () => {
    const html = renderDocument(documentOf({ wavelength_frame: "not_wavelength", axis: { ...airAxis("freq"), unit: "GHz" } }));
    expect(html).toContain("the spectral axis is not a wavelength: no air or vacuum frame applies");
    expect(html).not.toContain("data-frame-choice");
  });

  it("shows the backend refusal verbatim when the document cannot be read", () => {
    const refusal = "CTYPE1 = &#x27;WAVE-LOG&#x27;: log-lambda dispersion is not supported";
    const html = renderDocument(null, { error: "CTYPE1 = 'WAVE-LOG': log-lambda dispersion is not supported" });
    expect(html).toContain(refusal);
    expect(html).not.toContain("Click on the preview image");
    expect(html).not.toContain("Spectrum extraction failed");
  });

  it("keeps the cube rendering when the panel is not in document mode", () => {
    const html = renderClickedPixel(null);
    expect(html).not.toContain("data-frame-choice");
    expect(html).toContain("Collapse Mean");
  });
});

type Props = Record<string, unknown>;
const panelFunction = (SpectroscopyPanel as unknown as { type: (props: Props) => ReactNode }).type;

function findProps(node: ReactNode, match: (props: Props, type: unknown) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findProps(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Props>(node)) return null;
  if (match(node.props, node.type)) return node.props;
  return findProps(node.props.children as ReactNode, match);
}

function mustFind(node: ReactNode, match: (props: Props, type: unknown) => boolean, what: string): Props {
  const found = findProps(node, match);
  expect(found, what).not.toBeNull();
  return found!;
}

function pass(props: Props): ReactNode {
  hooks.run.active = true;
  hooks.run.cursor = 0;
  let tree: ReactNode;
  try {
    tree = panelFunction(props);
  } finally {
    hooks.run.active = false;
  }
  const effects = hooks.run.effects;
  hooks.run.effects = [];
  effects.forEach((effect) => effect());
  return tree;
}

const plotFunction = (SpectrumPlot as unknown as { type: (props: Props) => ReactNode }).type;
const plotRun = { cells: [] as typeof hooks.run.cells };

function spectrumPlotOf(panelTree: ReactNode): Props {
  return mustFind(panelTree, (_p, type) => type === SpectrumPlot, "spectrum plot");
}

function renderPlot(panelTree: ReactNode): ReactNode {
  const plotProps = spectrumPlotOf(panelTree);
  const panelCells = hooks.run.cells;
  hooks.run.cells = plotRun.cells;
  try {
    return hooks.render(plotFunction, plotProps) as ReactNode;
  } finally {
    hooks.run.cells = panelCells;
  }
}

function measured(source: LineMeasurement["source"]): LineMeasurement {
  return {
    z0: 5,
    z1: 39,
    n_channels: 35,
    axis_unit: "um",
    flux_unit: "erg/s/cm2/A x um",
    continuum_level: 1,
    continuum_slope: 0,
    continuum_sigma: 0.01,
    continuum_reference: AIR_VALUES[20],
    continuum_linear: false,
    continuum_channels: 10,
    continuum_windows: [
      [0, 4],
      [35, 39],
    ],
    flux: 1e-4,
    flux_err: 1e-6,
    equivalent_width: -1e-4,
    centroid: AIR_VALUES[20],
    sigma: 2e-4,
    fwhm: 4.7e-4,
    peak: 1,
    peak_channel: 20,
    snr: 100,
    velocity: null,
    fit: null,
    err_used: true,
    flux_err_terms: { pixel: 1e-6, level: 2e-7, slope: 1e-8 },
    n_dq_excluded: 0,
    n_nonfinite: 0,
    n_bad_err: 0,
    err_source: "ERR extension",
    notes: [],
    elapsed_ms: 1,
    source,
  };
}

function lineSection(tree: ReactNode): Props {
  return mustFind(tree, (p) => typeof p.onMeasure === "function", "line measurement section");
}

function commitRangeStart(tree: ReactNode, z0: number): void {
  const box = mustFind(tree, (p) => p.edge === "range:z0", "range z0 box");
  (box.onCommit as (value: number) => void)(z0);
}

async function brushAndMeasure(props: Props): Promise<void> {
  pass(props);
  commitRangeStart(pass(props), 5);
  await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
}

describe("SpectroscopyPanel document-mode line measurement", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectralMock.measure.mockReset();
    spectralMock.axis.mockReset();
    logMock.append.mockReset();
    spectralMock.measure.mockImplementation(async (_path, source) => measured(source as LineMeasurement["source"]));
  });

  const docProps = (): Props => ({ documentMode: true, spectrumDocument: documentOf(), filePath: DOC_PATH, publishGate: createFramePublishGate() });

  it("measures the spectrum file with its HDU, ERR and DQ on and the file's air frame", async () => {
    await brushAndMeasure(docProps());
    expect(spectralMock.measure).toHaveBeenCalledTimes(1);
    const [path, source, options] = spectralMock.measure.mock.calls[0];
    expect(path).toBe(DOC_PATH);
    expect(source).toEqual({ kind: "file", hdu: 0 });
    expect(options).toMatchObject({ z0: 5, z1: N_DOC - 1, useErr: true, useDq: true, wavelengthFrame: "air" });
    expect(spectralMock.axis).not.toHaveBeenCalled();
  });

  it("draws the overlay of an air document on its native axis, where the backend fitted, not on a vacuum conversion", async () => {
    const props = docProps();
    pass(props);
    commitRangeStart(pass(props), 5);
    const tree = pass(props);
    const overlay = mustFind(renderPlot(tree), (p) => p.className === "absolute inset-0 w-full pointer-events-none", "overlay canvas");
    const ctx = new Proxy({} as Record<string | symbol, unknown>, {
      get: (target, key) => (key in target ? target[key] : key === "measureText" ? () => ({ width: 10 }) : () => {}),
      set: (target, key, value) => {
        target[key] = value;
        return true;
      },
    });
    (overlay.ref as { current: unknown }).current = { width: 400, height: SPECTRUM_PLOT_HEIGHT, getContext: () => ctx };
    overlayAxes.calls = [];
    await (lineSection(tree).onMeasure as () => Promise<void>)();
    renderPlot(pass(props));
    expect(overlayAxes.calls.length).toBeGreaterThan(0);
    expect(overlayAxes.calls[overlayAxes.calls.length - 1]).toEqual(AIR_VALUES);
  });

  it("hands the plot the air axis the backend fitted on", async () => {
    const props = docProps();
    await brushAndMeasure(props);
    const lineOverlay = spectrumPlotOf(pass(props)).lineOverlay as { measurementAxis: number[] } | null;
    expect(lineOverlay?.measurementAxis).toEqual(AIR_VALUES);
  });

  it("gives the plot no overlay and no marker in document mode before a measurement", () => {
    const plot = spectrumPlotOf(settledPanel(docProps()));
    expect(plot.lineOverlay).toBeNull();
    expect(plot.fitOverlay).toBeNull();
    expect(plot.frameMarker).toBeNull();
    expect(plot.referenceChannel).toBeNull();
  });

  it("logs the file measurement as a spectrum file with the DQ request", async () => {
    await brushAndMeasure(docProps());
    expect(logMock.append).toHaveBeenCalledTimes(1);
    expect(logMock.append.mock.calls[0][0]).toMatchObject({ kind: "line", image: "spectrum file", dq: "requested", file: "ha_bess.fits" });
  });

  it("sends the frame chosen in the panel and the ERR and DQ switches of the line section", async () => {
    const props = docProps();
    pass(props);
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "vacuum" });
    pass(props);
    const controls = lineSection(pass(props)).errControls as {
      onUseErr: (v: boolean) => void;
      onUseDq: (v: boolean) => void;
      errAvailable: boolean;
      dqAvailable: boolean;
    };
    expect(controls.errAvailable).toBe(true);
    expect(controls.dqAvailable).toBe(false);
    controls.onUseErr(false);
    controls.onUseDq(false);
    commitRangeStart(pass(props), 5);
    await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    expect(spectralMock.measure.mock.calls[0][2]).toMatchObject({ useErr: false, useDq: false, wavelengthFrame: "vacuum" });
  });

  it("drops a measurement of one frame when the other frame is chosen, and keeps it when the same frame is pressed again", async () => {
    const props = docProps();
    await brushAndMeasure(props);
    expect(lineSection(pass(props)).result).not.toBeNull();
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "air" });
    pass(props);
    expect(lineSection(pass(props)).result).not.toBeNull();
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "vacuum" });
    pass(props);
    const after = lineSection(pass(props));
    expect(after.result).toBeNull();
    expect(after.error).toBeNull();
    expect(after.loading).toBe(false);
  });

  it("locks the frame buttons while a measurement runs and drops an answer measured in the frame left behind", async () => {
    let answer: () => void = () => {};
    spectralMock.measure.mockImplementation(
      (_path, source) =>
        new Promise((resolve) => {
          answer = () => resolve(measured(source as LineMeasurement["source"]));
        }),
    );
    const props = docProps();
    pass(props);
    commitRangeStart(pass(props), 5);
    const running = (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    const tree = pass(props);
    expect(lineSection(tree).loading).toBe(true);
    const viewControls = mustFind(tree, (_p, type) => type === SpectrumViewControls, "view controls");
    expect(viewControls.disabled).toBe(true);
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "vacuum" });
    pass(props);
    answer();
    await running;
    const after = lineSection(pass(props));
    expect(after.result).toBeNull();
    expect(after.loading).toBe(false);
    expect(logMock.append).not.toHaveBeenCalled();
  });

  it("leaves the wavelength frame, ERR and DQ choices out of a cube measurement", async () => {
    await brushAndMeasure({
      spectrum: AIR_VALUES.map(() => 1),
      pixelCoord: { x: 3, y: 4 },
      cubeDims: SPECTRAL_CUBE_DIMS,
      filePath: "C:/data/cube.fits",
      publishGate: createFramePublishGate(),
    });
    const [, source, options] = spectralMock.measure.mock.calls[0];
    expect(source).toEqual({ kind: "pixel", x: 3, y: 4 });
    expect(options).not.toHaveProperty("wavelengthFrame");
    expect(options).not.toHaveProperty("useErr");
    expect(options).not.toHaveProperty("useDq");
    expect(logMock.append.mock.calls[0][0]).toMatchObject({ image: "loaded cube", dq: "requested" });
  });
});

describe("SpectroscopyPanel document-mode barycentric correction", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
  });

  const axisControls = (p: Props) => typeof p.onCorrectionLoaded === "function";

  it("tells the axis controls that a spectrum file gets no correction request", () => {
    const tree = pass({ documentMode: true, spectrumDocument: documentOf(), filePath: DOC_PATH, publishGate: createFramePublishGate() });
    const controls = mustFind(tree, axisControls, "axis controls of a document");
    expect(controls.correctionUnavailable).toBe("The correction is not computed for a spectrum file.");
  });

  it("keeps the correction request of a cube", () => {
    const tree = pass({
      spectrum: AIR_VALUES.map(() => 1),
      pixelCoord: { x: 3, y: 4 },
      cubeDims: SPECTRAL_CUBE_DIMS,
      filePath: "C:/data/cube.fits",
      publishGate: createFramePublishGate(),
    });
    const controls = mustFind(tree, axisControls, "axis controls of a cube");
    expect(controls.filePath).toBe("C:/data/cube.fits");
    expect(controls.correctionUnavailable ?? null).toBeNull();
  });
});

describe("SpectroscopyPanel document-mode comparison and export", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    logMock.append.mockReset();
    backend.cubeSpectrum.mockReset();
    backend.pixelToWorld.mockReset();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const compareSection = (p: Props) => "fileSpectrumSource" in p;

  it("shows the comparison section and the CSV export for a spectrum file", () => {
    const html = renderDocument();
    expect(html).toContain("[compare]");
    expect(html).toContain("Copy CSV");
    expect(html).toContain("Save CSV");
  });

  it("compares on the spectrum's own channels with the document and its source", () => {
    const doc = documentOf();
    const tree = pass({ documentMode: true, spectrumDocument: doc, filePath: DOC_PATH, publishGate: createFramePublishGate() });
    const compare = mustFind(tree, compareSection, "comparison section of a document");
    expect(compare.fileSpectrum).toBe(doc.spectrum);
    expect(compare.fileSpectrumSource).toEqual({ kind: "file", hdu: 0 });
    expect(compare.channelCount).toBe(N_DOC);
    expect(compare.filePath).toBe(DOC_PATH);
    expect(compare.pixelCoord).toBeNull();
  });

  it("copies the spectrum file as CSV with its err column and logs the export with the document path", async () => {
    const writeText = vi.fn(async (text: string) => {
      void text;
    });
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    vi.stubGlobal("window", { setTimeout: () => 1, clearTimeout: () => {} });
    const props: Props = { documentMode: true, spectrumDocument: documentOf(), filePath: DOC_PATH, publishGate: createFramePublishGate() };
    const copy = mustFind(pass(props), (p) => p.children === "Copy CSV", "Copy CSV button");
    (copy.onClick as () => void)();
    await vi.waitFor(() => expect(logMock.append).toHaveBeenCalledTimes(1));
    const csv = writeText.mock.calls[0][0];
    const lines = csv.split("\r\n");
    const headerIndex = lines.findIndex((line) => !line.startsWith("#"));
    expect(lines[headerIndex]).toBe("channel,wavelength_vacuum_um,wavelength_air_um,air_wavelength_um,flux,flux_unit,err");
    expect(lines[headerIndex + 1].split(",")[2]).toBe(String(AIR_VALUES[0]));
    expect(csv).toContain("# wavelength_air_note: wavelength_vacuum_um converted from the spectrum's air wavelengths");
    expect(csv).toContain("# image: ha_bess.fits (spectrum file, read from disk)");
    expect(logMock.append.mock.calls[0][0]).toMatchObject({
      kind: "spectrum_export",
      file: "ha_bess.fits",
      image: "spectrum file",
      dq: "not_applied",
      source: '{"kind":"file","hdu":0}',
      params: { document_path: DOC_PATH, saved_path: null, err_column: true, dq_column: false },
    });
    expect(backend.cubeSpectrum).not.toHaveBeenCalled();
    expect(backend.pixelToWorld).not.toHaveBeenCalled();
  });
});

const REGION_CIRCLE: RegionShape = { shape: "circle", x: 4, y: 4, r: 2 };
const REGION_WEIGHT = 12.5;
const REGION_FACTOR_WINDOWS: NoiseFactor["windows"] = [
  [0, 4],
  [35, 39],
];
const NOISE_WARNING =
  "the empirical noise is 2.00 +- 0.10 x the propagated error over 40 line-free channels, outside [0.8, 1.25] by more than 2 standard errors: the ERR plane does not describe the scatter of this spectrum";
const PROPAGATION_NOTE = "error propagated from the ERR cube (HDU 2) as sqrt(sum w^2 ERR^2) over the aperture";

function regionWithErrors(factor: Partial<NoiseFactor> | null = {}, extra: Partial<RegionSpectrum> = {}): RegionSpectrum {
  const sum = AIR_VALUES.map((_, i) => 10 + Math.exp(-((i - 20) ** 2) / 8));
  const err = AIR_VALUES.map((_, i) => 0.2 + i * 0.001);
  const noiseFactor: NoiseFactor | null =
    factor === null ? null : { factor: 1.3, factor_se: 0.4, n_channels: 12, windows: REGION_FACTOR_WINDOWS, warning: null, ...factor };
  return {
    sum,
    mean: sum.map((v) => v / REGION_WEIGHT),
    npix: REGION_WEIGHT,
    n_bg: 0,
    bg_per_pixel: null,
    wavelengths: null,
    unit: "um",
    bg_subtracted: false,
    flux_jy: null,
    err,
    err_scaled: noiseFactor ? err.map((e) => e * noiseFactor.factor) : null,
    mean_err: err.map((e) => e / REGION_WEIGHT),
    used_weight: AIR_VALUES.map(() => REGION_WEIGHT),
    err_source: "ERR HDU 2",
    bg_err_per_pixel: null,
    flux_jy_err: null,
    n_dq_excluded: null,
    noise_factor: noiseFactor,
    notes: noiseFactor?.warning ? [PROPAGATION_NOTE, noiseFactor.warning] : [PROPAGATION_NOTE],
    elapsed_ms: 3,
    ...extra,
  };
}

function cubeProps(dims: CubeDims = SPECTRAL_CUBE_DIMS): Props {
  return {
    spectrum: AIR_VALUES.map(() => 1),
    pixelCoord: { x: 3, y: 4 },
    cubeDims: dims,
    filePath: "C:/data/cube.fits",
    publishGate: createFramePublishGate(),
  };
}

function showRegion(region: RegionSpectrum): void {
  spectrumState.region = region;
  spectrumState.regionSource = { kind: "region", shape: REGION_CIRCLE, background: null };
}

function renderRegion(region: RegionSpectrum): string {
  showRegion(region);
  return renderToStaticMarkup(createElement(SpectroscopyPanel, cubeProps() as never));
}

function occurrences(text: string, part: string): number {
  return text.split(part).length - 1;
}

function fakeCanvas() {
  const ctx = new Proxy({} as Record<string | symbol, unknown>, {
    get: (target, key) => (key in target ? target[key] : key === "measureText" ? () => ({ width: 10 }) : () => {}),
    set: (target, key, value) => {
      target[key] = value;
      return true;
    },
  });
  return { width: 0, height: 0, parentElement: { getBoundingClientRect: () => ({ width: 400 }) }, getContext: () => ctx };
}

function yTickCanvas() {
  const rightAligned: string[] = [];
  const ctx = new Proxy({} as Record<string | symbol, unknown>, {
    get: (target, key) => {
      if (key === "fillText") {
        return (text: string) => {
          if (target.textAlign === "right") rightAligned.push(text);
        };
      }
      return key in target ? target[key] : key === "measureText" ? () => ({ width: 10 }) : () => {};
    },
    set: (target, key, value) => {
      target[key] = value;
      return true;
    },
  });
  const canvas = { width: 0, height: 0, parentElement: { getBoundingClientRect: () => ({ width: 400 }) }, getContext: () => ctx };
  const lastTicks = () => rightAligned.slice(-5).map(Number);
  return { canvas, lastTicks };
}

const NO_WINDOWS_NOTE =
  "noise factor measured on continuum 0–4 and 35–39; no valid continuum windows are set: set them, then press Region spectrum to measure it again (without windows it drops the factor)";
const CHANGED_WINDOWS_NOTE = "noise factor measured on continuum 0–4 and 35–39; the windows changed since: press Region spectrum to measure it again";

describe("SpectroscopyPanel region spectrum errors", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
    bandCalls.calls = [];
    backend.regionSpectrum.mockReset();
  });

  afterEach(() => {
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
  });

  const pickCircle = () => {
    regionDocState.regions = [
      { id: "r1", shape: REGION_CIRCLE, props: { color: null, width: null, text: null, dash: null, include: true }, backgroundId: null } as Region,
    ];
  };
  const pressRegionSpectrum = (tree: ReactNode) =>
    (
      mustFind(
        tree,
        (p) => typeof p.title === "string" && p.title.startsWith("circle region") && typeof p.onClick === "function",
        "region spectrum button",
      ).onClick as () => Promise<void>
    )();
  const commitField = (tree: ReactNode, label: string, value: number) =>
    (mustFind(tree, (p) => p.channelLabel === label, label).onCommit as (value: number) => void)(value);
  const staleNote = (tree: ReactNode) =>
    findProps(tree, (p) => typeof p.children === "string" && p.children.startsWith("noise factor measured on continuum"))?.children ?? null;
  const factorLine = (tree: ReactNode) => mustFind(tree, (p) => "data-noise-factor" in p, "noise factor line");

  it("sends no continuum when the panel's windows fall outside the cube", async () => {
    backend.regionSpectrum.mockResolvedValue(regionWithErrors(null));
    pickCircle();
    const props = cubeProps();
    pass(props);
    commitField(pass(props), "Continuum B last channel", N_DOC + 5);
    await pressRegionSpectrum(pass(props));
    expect(backend.regionSpectrum).toHaveBeenCalledTimes(1);
    expect(backend.regionSpectrum).toHaveBeenLastCalledWith("C:/data/cube.fits", REGION_CIRCLE, null, null);
  });

  it("asks for windows first when none would be sent, and says the windows changed when they differ from the factor's", () => {
    showRegion(regionWithErrors());
    const props = cubeProps();
    pass(props);
    expect(staleNote(pass(props))).toBe(NO_WINDOWS_NOTE);
    commitRangeStart(pass(props), 5);
    (mustFind(pass(props), (p) => p.children === "auto", "auto windows button").onClick as () => void)();
    expect(staleNote(pass(props))).toBe(CHANGED_WINDOWS_NOTE);
    commitField(pass(props), "Continuum B last channel", 39);
    commitField(pass(props), "Continuum B first channel", 35);
    expect(staleNote(pass(props))).toBeNull();
    commitField(pass(props), "Continuum B last channel", N_DOC + 5);
    expect(staleNote(pass(props))).toBe(NO_WINDOWS_NOTE);
  });

  it("says the factor was not measured on the windows it sent when the backend refused them", async () => {
    const refusal = "noise factor not measured: continuum window 0..=2 has 3 channels: the noise factor needs at least 5 per window";
    backend.regionSpectrum.mockResolvedValue(regionWithErrors(null, { notes: [PROPAGATION_NOTE, refusal] }));
    pickCircle();
    const props = cubeProps();
    pass(props);
    commitRangeStart(pass(props), 3);
    (mustFind(pass(props), (p) => p.children === "auto", "auto windows button").onClick as () => void)();
    await pressRegionSpectrum(pass(props));
    const sent = guardedContinuumWindows({ z0: 3, z1: N_DOC - 1 }, N_DOC);
    expect(sent).toEqual([
      [0, 2],
      [0, 2],
    ]);
    expect(backend.regionSpectrum).toHaveBeenLastCalledWith("C:/data/cube.fits", REGION_CIRCLE, null, sent);
    const line = factorLine(pass(props));
    expect(line["data-noise-factor"]).toBe("not_measured");
    expect(line.children).toBe("error band: ERR HDU 2; noise factor not measured on continuum 0–2 (see notes)");
  });

  it("marks the factor line and the plot with the bands drawn, and explains the fill and the dashed outline", () => {
    let html = renderRegion(regionWithErrors({ factor: 1.31, factor_se: 0.05 }));
    expect(html).toContain('data-noise-factor="measured"');
    expect(html).toContain('data-error-band="propagated+scaled"');
    expect(html).toContain("(fill: propagated error; dashed: error x noise factor)");
    html = renderRegion(regionWithErrors(null));
    expect(html).toContain('data-noise-factor="none"');
    expect(html).toContain('data-error-band="propagated"');
    expect(html).not.toContain("dashed:");
    html = renderRegion(regionWithErrors(null, { err: null, mean_err: null, err_source: "none" }));
    expect(html).toContain('data-error-band="none"');
    expect(html).not.toContain("data-noise-factor");
    spectrumState.region = null;
    spectrumState.regionSource = null;
    expect(renderToStaticMarkup(createElement(SpectroscopyPanel, cubeProps() as never))).toContain('data-error-band="none"');
  });

  it("fits the error band, the scaled outline included, inside the plot's y range", () => {
    const sum = AIR_VALUES.map((_, i) => 0.1 + 0.1 * (i % 2));
    const err = AIR_VALUES.map(() => 0.5);
    showRegion(
      regionWithErrors(
        { factor: 1.3 },
        { sum, mean: sum.map((v) => v / REGION_WEIGHT), err, err_scaled: err.map((e) => e * 1.3), mean_err: err.map((e) => e / REGION_WEIGHT) },
      ),
    );
    const props = cubeProps();
    const tree = pass(props);
    const plot = yTickCanvas();
    (mustFind(renderPlot(tree), (p) => p.className === "w-full rounded-md cursor-crosshair", "plot canvas").ref as { current: unknown }).current = plot.canvas;
    (mustFind(pass(props), (p) => p.children === "mean", "mean series button").onClick as () => void)();
    renderPlot(pass(props));
    (mustFind(pass(props), (p) => p.children === "sum", "sum series button").onClick as () => void)();
    renderPlot(pass(props));
    const ticks = plot.lastTicks();
    expect(ticks).toHaveLength(5);
    expect(ticks[0]).toBeGreaterThanOrEqual(0.2 + 0.65);
    expect(ticks[4]).toBeLessThanOrEqual(0.1 - 0.65);
  });

  it("prints the noise factor with its ERR source, standard error and line-free channel count", () => {
    const html = renderRegion(regionWithErrors({ factor: 1.31, factor_se: 0.05 }));
    expect(html).toContain("ERR HDU 2 x 1.31 +- 0.05 (12 line-free channels)");
  });

  it("shows no warning for a factor of 1.3 with a wide standard error when the backend sent no warning", () => {
    const html = renderRegion(regionWithErrors({ factor: 1.3, factor_se: 0.4, warning: null }));
    expect(html).toContain("ERR HDU 2 x 1.30 +- 0.40 (12 line-free channels)");
    expect(html).not.toContain("data-noise-warning");
    expect(html).not.toContain("outside [0.8, 1.25]");
  });

  it("shows the backend warning verbatim, once, exactly when the backend sent one", () => {
    const html = renderRegion(regionWithErrors({ factor: 1.1, factor_se: 0.01, warning: NOISE_WARNING }));
    expect(html).toContain("data-noise-warning");
    expect(occurrences(html, NOISE_WARNING)).toBe(1);
    expect(html).toContain(PROPAGATION_NOTE);
  });

  it("prints no factor line for a region spectrum measured without continuum windows", () => {
    const html = renderRegion(regionWithErrors(null));
    expect(html).not.toContain("line-free channels");
    expect(html).not.toContain("data-noise-warning");
  });

  it("asks for the region spectrum with the panel's continuum windows so the factor uses the line's windows", async () => {
    backend.regionSpectrum.mockResolvedValue(regionWithErrors());
    regionDocState.regions = [
      { id: "r1", shape: REGION_CIRCLE, props: { color: null, width: null, text: null, dash: null, include: true }, backgroundId: null } as Region,
    ];
    const props = cubeProps();
    const regionButton = (tree: ReactNode) =>
      mustFind(tree, (p) => typeof p.title === "string" && p.title.startsWith("circle region") && typeof p.onClick === "function", "region spectrum button");
    pass(props);
    await (regionButton(pass(props)).onClick as () => Promise<void>)();
    expect(backend.regionSpectrum).toHaveBeenLastCalledWith("C:/data/cube.fits", REGION_CIRCLE, null, null);
    commitRangeStart(pass(props), 5);
    (mustFind(pass(props), (p) => p.children === "auto", "auto windows button").onClick as () => void)();
    await (regionButton(pass(props)).onClick as () => Promise<void>)();
    const expected = guardedContinuumWindows({ z0: 5, z1: N_DOC - 1 }, N_DOC);
    expect(expected).not.toBeNull();
    expect(backend.regionSpectrum).toHaveBeenLastCalledWith("C:/data/cube.fits", REGION_CIRCLE, null, expected);
  });

  it("draws the mean view's band from mean_err and the sum view's band from err, each with its scaled band", () => {
    const region = regionWithErrors({ factor: 1.3 });
    showRegion(region);
    const props = cubeProps();
    const tree = pass(props);
    const canvas = mustFind(renderPlot(tree), (p) => p.className === "w-full rounded-md cursor-crosshair", "plot canvas");
    (canvas.ref as { current: unknown }).current = fakeCanvas();
    (mustFind(pass(props), (p) => p.children === "mean", "mean series button").onClick as () => void)();
    renderPlot(pass(props));
    expect(bandCalls.calls.length).toBeGreaterThan(0);
    const meanErrs = bandCalls.calls.map((call) => call.err);
    expect(meanErrs).toContainEqual(region.mean_err);
    expect(meanErrs).toContainEqual(region.mean_err!.map((e) => e * 1.3));
    expect(meanErrs).not.toContainEqual(region.err);
    expect(bandCalls.calls.every((call) => call.y === bandCalls.calls[0].y)).toBe(true);
    expect(bandCalls.calls[0].y).toEqual(region.mean);
    bandCalls.calls = [];
    (mustFind(pass(props), (p) => p.children === "sum", "sum series button").onClick as () => void)();
    renderPlot(pass(props));
    const sumErrs = bandCalls.calls.map((call) => call.err);
    expect(sumErrs).toContainEqual(region.err);
    expect(sumErrs).toContainEqual(region.err_scaled);
    expect(sumErrs).not.toContainEqual(region.mean_err);
  });

  it("hands the plot the series and the band of the shown region view", () => {
    const region = regionWithErrors({ factor: 1.3 });
    showRegion(region);
    const props = cubeProps();
    pass(props);
    (mustFind(pass(props), (p) => p.children === "mean", "mean series button").onClick as () => void)();
    const mean = spectrumPlotOf(pass(props)).model as SpectrumPlotModel;
    expect(mean.series).toEqual(region.mean);
    expect(mean.errors?.err).toEqual(region.mean_err);
    expect(mean.errors?.scaled).toEqual(region.mean_err!.map((e) => e * 1.3));
    (mustFind(pass(props), (p) => p.children === "sum", "sum series button").onClick as () => void)();
    const sum = spectrumPlotOf(pass(props)).model as SpectrumPlotModel;
    expect(sum.series).toEqual(region.sum);
    expect(sum.errors?.err).toEqual(region.err);
    expect(sum.errors?.scaled).toEqual(region.err_scaled);
  });
});

function momentResult(extra: Partial<MomentMapsResult> = {}): MomentMapsResult {
  const files = { png_path: "C:/out/m.png", fits_path: "C:/out/m.fits" };
  return {
    m0: files,
    m1: files,
    m2: files,
    m0_unit: "MJy/sr km/s",
    velocity_unit: "km/s",
    noise_per_channel: 0.01,
    n_channels: 35,
    notes: [],
    dimensions: [8, 8],
    has_err: true,
    err_source: "ERR HDU 2",
    noise_factor: 1.3127,
    elapsed_ms: 4,
    ...extra,
  };
}

const MEDIAN_NOTE = "the median error is approximate: sqrt(pi/2) = 1.2533 times the error of the mean of the same channels";

function collapseResult(extra: Partial<CollapseRangeResult> = {}): CollapseRangeResult {
  return {
    png_path: "C:/out/c.png",
    fits_path: "C:/out/c.fits",
    z0: 5,
    z1: 39,
    mode: "median",
    zero_policy: "include_physical_zeros",
    dimensions: [8, 8],
    has_err: true,
    err_source: "ERR HDU 2",
    notes: ["ERR extension propagated from the ERR cube (HDU 2)", MEDIAN_NOTE],
    elapsed_ms: 2,
    ...extra,
  };
}

describe("SpectroscopyPanel moment and collapse error planes", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    backend.moments.mockReset();
    backend.collapseRange.mockReset();
  });

  const restDims: CubeDims = { ...SPECTRAL_CUBE_DIMS, spectral_axis: { ...airAxis("wave"), rest_wavelength_um: 0.6563 } };
  const errPlane = (tree: ReactNode, which: string) => mustFind(tree, (p) => p["data-err-plane"] === which, `${which} ERR line`);

  it("says the moment maps carry an ERR plane, from which ERR cube, and their noise factor", async () => {
    backend.moments.mockResolvedValue(momentResult());
    const props = cubeProps(restDims);
    pass(props);
    commitRangeStart(pass(props), 5);
    await (mustFind(pass(props), (p) => p.label === "Moments", "Moments button").onClick as () => Promise<void>)();
    expect(errPlane(pass(props), "moments").children).toBe("ERR written (ERR HDU 2), noise factor 1.31 in ABNOISEF");
  });

  it("says a moment run on a cube without ERR wrote no ERR plane", async () => {
    backend.moments.mockResolvedValue(momentResult({ has_err: false, err_source: "none", noise_factor: null }));
    const props = cubeProps(restDims);
    pass(props);
    commitRangeStart(pass(props), 5);
    await (mustFind(pass(props), (p) => p.label === "Moments", "Moments button").onClick as () => Promise<void>)();
    expect(errPlane(pass(props), "moments").children).toBe("no ERR written (error source: none)");
  });

  const outputNoteFor = (tree: ReactNode, path: string, what: string) => mustFind(tree, (p, type) => type === OutputNote && p.path === path, what);

  it.each(["Moments", "Collapse range", "Collapse Mean"])("exposes the current %s manifest and clears it after a source change", async (label) => {
    const provenance = { source: { input_ref: "C:/data/cube.fits", sha256: "1234567890abcdef".repeat(4) }, integrity: "sha256_before_open_and_before_publication" };
    const trace = { provenance_path: "C:/out/run/provenance.json", provenance };
    backend.moments.mockResolvedValue(momentResult(trace));
    backend.collapseRange.mockResolvedValue(collapseResult(trace));
    const props = cubeProps(restDims);
    pass(props);
    commitRangeStart(pass(props), 5);
    await (mustFind(pass(props), (p) => p.label === label, label).onClick as () => Promise<void>)();
    const note = outputNoteFor(pass(props), trace.provenance_path, "result manifest");
    expect(note.kind).toBe("manifest");
    expect(note.provenance).toBe(provenance);
    const next = settledPanel({ ...props, filePath: "C:/data/other.fits" });
    expect(findProps(next, (p) => p.path === trace.provenance_path)).toBeNull();
  });

  it("names the moment map on screen as a file of the output folder, which is cleared at launch", async () => {
    const m0 = { png_path: "C:/out/cube_moment0_5-39.png", fits_path: "C:/out/cube_moment0_5-39.fits" };
    const m1 = { png_path: "C:/out/cube_moment1_5-39.png", fits_path: "C:/out/cube_moment1_5-39.fits" };
    backend.moments.mockResolvedValue(momentResult({ m0, m1 }));
    const props = cubeProps(restDims);
    pass(props);
    commitRangeStart(pass(props), 5);
    await (mustFind(pass(props), (p) => p.label === "Moments", "Moments button").onClick as () => Promise<void>)();
    outputNoteFor(pass(props), m0.fits_path, "M0 output note");
    (mustFind(pass(props), (p) => p.children === "m1", "m1 button").onClick as () => void)();
    outputNoteFor(pass(props), m1.fits_path, "M1 output note");
  });

  it("names the range collapse and the full collapse as files of the output folder", async () => {
    backend.collapseRange.mockResolvedValue(collapseResult({ fits_path: "C:/out/cube_collapse_median_5-39.fits" }));
    const props = cubeProps();
    pass(props);
    commitRangeStart(pass(props), 5);
    await (mustFind(pass(props), (p) => p.label === "Collapse range", "Collapse range button").onClick as () => Promise<void>)();
    outputNoteFor(pass(props), "C:/out/cube_collapse_median_5-39.fits", "range collapse output note");

    backend.collapseRange.mockResolvedValue(collapseResult({ fits_path: "C:/out/cube_collapse_mean_0-39.fits", mode: "mean", z0: 0 }));
    await (mustFind(pass(props), (p) => p.label === "Collapse Mean", "Collapse Mean button").onClick as () => Promise<void>)();
    outputNoteFor(pass(props), "C:/out/cube_collapse_mean_0-39.fits", "full collapse output note");
  });

  it("shows the range collapse's error source and its notes, the approximate median error among them", async () => {
    backend.collapseRange.mockResolvedValue(collapseResult());
    const props = cubeProps();
    pass(props);
    commitRangeStart(pass(props), 5);
    await (mustFind(pass(props), (p) => p.label === "Collapse range", "Collapse range button").onClick as () => Promise<void>)();
    const tree = pass(props);
    expect(errPlane(tree, "collapse").children).toBe("ERR written (ERR HDU 2)");
    mustFind(tree, (p) => p.children === MEDIAN_NOTE, "median error note");
  });
});

function fitResult(): LineFitResult {
  return {
    planes: { velocity: { png_path: "C:/out/fit.png", fits_path: "C:/out/fit.fits", previewUrl: "asset://fit.png" } },
    plane_order: ["velocity"],
    units: { flux: "MJy/sr km/s", velocity: "km/s", sigma: "km/s" },
    sigma_label: "observed",
    weighting: "err",
    err_hdu: 2,
    dq_hdu: 3,
    z0: 5,
    z1: 39,
    n_channels: 35,
    rest_um: 2.16612,
    convention: "optical",
    components: "one",
    continuum_windows: [[0, 4], [35, 39]],
    resolving_power: null,
    snr_threshold: 3,
    n_fit: 64,
    n_masked: 0,
    n_const_continuum: 0,
    n_two_components: 0,
    median_chi2_red: 1,
    notes: [],
    dimensions: [8, 8],
    elapsed_ms: 3,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function settledPanel(props: Props): ReactNode {
  pass(props);
  return pass(props);
}

function fitSection(tree: ReactNode): Props {
  return mustFind(tree, (p) => typeof p.onFit === "function", "fit section");
}

function axisSection(tree: ReactNode): Props {
  return mustFind(tree, (p) => typeof p.onRestChange === "function", "spectral axis section");
}

function clickCalculation(tree: ReactNode, label: string): Promise<void> {
  return (mustFind(tree, (p) => p.label === label, `${label} button`).onClick as () => Promise<void>)();
}

type PanelEdit = (tree: ReactNode) => void;

const commonCalculationEdits: [string, PanelEdit][] = [
  ["range", (tree) => commitRangeStart(tree, 6)],
  ["continuum", (tree) => (mustFind(tree, (p) => p.channelLabel === "Continuum A first channel", "continuum A").onCommit as (n: number) => void)(1)],
  ["rest wavelength", (tree) => (axisSection(tree).onRestChange as (v: number) => void)(2.16612)],
  ["velocity convention", (tree) => (axisSection(tree).onConventionChange as (v: string) => void)("radio")],
];

const fitParameterEdits: [string, PanelEdit][] = [
  ...commonCalculationEdits,
  ["fit S/N", (tree) => (fitSection(tree).onSnrText as (v: string) => void)("5")],
  ["emission only", (tree) => (fitSection(tree).onEmissionOnly as (v: boolean) => void)(false)],
  ["ERR weighting", (tree) => (fitSection(tree).onUseErr as (v: boolean) => void)(false)],
  ["resolving power", (tree) => (fitSection(tree).onResolvingPowerText as (v: string) => void)("2700")],
  ["components", (tree) => (fitSection(tree).onComponents as (v: string) => void)("two")],
  ["redshift", (tree) => (mustFind(tree, (p) => typeof p.onRedshiftChange === "function", "line list").onRedshiftChange as (z: number) => void)(0.2)],
];

const fitCalculationEdits: [string, PanelEdit][] = [
  ...fitParameterEdits,
  ["DQ masking", (tree) => (fitSection(tree).onUseDq as (v: boolean) => void)(false)],
];

const momentCalculationEdits: [string, PanelEdit][] = [
  ...commonCalculationEdits,
  ["moment S/N", (tree) => (mustFind(tree, (p) => p.min === 0 && typeof p.onChange === "function", "moment S/N").onChange as (v: string) => void)("5")],
  ["moment masking", (tree) => (mustFind(tree, (p) => p.type === "checkbox", "moment mask").onChange as (e: unknown) => void)({ target: { checked: false } })],
];

function allProps(node: ReactNode, match: (props: Props, type: unknown) => boolean): Props[] {
  if (Array.isArray(node)) return node.flatMap((child) => allProps(child, match));
  if (!isValidElement<Props>(node)) return [];
  return [...(match(node.props, node.type) ? [node.props] : []), ...allProps(node.props.children as ReactNode, match)];
}

const staleNotes = (tree: ReactNode) => allProps(tree, (_p, type) => type === StaleResultNote).length;
const staleMarks = (tree: ReactNode) => ({ panelNotes: staleNotes(tree), fitStale: fitSection(tree).stale });
const NOT_STALE = { panelNotes: 0, fitStale: false };
const markedStale = (label: string) => (label === "Fit lines" ? { panelNotes: 0, fitStale: true } : { panelNotes: 1, fitStale: false });
const momentSnr = (tree: ReactNode, text: string) =>
  (mustFind(tree, (p) => p.min === 0 && typeof p.onChange === "function", "moment S/N").onChange as (v: string) => void)(text);
const fitSnr = (tree: ReactNode, text: string) => (fitSection(tree).onSnrText as (v: string) => void)(text);
const chooseRangeMode = (tree: ReactNode, value: string) =>
  (mustFind(tree, (p) => p["aria-label"] === "Range collapse mode", "collapse mode").onChange as (e: unknown) => void)({ target: { value } });

describe("SpectroscopyPanel result validity", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
    fileState.id = "cube-1";
    fileState.imageRef = "C:/data/cube.fits";
    fileState.reloads = 0;
    fileState.finishedAt = 1;
    spectrumActions.begin.mockClear();
    spectrumActions.commit.mockClear();
    spectrumActions.fail.mockClear();
    spectrumActions.clear.mockClear();
    backend.regionSpectrum.mockReset();
    backend.fit.mockReset().mockResolvedValue(fitResult());
    backend.inspect.mockReset();
    backend.moments.mockReset().mockResolvedValue(momentResult({ m0: { png_path: "m.png", fits_path: "m.fits", previewUrl: "asset://m.png" } }));
    backend.collapseRange.mockReset();
    spectralMock.measure.mockReset().mockImplementation(async (_path, source) => measured(source as LineMeasurement["source"]));
    spectralMock.axis.mockReset();
    logMock.append.mockReset();
  });

  const prepared = () => {
    const props = { ...cubeProps(), publishGate: createFramePublishGate(), onCubeResult: vi.fn() };
    settledPanel(props);
    commitRangeStart(pass(props), 5);
    settledPanel(props);
    return props;
  };

  const calculateWithDq = (tree: ReactNode, label: string) => label === "Fit lines" ? (fitSection(tree).onFit as () => Promise<void>)() : clickCalculation(tree, label);
  const requestedDq = (label: string) => {
    if (label !== "Fit lines" && label !== "Moments") return backend.collapseRange.mock.calls.at(-1)?.[6];
    const config = (label === "Fit lines" ? backend.fit.mock.calls.at(-1)?.[2] : backend.moments.mock.calls.at(-1)?.[2]) as { dq_policy?: string } | undefined;
    return config?.dq_policy;
  };

  it.each(["Collapse Mean", "Collapse range", "Moments", "Fit lines"])("sends the shared explicit DO_NOT_USE policy to %s and invalidates its result when ignored", async (label) => {
    backend.collapseRange.mockResolvedValue(collapseResult({ previewUrl: "asset://collapse.png" }));
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await calculateWithDq(pass(props), label);
    expect(requestedDq(label)).toBe("do_not_use");
    const select = mustFind(pass(props), (p) => p["aria-label"] === "Science DQ policy", "shared DQ policy");
    (select.onChange as (e: unknown) => void)({ target: { value: "ignore" } });
    settledPanel(props);
    expect(props.onInvalidateCubeResult).toHaveBeenCalled();
    await calculateWithDq(pass(props), label);
    expect(requestedDq(label)).toBe("ignore");
  });

  it.each(["Collapse Mean", "Collapse range", "Moments", "Fit lines"])("rejects a pending %s after the common DQ policy changes", async (label) => {
    const pending = deferred<unknown>();
    if (label === "Fit lines") backend.fit.mockReturnValue(pending.promise);
    else if (label === "Moments") backend.moments.mockReturnValue(pending.promise);
    else backend.collapseRange.mockReturnValue(pending.promise);
    const props = prepared();
    const running = calculateWithDq(pass(props), label);
    await Promise.resolve();
    const select = mustFind(pass(props), (p) => p["aria-label"] === "Science DQ policy", "shared DQ policy");
    (select.onChange as (e: unknown) => void)({ target: { value: "ignore" } });
    settledPanel(props);
    pending.resolve(label === "Fit lines" ? fitResult() : label === "Moments" ? momentResult() : collapseResult({ previewUrl: "asset://collapse.png" }));
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
  });

  it.each(["Collapse Mean", "Collapse Median", "Collapse range"])("includes physical zeros by default and invalidates %s when the policy changes", async (label) => {
    backend.collapseRange.mockResolvedValue(collapseResult({ previewUrl: "asset://collapse.png" }));
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await clickCalculation(pass(props), label);
    expect(backend.collapseRange.mock.calls.at(-1)?.[5]).toBe("include_physical_zeros");
    const select = mustFind(pass(props), (p) => p["aria-label"] === "Collapse zero handling", "zero policy");
    (select.onChange as (e: unknown) => void)({ target: { value: "exclude_zero_padding" } });
    settledPanel(props);
    expect(props.onInvalidateCubeResult).toHaveBeenCalledWith(["C:/out/c.fits"]);
    await clickCalculation(pass(props), label);
    expect(backend.collapseRange.mock.calls.at(-1)?.[5]).toBe("exclude_zero_padding");
  });

  it.each(["Collapse Mean", "Collapse range"])("rejects a pending %s after changing zero handling", async (label) => {
    const pending = deferred<CollapseRangeResult>();
    backend.collapseRange.mockReturnValue(pending.promise);
    const props = prepared();
    const running = clickCalculation(pass(props), label);
    await Promise.resolve();
    const select = mustFind(pass(props), (p) => p["aria-label"] === "Collapse zero handling", "zero policy");
    (select.onChange as (e: unknown) => void)({ target: { value: "exclude_zero_padding" } });
    settledPanel(props);
    pending.resolve(collapseResult({ previewUrl: "asset://collapse.png" }));
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
  });

  it("clears a completed fit and invalidates its maps after changing the DQ masking", async () => {
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await (fitSection(pass(props)).onFit as () => Promise<void>)();
    expect(fitSection(pass(props)).run).not.toBeNull();
    (fitSection(pass(props)).onUseDq as (v: boolean) => void)(false);
    const tree = settledPanel(props);
    const after = fitSection(tree);
    expect(after.run).toBeNull();
    expect(after.inspect).toBeNull();
    expect(after.error).toBeNull();
    expect(staleMarks(tree)).toEqual(NOT_STALE);
    expect(props.onInvalidateCubeResult).toHaveBeenCalledExactlyOnceWith(["C:/out/fit.fits"]);
  });

  it.each(fitParameterEdits)("keeps a completed fit and its published map, marked stale, after changing %s", async (_name, edit) => {
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await (fitSection(pass(props)).onFit as () => Promise<void>)();
    const run = fitSection(pass(props)).run;
    expect(run).not.toBeNull();
    expect(staleMarks(settledPanel(props))).toEqual(NOT_STALE);
    edit(pass(props));
    const tree = settledPanel(props);
    expect(fitSection(tree).run).toBe(run);
    expect(fitSection(tree).error).toBeNull();
    expect(staleMarks(tree)).toEqual(markedStale("Fit lines"));
    expect(props.onInvalidateCubeResult).not.toHaveBeenCalled();
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
  });

  it.each(fitCalculationEdits)("rejects a late fit after changing %s", async (_name, edit) => {
    const pending = deferred<LineFitResult>();
    backend.fit.mockReturnValue(pending.promise);
    const props = prepared();
    const running = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    expect(backend.fit).toHaveBeenCalledTimes(1);
    edit(pass(props));
    settledPanel(props);
    pending.resolve(fitResult());
    await running;
    expect(fitSection(settledPanel(props)).run).toBeNull();
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(logMock.append).not.toHaveBeenCalled();
  });

  it.each(momentCalculationEdits)("keeps completed moment maps and the published map, marked stale, after changing %s", async (_name, edit) => {
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await clickCalculation(pass(props), "Moments");
    expect(staleNotes(settledPanel(props))).toBe(0);
    edit(pass(props));
    const tree = settledPanel(props);
    expect(findProps(tree, (p) => p["data-err-plane"] === "moments")).not.toBeNull();
    expect(staleNotes(tree)).toBe(1);
    expect(props.onInvalidateCubeResult).not.toHaveBeenCalled();
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
  });

  it.each(momentCalculationEdits)("rejects late moment maps after changing %s", async (_name, edit) => {
    const pending = deferred<MomentMapsResult>();
    backend.moments.mockReturnValue(pending.promise);
    const props = prepared();
    const running = clickCalculation(pass(props), "Moments");
    await Promise.resolve();
    expect(backend.moments).toHaveBeenCalledTimes(1);
    edit(pass(props));
    settledPanel(props);
    pending.resolve(momentResult({ m0: { png_path: "m.png", fits_path: "m.fits", previewUrl: "asset://m.png" } }));
    await running;
    expect(findProps(settledPanel(props), (p) => p["data-err-plane"] === "moments")).toBeNull();
    expect(props.onCubeResult).not.toHaveBeenCalled();
  });

  it.each([
    ...commonCalculationEdits,
    ["line model", (tree: ReactNode) => (lineSection(tree).onModelChange as (v: string) => void)("lorentzian")],
  ] as [string, PanelEdit][])("clears a line measurement after changing %s", async (_name, edit) => {
    const props = prepared();
    await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    expect(lineSection(pass(props)).result).not.toBeNull();
    edit(pass(props));
    expect(lineSection(settledPanel(props)).result).toBeNull();
  });

  it("does not revive a measurement after moving from spaxel A to B and back to A", async () => {
    const props = prepared();
    await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    expect(lineSection(pass(props)).result).not.toBeNull();
    expect(lineSection(settledPanel({ ...props, pixelCoord: { x: 4, y: 4 } })).result).toBeNull();
    expect(lineSection(settledPanel(props)).result).toBeNull();
  });

  it("does not publish a pending measurement after the selected spaxel changes", async () => {
    const pending = deferred<LineMeasurement>();
    spectralMock.measure.mockReturnValue(pending.promise);
    const props = prepared();
    const running = (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    const next = { ...props, pixelCoord: { x: 4, y: 4 } };
    settledPanel(next);
    pending.resolve(measured({ kind: "pixel", x: 3, y: 4 }));
    await running;
    expect(lineSection(settledPanel(next)).result).toBeNull();
    expect(logMock.append).not.toHaveBeenCalled();
    expect(lineSection(settledPanel(props)).result).toBeNull();
  });

  it.each(["fit", "moments", "collapse"])("does not publish a pending %s after unmount", async (kind) => {
    const pending = deferred<unknown>();
    const props = prepared();
    let result: unknown;
    let running: Promise<void>;
    if (kind === "fit") {
      backend.fit.mockReturnValue(pending.promise);
      result = fitResult();
      running = (fitSection(pass(props)).onFit as () => Promise<void>)();
    } else if (kind === "moments") {
      backend.moments.mockReturnValue(pending.promise);
      result = momentResult({ m0: { png_path: "m.png", fits_path: "m.fits", previewUrl: "asset://m.png" } });
      running = clickCalculation(pass(props), "Moments");
    } else {
      backend.collapseRange.mockReturnValue(pending.promise);
      result = collapseResult({ previewUrl: "asset://c.png" });
      running = clickCalculation(pass(props), "Collapse range");
    }
    await Promise.resolve();
    hooks.mount();
    pending.resolve(result);
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(logMock.append).not.toHaveBeenCalled();
  });

  it.each(["Collapse Mean", "Collapse range"])("keeps a newer %s running when an older request finishes", async (label) => {
    const old = deferred<CollapseRangeResult>();
    const current = deferred<CollapseRangeResult>();
    backend.collapseRange.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    const props = prepared();
    const first = clickCalculation(pass(props), label);
    await Promise.resolve();
    const next = { ...props, filePath: "C:/data/other.fits" };
    settledPanel(next);
    commitRangeStart(pass(next), 5);
    settledPanel(next);
    const second = clickCalculation(pass(next), label);
    await Promise.resolve();
    expect(backend.collapseRange).toHaveBeenCalledTimes(2);
    old.resolve(collapseResult());
    await first;
    expect(mustFind(settledPanel(next), (p) => p.label === label, label).loading).toBe(true);
    current.resolve(collapseResult());
    await second;
    expect(mustFind(settledPanel(next), (p) => p.label === label, label).loading).toBe(false);
  });

  it.each([
    ["reduction mode", (tree: ReactNode) => chooseRangeMode(tree, "mean")],
    ["range", (tree: ReactNode) => commitRangeStart(tree, 6)],
  ] as [string, PanelEdit][])("keeps the range collapse and its published map, marked stale, when its %s changes", async (_name, edit) => {
    backend.collapseRange.mockResolvedValue(collapseResult({ previewUrl: "asset://collapse.png" }));
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await clickCalculation(pass(props), "Collapse range");
    expect(staleNotes(settledPanel(props))).toBe(0);
    edit(pass(props));
    const tree = settledPanel(props);
    expect(findProps(tree, (p) => p["data-err-plane"] === "collapse")).not.toBeNull();
    expect(staleNotes(tree)).toBe(1);
    expect(props.onInvalidateCubeResult).not.toHaveBeenCalled();
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["Collapse range", (tree: ReactNode) => chooseRangeMode(tree, "mean"), (tree: ReactNode) => chooseRangeMode(tree, "sum")],
    ["Moments", (tree: ReactNode) => momentSnr(tree, "5"), (tree: ReactNode) => momentSnr(tree, "3")],
    ["Fit lines", (tree: ReactNode) => fitSnr(tree, "5"), (tree: ReactNode) => fitSnr(tree, "3")],
  ] as [string, PanelEdit, PanelEdit][])("drops the stale note of %s when its parameters return to the values of the result", async (label, edit, revert) => {
    backend.collapseRange.mockResolvedValue(collapseResult({ previewUrl: "asset://collapse.png" }));
    const props = prepared();
    await calculateWithDq(pass(props), label);
    edit(pass(props));
    expect(staleMarks(settledPanel(props))).toEqual(markedStale(label));
    revert(pass(props));
    expect(staleMarks(settledPanel(props))).toEqual(NOT_STALE);
  });

  it("marks only the results whose parameters changed", async () => {
    backend.collapseRange.mockResolvedValue(collapseResult({ previewUrl: "asset://collapse.png" }));
    const props = prepared();
    await clickCalculation(pass(props), "Collapse range");
    await clickCalculation(pass(props), "Moments");
    await (fitSection(pass(props)).onFit as () => Promise<void>)();
    expect(staleMarks(settledPanel(props))).toEqual(NOT_STALE);
    momentSnr(pass(props), "5");
    expect(staleMarks(settledPanel(props))).toEqual({ panelNotes: 1, fitStale: false });
    chooseRangeMode(pass(props), "mean");
    expect(staleMarks(settledPanel(props))).toEqual({ panelNotes: 2, fitStale: false });
    fitSnr(pass(props), "5");
    expect(staleMarks(settledPanel(props))).toEqual({ panelNotes: 2, fitStale: true });
  });

  const nextRun: Record<string, { service: typeof backend.fit; old: string[]; fresh: string; result: () => unknown; shown: (tree: ReactNode) => boolean }> = {
    "Collapse range": {
      service: backend.collapseRange,
      old: ["C:/out/c.fits"],
      fresh: "C:/out/c2.fits",
      result: () => collapseResult({ fits_path: "C:/out/c2.fits", previewUrl: "asset://c2.png" }),
      shown: (tree) => findProps(tree, (p, type) => type === OutputNote && p.path === "C:/out/c2.fits") !== null,
    },
    Moments: {
      service: backend.moments,
      old: ["m.fits", "C:/out/m.fits"],
      fresh: "C:/out/m2_0.fits",
      result: () =>
        momentResult({
          m0: { png_path: "C:/out/m2_0.png", fits_path: "C:/out/m2_0.fits", previewUrl: "asset://m2_0.png" },
          m1: { png_path: "C:/out/m2_1.png", fits_path: "C:/out/m2_1.fits" },
          m2: { png_path: "C:/out/m2_2.png", fits_path: "C:/out/m2_2.fits" },
        }),
      shown: (tree) => findProps(tree, (p, type) => type === OutputNote && p.path === "C:/out/m2_0.fits") !== null,
    },
    "Fit lines": {
      service: backend.fit,
      old: ["C:/out/fit.fits"],
      fresh: "C:/out/fit2.fits",
      result: () => ({ ...fitResult(), planes: { velocity: { png_path: "C:/out/fit2.png", fits_path: "C:/out/fit2.fits", previewUrl: "asset://fit2.png" } } }),
      shown: (tree) => (fitSection(tree).run as { result: LineFitResult } | null)?.result.planes.velocity?.fits_path === "C:/out/fit2.fits",
    },
  };

  it.each(Object.keys(nextRun))("replaces a stale %s on the next run and invalidates the maps it replaced", async (label) => {
    const next = nextRun[label];
    backend.collapseRange.mockResolvedValue(collapseResult({ previewUrl: "asset://collapse.png" }));
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await calculateWithDq(pass(props), label);
    commitRangeStart(pass(props), 6);
    expect(staleMarks(settledPanel(props))).toEqual(markedStale(label));
    next.service.mockResolvedValue(next.result());
    await calculateWithDq(pass(props), label);
    const tree = settledPanel(props);
    expect(next.shown(tree)).toBe(true);
    expect(staleMarks(tree)).toEqual(NOT_STALE);
    expect(props.onCubeResult).toHaveBeenLastCalledWith(expect.objectContaining({ fitsPath: next.fresh }));
    expect(new Set(props.onInvalidateCubeResult.mock.calls.flatMap(([paths]) => paths as string[]))).toEqual(new Set(next.old));
  });

  it.each(Object.keys(nextRun))("keeps the stale %s when another edit rejects its next run", async (label) => {
    const next = nextRun[label];
    backend.collapseRange.mockResolvedValue(collapseResult({ previewUrl: "asset://collapse.png" }));
    const props = { ...prepared(), onInvalidateCubeResult: vi.fn() };
    await calculateWithDq(pass(props), label);
    commitRangeStart(pass(props), 6);
    settledPanel(props);
    const pending = deferred<unknown>();
    next.service.mockReturnValue(pending.promise);
    const running = calculateWithDq(pass(props), label);
    await Promise.resolve();
    commitRangeStart(pass(props), 7);
    settledPanel(props);
    pending.resolve(next.result());
    await running;
    const tree = settledPanel(props);
    expect(next.shown(tree)).toBe(false);
    expect(staleMarks(tree)).toEqual(markedStale(label));
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
    expect(props.onInvalidateCubeResult).not.toHaveBeenCalled();
  });

  it.each(["reload", "HDU", "file identity"])("rejects a pending fit after changing the input %s at the same path", async (change) => {
    const pending = deferred<LineFitResult>();
    backend.fit.mockReturnValue(pending.promise);
    const props = prepared();
    const running = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    if (change === "reload") fileState.reloads++;
    else if (change === "HDU") fileState.imageRef = "C:/data/cube.fits#hdu=2";
    else fileState.id = "cube-2";
    settledPanel(props);
    pending.resolve(fitResult());
    await running;
    expect(fitSection(settledPanel(props)).run).toBeNull();
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(logMock.append).not.toHaveBeenCalled();
  });

  it("ignores an older fit failure while a new fit is running", async () => {
    const old = deferred<LineFitResult>();
    const current = deferred<LineFitResult>();
    backend.fit.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise);
    const props = prepared();
    const first = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    (fitSection(pass(props)).onSnrText as (v: string) => void)("5");
    settledPanel(props);
    const second = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    old.reject(new Error("old fit failed"));
    await first;
    expect(fitSection(settledPanel(props)).loading).toBe(true);
    expect(fitSection(pass(props)).error).toBeNull();
    current.resolve(fitResult());
    await second;
    expect(fitSection(settledPanel(props)).loading).toBe(false);
    expect(fitSection(pass(props)).run).not.toBeNull();
  });

  it("does not revive a fit after its parameters change and return to their original values", async () => {
    const pending = deferred<LineFitResult>();
    backend.fit.mockReturnValue(pending.promise);
    const props = prepared();
    const running = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    (fitSection(pass(props)).onSnrText as (v: string) => void)("5");
    settledPanel(props);
    (fitSection(pass(props)).onSnrText as (v: string) => void)("3");
    settledPanel(props);
    pending.resolve(fitResult());
    await running;
    expect(fitSection(settledPanel(props)).run).toBeNull();
    expect(props.onCubeResult).not.toHaveBeenCalled();
  });

  it("invalidates all fit products after an input edit but preserves snapshots on unmount", async () => {
    const invalidated = vi.fn();
    const props = { ...prepared(), onInvalidateCubeResult: invalidated };
    await (fitSection(pass(props)).onFit as () => Promise<void>)();
    settledPanel(props);
    (fitSection(pass(props)).onUseDq as (v: boolean) => void)(false);
    settledPanel(props);
    expect(invalidated).toHaveBeenCalledExactlyOnceWith(["C:/out/fit.fits"]);
    invalidated.mockClear();
    await (fitSection(pass(props)).onFit as () => Promise<void>)();
    settledPanel(props);
    hooks.mount();
    expect(invalidated).not.toHaveBeenCalled();
  });

  it("keeps a completed fit when only the plot display mode changes", async () => {
    const props = prepared();
    await (fitSection(pass(props)).onFit as () => Promise<void>)();
    const before = fitSection(pass(props)).run;
    (axisSection(pass(props)).onModeChange as (v: string) => void)("frequency");
    expect(fitSection(settledPanel(props)).run).toBe(before);
  });

  it("does not let an older fit replace a more recently requested moment map", async () => {
    const pending = deferred<LineFitResult>();
    backend.fit.mockReturnValue(pending.promise);
    const props = prepared();
    const fitting = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    await clickCalculation(pass(props), "Moments");
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
    pending.resolve(fitResult());
    await fitting;
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
    expect(fitSection(settledPanel(props)).run).not.toBeNull();
  });

  it("does not let pending moments replace a manually selected fit plane", async () => {
    const props = prepared();
    await (fitSection(pass(props)).onFit as () => Promise<void>)();
    const pending = deferred<MomentMapsResult>();
    backend.moments.mockReturnValue(pending.promise);
    const running = clickCalculation(pass(props), "Moments");
    await Promise.resolve();
    (fitSection(pass(props)).onShowPlane as (plane: string) => void)("velocity");
    expect(props.onCubeResult).toHaveBeenCalledTimes(2);
    pending.resolve(momentResult({ m0: { png_path: "m.png", fits_path: "m.fits", previewUrl: "asset://m.png" } }));
    await running;
    expect(props.onCubeResult).toHaveBeenCalledTimes(2);
  });

  it("does not let an older collapse replace a newer result from the RAMP panel", async () => {
    const pending = deferred<CollapseRangeResult>();
    backend.collapseRange.mockReturnValue(pending.promise);
    const props = prepared();
    const running = clickCalculation(pass(props), "Collapse Mean");
    await Promise.resolve();
    const ramp = framePublishers(props.publishGate, props.onCubeResult);
    ramp.result({ fitsPath: "slope.fits", previewUrl: "slope.png", label: "Quick slope", dimensions: [8, 8] });
    pending.resolve(collapseResult({ previewUrl: "asset://collapse.png" }));
    await running;
    expect(props.onCubeResult).toHaveBeenCalledTimes(1);
    expect(props.onCubeResult).toHaveBeenCalledWith(expect.objectContaining({ fitsPath: "slope.fits" }));
  });

  it("does not let a pending fit replace a newer committed frame", async () => {
    const pending = deferred<LineFitResult>();
    backend.fit.mockReturnValue(pending.promise);
    const props = prepared();
    const running = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    props.publishGate.commit();
    pending.resolve(fitResult());
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(fitSection(settledPanel(props)).run).not.toBeNull();
  });

  it("keeps an older fit from publishing while a newer RAMP calculation is pending", async () => {
    const pending = deferred<LineFitResult>();
    backend.fit.mockReturnValue(pending.promise);
    const props = prepared();
    const running = (fitSection(pass(props)).onFit as () => Promise<void>)();
    await Promise.resolve();
    const gate = props.publishGate;
    const rampTicket = gate.reserve();
    pending.resolve(fitResult());
    await running;
    expect(props.onCubeResult).not.toHaveBeenCalled();
    expect(gate.isLatest(rampTicket)).toBe(true);
    expect(fitSection(settledPanel(props)).run).not.toBeNull();
  });

  const pickCircle = () => {
    regionDocState.regions = [
      { id: "r1", shape: REGION_CIRCLE, props: { color: null, width: null, text: null, dash: null, include: true }, backgroundId: null } as Region,
    ];
  };

  const pressRegion = (tree: ReactNode) => (mustFind(
    tree,
    (p) => typeof p.title === "string" && p.title.startsWith("circle region") && typeof p.onClick === "function",
    "region spectrum button",
  ).onClick as () => Promise<void>)();

  it("carries the region request token through success and failure", async () => {
    pickCircle();
    const props = prepared();
    const result = regionWithErrors();
    backend.regionSpectrum.mockResolvedValueOnce(result).mockRejectedValueOnce(new Error("region failed"));
    await pressRegion(pass(props));
    expect(spectrumActions.commit).toHaveBeenCalledWith(result, expect.objectContaining({ kind: "region" }), null, 41);
    await pressRegion(pass(props));
    expect(spectrumActions.fail).toHaveBeenCalledExactlyOnceWith("region failed", 41);
  });

  it.each(["resolve", "reject"])("discards a pending region %s after its geometry changes", async (outcome) => {
    pickCircle();
    const props = prepared();
    const pending = deferred<RegionSpectrum>();
    backend.regionSpectrum.mockReturnValue(pending.promise);
    const running = pressRegion(pass(props));
    regionDocState.regions = [
      { ...(regionDocState.regions[0] as Region), shape: { shape: "circle", x: 5, y: 4, r: 2 } },
    ];
    settledPanel(props);
    if (outcome === "resolve") pending.resolve(regionWithErrors());
    else pending.reject(new Error("old geometry failed"));
    await running;
    expect(spectrumActions.clear).toHaveBeenCalled();
    expect(spectrumActions.commit).not.toHaveBeenCalled();
    expect(spectrumActions.fail).not.toHaveBeenCalled();
  });
});

describe("SpectroscopyPanel product of an earlier run on the viewer", () => {
  const OLD_MOMENT = {
    kind: "cube",
    label: "Moment m0 · 35 ch",
    fitsPath: "C:/out/moments-1/cube_moment0_5-39.fits",
    previewUrl: "asset://old_m0.png",
    dimensions: [8, 8],
    inputPath: "C:/data/cube.fits",
  };
  const reopened = (tree: ReactNode) => findProps(tree, (_p, type) => type === ReopenedProductNote);

  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
    backend.moments.mockReset().mockResolvedValue(momentResult({ m0: { png_path: "C:/out/m.png", fits_path: "C:/out/m.fits", previewUrl: "asset://m.png" } }));
  });
  afterEach(() => {
    renderState.processed = null;
  });

  it("names a product of an earlier run on the viewer and offers Show cube", () => {
    renderState.processed = OLD_MOMENT;
    const props = { ...cubeProps(), onInvalidateCubeResult: vi.fn() };
    const note = reopened(settledPanel(props));
    expect(note?.label).toBe("Moment m0 · 35 ch");
    (note?.onShowCube as () => void)();
    expect(props.onInvalidateCubeResult).toHaveBeenCalledExactlyOnceWith([OLD_MOMENT.fitsPath]);
  });

  it("names the product in the empty panel before a spectrum is extracted", () => {
    renderState.processed = { ...OLD_MOMENT, label: "Line fit Velocity · ch 5-39 · 64 px", fitsPath: "C:/out/fit-1/velocity.fits" };
    const tree = settledPanel({ ...cubeProps(), spectrum: [], pixelCoord: null });
    expect(findProps(tree, (p) => p.children === "Click on the preview image to extract a spectrum")).not.toBeNull();
    expect(reopened(tree)?.label).toBe("Line fit Velocity · ch 5-39 · 64 px");
  });

  it.each([
    ["the panel title", () => cubeProps(), "Spectroscopy"],
    ["the hint of the empty panel", () => ({ ...cubeProps(), spectrum: [], pixelCoord: null }), "Click on the preview image to extract a spectrum"],
  ] as [string, () => Record<string, unknown>, string][])("hands focus to %s when Show cube removes the note", (_name, props, text) => {
    renderState.processed = OLD_MOMENT;
    const tree = settledPanel(props());
    const target = findProps(tree, (p) => p.children === text);
    expect(target?.tabIndex).toBe(-1);
    const element = { focus: vi.fn() };
    (target?.ref as (element: unknown) => void)(element);
    expect((reopened(tree)?.focusAfterShowCube as { current: unknown } | undefined)?.current).toBe(element);
  });

  it("does not name the product of the result the panel shows", async () => {
    const props = cubeProps();
    settledPanel(props);
    commitRangeStart(pass(props), 5);
    settledPanel(props);
    await clickCalculation(pass(props), "Moments");
    renderState.processed = { ...OLD_MOMENT, fitsPath: "C:/out/m.fits", previewUrl: "asset://m.png" };
    expect(reopened(settledPanel(props))).toBeNull();
  });

  it.each([
    ["a channel preview", { ...OLD_MOMENT, label: "ch 5", frameIndex: 5 }],
    ["a quick slope of the RAMP section", { ...OLD_MOMENT, label: "Quick slope (DN/s)" }],
    ["a product without a FITS file", { ...OLD_MOMENT, fitsPath: null }],
    ["a product of another file", { ...OLD_MOMENT, inputPath: "C:/data/other.fits" }],
  ])("does not name %s", (_name, processed) => {
    renderState.processed = processed;
    expect(reopened(settledPanel(cubeProps()))).toBeNull();
  });

  it("does not name a product in a spectrum file", () => {
    renderState.processed = { ...OLD_MOMENT, inputPath: DOC_PATH };
    cubeState.ramp = null;
    const tree = settledPanel({ documentMode: true, spectrumDocument: documentOf(), filePath: DOC_PATH, publishGate: createFramePublishGate() });
    expect(reopened(tree)).toBeNull();
  });

  it("hides the note while a run that will replace the product is in progress", async () => {
    renderState.processed = OLD_MOMENT;
    const pending = deferred<MomentMapsResult>();
    backend.moments.mockReturnValue(pending.promise);
    const props = cubeProps();
    settledPanel(props);
    commitRangeStart(pass(props), 5);
    expect(reopened(settledPanel(props))).not.toBeNull();
    const running = clickCalculation(pass(props), "Moments");
    await Promise.resolve();
    expect(reopened(settledPanel(props))).toBeNull();
    pending.resolve(momentResult());
    await running;
  });
});

type PathOp = ["M" | "L", number, number];

function strokeCanvas() {
  const strokes: { style: unknown; ops: PathOp[] }[] = [];
  const ticks: string[] = [];
  let ops: PathOp[] = [];
  const ctx = new Proxy({} as Record<string | symbol, unknown>, {
    get: (target, key) => {
      if (key === "beginPath") return () => (ops = []);
      if (key === "moveTo") return (x: number, y: number) => ops.push(["M", x, y]);
      if (key === "lineTo") return (x: number, y: number) => ops.push(["L", x, y]);
      if (key === "stroke") return () => strokes.push({ style: target.strokeStyle, ops });
      if (key === "fillText") {
        return (text: string) => {
          if (target.textAlign === "right") ticks.push(text);
        };
      }
      return key in target ? target[key] : key === "measureText" ? () => ({ width: 10 }) : () => {};
    },
    set: (target, key, value) => {
      target[key] = value;
      return true;
    },
  });
  const canvas = { width: 0, height: 0, parentElement: { getBoundingClientRect: () => ({ width: 400 }) }, getContext: () => ctx };
  return { canvas, strokes, lastTicks: () => ticks.slice(-5).map(Number) };
}

const SERIES_STROKE = "#a855f7";
const PLOT_BOTTOM = SPECTRUM_PLOT_HEIGHT - SPECTRUM_PLOT_PAD.bottom;
const PLOT_SPAN = 400 - SPECTRUM_PLOT_PAD.left - SPECTRUM_PLOT_PAD.right;

function subpaths(ops: PathOp[]): PathOp[][] {
  return ops.reduce<PathOp[][]>((paths, op) => {
    if (op[0] === "M" || paths.length === 0) paths.push([op]);
    else paths[paths.length - 1].push(op);
    return paths;
  }, []);
}

function channelPx(i: number): number {
  return SPECTRUM_PLOT_PAD.left + ((AIR_VALUES[i] - AIR_VALUES[0]) / (AIR_VALUES[N_DOC - 1] - AIR_VALUES[0])) * PLOT_SPAN;
}

function drawnDocument(props: Props) {
  const plot = strokeCanvas();
  (mustFind(renderPlot(pass(props)), (p) => p.className === "w-full rounded-md cursor-crosshair", "plot canvas").ref as { current: unknown }).current = plot.canvas;
  spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "vacuum" });
  renderPlot(pass(props));
  const series = plot.strokes.filter((s) => s.style === SERIES_STROKE).at(-1);
  const dqTicks = plot.strokes.filter((s) => s.ops.length > 0 && s.ops.every((op) => (op[0] === "M" ? op[2] === PLOT_BOTTOM : op[2] < PLOT_BOTTOM)));
  return { series: series ? subpaths(series.ops) : [], dqTicks: dqTicks.at(-1)?.ops.filter((op) => op[0] === "M") ?? [], ticks: plot.lastTicks() };
}

function readoutCanvas() {
  const texts: string[] = [];
  const ctx = new Proxy({} as Record<string | symbol, unknown>, {
    get: (target, key) => {
      if (key === "fillText") return (text: string) => texts.push(text);
      return key in target ? target[key] : key === "measureText" ? () => ({ width: 10 }) : () => {};
    },
    set: (target, key, value) => {
      target[key] = value;
      return true;
    },
  });
  return { canvas: { width: 400, height: SPECTRUM_PLOT_HEIGHT, getContext: () => ctx }, texts };
}

function hoverReadout(props: Props, channel: number): string | undefined {
  const tree = renderPlot(pass(props));
  const overlay = readoutCanvas();
  (mustFind(tree, (p) => p.className === "absolute inset-0 w-full pointer-events-none", "overlay canvas").ref as { current: unknown }).current =
    overlay.canvas;
  const plot = mustFind(tree, (p) => p.className === "w-full rounded-md cursor-crosshair", "plot canvas");
  (plot.ref as { current: unknown }).current = { width: 400, height: SPECTRUM_PLOT_HEIGHT, getBoundingClientRect: () => ({ left: 0, width: 400 }) };
  (plot.onMouseMove as (e: unknown) => void)({ clientX: SPECTRUM_PLOT_PAD.left + (PLOT_SPAN * channel) / (N_DOC - 1) });
  renderPlot(pass(props));
  return overlay.texts.filter((text) => new RegExp(`^ch ${channel}[ ,]`).test(text)).at(-1);
}

const FAINT_FLUX = AIR_VALUES.map((_, i) => 1e-17 * (1 + Math.exp(-((i - 20) ** 2) / 8)));

describe("SpectroscopyPanel plot of a spectrum file", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
  });

  const docProps = (doc: SpectrumDocument): Props => ({ documentMode: true, spectrumDocument: doc, filePath: DOC_PATH, publishGate: createFramePublishGate() });

  it("labels the y axis of an F_lambda spectrum with its own range", () => {
    const props = docProps(documentOf({ flux: FAINT_FLUX }));
    const plot = yTickCanvas();
    (mustFind(renderPlot(pass(props)), (p) => p.className === "w-full rounded-md cursor-crosshair", "plot canvas").ref as { current: unknown }).current = plot.canvas;
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "vacuum" });
    renderPlot(pass(props));
    const ticks = plot.lastTicks();
    expect(ticks[4]).toBeGreaterThan(0);
    expect(ticks[4]).toBeCloseTo(9.5e-18, 20);
    expect(ticks[0]).toBeCloseTo(2.05e-17, 19);
  });

  it("lifts the pen across non-finite flux instead of bridging the gap with a straight line", () => {
    const flux = AIR_VALUES.map((_, i) => (i >= 10 && i <= 14 ? NaN : 1 + Math.exp(-((i - 25) ** 2) / 8)));
    const { series } = drawnDocument(docProps(documentOf({ flux })));
    expect(series.map((path) => path.length)).toEqual([10, 25]);
    expect(series[0].at(-1)![1]).toBeCloseTo(channelPx(9), 6);
    expect(series[1][0][1]).toBeCloseTo(channelPx(15), 6);
  });

  it("leaves DQ-flagged rows out of the line and of the y range, and marks them at the bottom edge", () => {
    const flux = AIR_VALUES.map((_, i) => (i >= 10 && i <= 14 ? 0 : 5 + Math.exp(-((i - 25) ** 2) / 8)));
    const dq = AIR_VALUES.map((_, i) => (i >= 10 && i <= 14 ? 1 : 0));
    const drawn = drawnDocument(docProps(documentOf({ flux, dq, dq_table: "jwst" })));
    expect(drawn.series.map((path) => path.length)).toEqual([10, 25]);
    expect(drawn.ticks[4]).toBeGreaterThan(4.9);
    expect(drawn.dqTicks).toHaveLength(5);
    drawn.dqTicks.forEach((op, k) => expect(op[1]).toBeCloseTo(Math.round(channelPx(10 + k)) + 0.5, 6));
  });

  it("draws the flagged rows as data when DQ masking is switched off, as the measurement then uses them", () => {
    const flux = AIR_VALUES.map((_, i) => (i >= 10 && i <= 14 ? 0 : 5 + Math.exp(-((i - 25) ** 2) / 8)));
    const dq = AIR_VALUES.map((_, i) => (i >= 10 && i <= 14 ? 1 : 0));
    const props = docProps(documentOf({ flux, dq, dq_table: "jwst" }));
    pass(props);
    (lineSection(pass(props)).errControls as { onUseDq: (v: boolean) => void }).onUseDq(false);
    const drawn = drawnDocument(props);
    expect(drawn.series.map((path) => path.length)).toEqual([40]);
    expect(drawn.ticks[4]).toBeLessThan(0);
    expect(drawn.dqTicks).toHaveLength(0);
  });

  it("draws flagged rows as data when the DQ flag table is unknown, as the measurement does", () => {
    const flux = AIR_VALUES.map((_, i) => (i >= 10 && i <= 14 ? 0 : 5));
    const dq = AIR_VALUES.map((_, i) => (i >= 10 && i <= 14 ? 1 : 0));
    const drawn = drawnDocument(docProps(documentOf({ flux, dq, dq_table: "unknown" })));
    expect(drawn.series.map((path) => path.length)).toEqual([40]);
  });

  it("prints the hovered flux of a faint spectrum with its error and unit", () => {
    const err = AIR_VALUES.map(() => 5e-19);
    const readout = hoverReadout(docProps(documentOf({ flux: FAINT_FLUX, err })), 20);
    expect(readout).toMatch(/^ch 20 · 0\.\d{4} μm → 2\.000e-17 ± 5\.000e-19 erg\/s\/cm2\/A$/);
  });

  it("prints the hovered flux without an error when the file has none, and native when it has no unit", () => {
    const readout = hoverReadout(docProps(documentOf({ flux: FAINT_FLUX, err: null, flux_unit: null })), 20);
    expect(readout).toMatch(/^ch 20 · 0\.\d{4} μm → 2\.000e-17 native$/);
  });

  it("prints the hovered wavelength to the channel spacing of a high-resolution spectrum", () => {
    const fine = AIR_VALUES.map((_, i) => 0.658 + i * 2e-6);
    const doc = documentOf({ wavelength_frame: "vacuum", axis: { ...airAxis("wave"), values: fine, crval: fine[0], cdelt: 2e-6 } });
    expect(hoverReadout(docProps(doc), 20)).toMatch(/^ch 20 · 0\.658040 μm → /);
    expect(hoverReadout(docProps(doc), 21)).toMatch(/^ch 21 · 0\.658042 μm → /);
  });

  it("states an assumed frame and the one air and vacuum formula", () => {
    const assumed = documentOf({
      notes: [
        "CTYPE1 'LINEAR' with no JWST, HST or Roman TELESCOP and no AIRORVAC or VACUUM card naming the frame: air wavelengths assumed, the ground-based default",
      ],
    });
    const html = renderDocument(assumed);
    expect(html).toContain("air wavelengths, assumed (no card of the file names the frame)");
    expect(html).toContain("air and vacuum wavelengths are converted with Morton (2000), for the plot and for line velocities");
    expect(html).not.toContain("Greisen");
  });
});

describe("SpectroscopyPanel hover readout of a cube", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
  });

  afterEach(() => {
    spectrumState.region = null;
    spectrumState.regionSource = null;
  });

  it("prints a pixel spectrum's flux in the BUNIT of the cube", () => {
    expect(hoverReadout(cubeProps(), 20)).toBe("ch 20, +20 from displayed · 0.6575 μm → 1.000 DN");
  });

  it("prints a region spectrum's flux with the error of the shown series in the unit of the export", () => {
    showRegion(regionWithErrors(null));
    expect(hoverReadout(cubeProps(), 20)).toBe("ch 20, +20 from displayed · 0.6575 μm → 11.00 ± 0.2200 DN x pix");
  });
});

describe("SpectroscopyPanel region pick", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    backend.regionSpectrum.mockReset().mockResolvedValue(regionWithErrors(null));
  });

  afterEach(() => {
    regionDocState.regions = [];
    spectrumState.region = null;
    spectrumState.regionSource = null;
  });

  it("falls back to the first included area region, not to an exclude region drawn before it", async () => {
    const box: RegionShape = { shape: "box", x: 2, y: 2, width: 2, height: 2, angle: 0 };
    regionDocState.regions = [
      { id: "r0", shape: box, props: { color: null, width: null, text: null, dash: null, include: false }, backgroundId: null } as Region,
      { id: "r1", shape: REGION_CIRCLE, props: { color: null, width: null, text: null, dash: null, include: true }, backgroundId: null } as Region,
    ];
    const props = cubeProps();
    pass(props);
    const button = mustFind(
      pass(props),
      (p) => typeof p.title === "string" && p.title.includes(" region") && typeof p.onClick === "function",
      "region spectrum button",
    );
    await (button.onClick as () => Promise<void>)();
    expect(backend.regionSpectrum).toHaveBeenLastCalledWith("C:/data/cube.fits", REGION_CIRCLE, null, null);
  });
});

describe("SpectroscopyPanel line measurement at a redshift", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectralMock.measure.mockReset().mockImplementation(async (_path, source) => measured(source as LineMeasurement["source"]));
    logMock.append.mockReset();
  });

  const docProps = (): Props => ({ documentMode: true, spectrumDocument: documentOf(), filePath: DOC_PATH, publishGate: createFramePublishGate() });
  const lineList = (tree: ReactNode) => mustFind(tree, (p) => typeof p.onRedshiftChange === "function", "line list controls");

  it("sends the redshift of the line list, zero included", async () => {
    await brushAndMeasure(docProps());
    expect(spectralMock.measure.mock.calls[0][2]).toMatchObject({ redshift: 0 });
    const props = docProps();
    hooks.run.cells = [];
    pass(props);
    (lineList(pass(props)).onRedshiftChange as (z: number) => void)(0.5);
    commitRangeStart(pass(props), 5);
    await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    expect(spectralMock.measure.mock.calls[1][2]).toMatchObject({ redshift: 0.5 });
  });

  it("sends the redshift for a cube spaxel too", async () => {
    const props = cubeProps();
    pass(props);
    (lineList(pass(props)).onRedshiftChange as (z: number) => void)(2.0);
    commitRangeStart(pass(props), 5);
    await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    expect(spectralMock.measure.mock.calls[0][2]).toMatchObject({ redshift: 2.0 });
  });

  it("clears a measurement after the redshift changes, as it does for every other request input", async () => {
    const props = docProps();
    await brushAndMeasure(props);
    expect(lineSection(pass(props)).result).not.toBeNull();
    (lineList(pass(props)).onRedshiftChange as (z: number) => void)(0.5);
    expect(lineSection(settledPanel(props)).result).toBeNull();
  });
});

describe("SpectroscopyPanel frame navigation before a spectrum", () => {
  const SHOWN_CHANNEL = 30;
  const CHANNEL_ON_SCREEN = {
    kind: "cube",
    label: `ch ${SHOWN_CHANNEL}`,
    fitsPath: `C:/out/cube_frame_${SHOWN_CHANNEL}.fits`,
    previewUrl: `asset://cube_frame_${SHOWN_CHANNEL}.png`,
    dimensions: [8, 8],
    inputPath: "C:/data/cube.fits",
    frameIndex: SHOWN_CHANNEL,
  };
  const emptyPanel = (): Props => ({ ...cubeProps(), spectrum: [], pixelCoord: null });
  const frameNav = (tree: ReactNode) => findProps(tree, (p) => typeof p.onFrameRequest === "function");

  function frameNavSlot(tree: ReactNode): { root: unknown; type: unknown; key: string | null } | null {
    if (!isValidElement<Props>(tree)) return null;
    const children = ([] as ReactNode[]).concat(tree.props.children as ReactNode);
    const slot = children.find((child) => isValidElement<Props>(child) && frameNav(child) !== null);
    return isValidElement(slot) ? { root: tree.type, type: slot.type, key: slot.key } : null;
  }

  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    renderState.processed = CHANNEL_ON_SCREEN;
  });

  afterEach(() => {
    renderState.processed = null;
  });

  it("keeps Frame Navigation on the channel on screen when the cube comes back from the Files list without a spectrum", () => {
    const tree = settledPanel(emptyPanel());
    expect(findProps(tree, (p) => p.children === "Click on the preview image to extract a spectrum")).not.toBeNull();
    const nav = frameNav(tree);
    expect(nav, "Frame Navigation registers the frame requests of the viewer's stepper and arrow keys").not.toBeNull();
    expect(nav).toMatchObject({ filePath: "C:/data/cube.fits", totalFrames: N_DOC, frame: SHOWN_CHANNEL });
  });

  it("keeps the same Frame Navigation when the first spectrum arrives, so a frame load or playback in progress goes on", () => {
    const empty = frameNavSlot(settledPanel(emptyPanel()));
    hooks.run.cells = [];
    const withSpectrum = frameNavSlot(settledPanel(cubeProps()));
    expect(empty?.key, "React keeps a child of the same root across the two layouts only by its key").toEqual(expect.any(String));
    expect(withSpectrum).toEqual(empty);
  });
});

describe("SpectroscopyPanel plot title, file error band and keys", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
  });

  afterEach(() => {
    spectrumState.region = null;
    spectrumState.regionSource = null;
  });

  const docProps = (doc: SpectrumDocument): Props => ({ documentMode: true, spectrumDocument: doc, filePath: DOC_PATH, publishGate: createFramePublishGate() });
  const freshModel = (props: Props): SpectrumPlotModel => {
    hooks.run.cells = [];
    return spectrumPlotOf(pass(props)).model as SpectrumPlotModel;
  };
  const plotContainer = (tree: ReactNode) =>
    mustFind(tree, (p) => p.className === "p-2 rounded-md outline-none focus-visible:ring-1 focus-visible:ring-violet-500/50", "plot container");

  it("gives a spectrum file its error band without letting the errors set the y range", () => {
    const err = AIR_VALUES.map(() => 0.05);
    const doc = documentOf({ err });
    const model = freshModel(docProps(doc));
    expect(model.errors).toEqual({ err, scaled: null });
    expect(model.errorBand).toBe("propagated");
    const html = renderDocument(doc);
    expect(html).toContain('data-error-band="propagated"');
    expect(html).toContain("error band: ±1 sigma from the file (ERR extension)");

    const bare = documentOf({ err: null });
    expect(freshModel(docProps(bare)).errors).toBeNull();
    const bareHtml = renderDocument(bare);
    expect(bareHtml).toContain('data-error-band="none"');
    expect(bareHtml).not.toContain("sigma from the file");

    const spike = documentOf({ err: AIR_VALUES.map((_, i) => (i === 20 ? 1e6 : 0.05)) });
    const spiked = freshModel(docProps(spike));
    expect(spiked.errorBand).toBe("propagated");
    expect(spiked.yMax).toBeLessThan(3);

    const x1d = documentOf({ source: { kind: "x1d", path: DOC_PATH, hdu: 1, extver: null } });
    expect(renderDocument(x1d)).toContain("error band: ±1 sigma from the file (FLUX_ERROR column)");
    const csv = documentOf({ source: { kind: "csv", path: "C:/data/ha.csv" } });
    expect(renderDocument(csv)).toContain("error band: ±1 sigma from the file (err column)");
  });

  it("titles the y axis with the unit of the readout", () => {
    expect(freshModel(docProps(documentOf())).yLabel).toBe("Flux (erg/s/cm2/A)");
    expect(freshModel(docProps(documentOf({ flux_unit: null }))).yLabel).toBe("Flux (native)");
    expect(freshModel(cubeProps()).yLabel).toBe("Flux (DN)");
    showRegion(regionWithErrors(null, { flux_jy: AIR_VALUES.map(() => 1e-3) }));
    const props = cubeProps();
    expect(freshModel(props).yLabel).toBe("Flux (DN x pix)");
    const view = (label: string) => {
      (mustFind(pass(props), (p) => p.children === label, `${label} series button`).onClick as () => void)();
      return (spectrumPlotOf(pass(props)).model as SpectrumPlotModel).yLabel;
    };
    expect(view("mean")).toBe("Flux (DN)");
    expect(view("Jy")).toBe("Flux (Jy)");
    expect(view("sum")).toBe("Flux (DN x pix)");
  });

  it("makes the plot the one tab stop of a spectrum file and keeps the frame keys on a cube", () => {
    const docTree = pass(docProps(documentOf()));
    const docPlot = spectrumPlotOf(docTree);
    expect(docPlot.cursorKeys).toBe(true);
    expect(docPlot.keyHint).toBe(
      "Left and Right arrows move the channel cursor, Shift moves ten channels, Home and End go to the ends, Escape clears it.",
    );
    expect(docPlot.status).toBeNull();
    expect(plotContainer(docTree).tabIndex).toBeUndefined();
    expect(plotContainer(docTree).onKeyDown).toBeUndefined();

    hooks.run.cells = [];
    const cubeTree = pass(cubeProps());
    const cubePlot = spectrumPlotOf(cubeTree);
    const frameLabel = mustFind(cubeTree, (p) => typeof p.onFrameRequest === "function", "frame navigation").frameLabel;
    expect(frameLabel).toEqual(expect.any(String));
    expect(cubePlot.cursorKeys).toBe(false);
    expect(cubePlot.keyHint).toBe(FRAME_KEY_HINT);
    expect(cubePlot.status).toBe(frameLabel);
    expect(plotContainer(cubeTree).tabIndex).toBe(0);
    expect(plotContainer(cubeTree).onKeyDown).toEqual(expect.any(Function));
    expect(plotContainer(cubeTree).title).toBe(FRAME_KEY_HINT);
  });
});

describe("SpectroscopyPanel view store", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
    spectralMock.measure.mockReset().mockImplementation(async (_path, source) => measured(source as LineMeasurement["source"]));
    logMock.append.mockReset();
    backend.cubeSpectrum.mockReset().mockResolvedValue({ flux_jy: null });
    backend.pixelToWorld.mockReset().mockResolvedValue({ points: [] });
    csvInputs.calls = [];
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  const docProps = (filePath: string = DOC_PATH): Props => ({
    documentMode: true,
    spectrumDocument: documentOf(),
    filePath,
    publishGate: createFramePublishGate(),
  });
  const lineList = (tree: ReactNode) => mustFind(tree, (p) => typeof p.onRedshiftChange === "function", "line list controls");
  const CHOSEN = { mode: "velocity", restValue: 0.656461, redshift: 0.05, range: { z0: 5, z1: N_DOC - 1 } };
  const DEFAULTS = { mode: "wavelength_air", restValue: null, redshift: 0, range: null };
  const NATIVE_DISPLAY = { wavelengthUnit: "um", fluxUnit: "native", binFactor: 1, dqExcluded: null };

  function choose(props: Props): void {
    pass(props);
    (axisSection(pass(props)).onModeChange as (mode: string) => void)("velocity");
    (axisSection(pass(props)).onRestChange as (restUm: number) => void)(0.656461);
    (lineList(pass(props)).onRedshiftChange as (z: number) => void)(0.05);
    commitRangeStart(pass(props), 5);
  }

  function mountedAgain(props: Props) {
    hooks.run.cells = [];
    const tree = pass(props);
    const axis = axisSection(tree);
    return { mode: axis.mode, restValue: axis.restValue, redshift: lineList(tree).redshift, range: spectrumPlotOf(tree).range };
  }

  async function exportedInput(props: Props): Promise<{ software?: string | null; display?: unknown }> {
    csvInputs.calls = [];
    logMock.append.mockClear();
    (mustFind(pass(props), (p) => p.children === "Copy CSV", "Copy CSV button").onClick as () => void)();
    await vi.waitFor(() => expect(logMock.append).toHaveBeenCalledTimes(1));
    expect(csvInputs.calls).toHaveLength(1);
    return csvInputs.calls[0] as { software?: string | null; display?: unknown };
  }

  it("keeps the choices of a spectrum when its panel is mounted again", () => {
    const props = docProps();
    choose(props);
    expect(mountedAgain(props)).toEqual(CHOSEN);
  });

  it("keeps two spectra apart", () => {
    choose(docProps());
    expect(mountedAgain(docProps("C:/data/other.fits"))).toEqual(DEFAULTS);
    expect(mountedAgain(docProps())).toEqual(CHOSEN);
  });

  it("stores a cube's choices under the cube key", () => {
    const props = cubeProps();
    pass(props);
    commitRangeStart(pass(props), 5);
    expect(spectrumViewStore.get(CUBE_VIEW_KEY).range).toEqual({ z0: 5, z1: N_DOC - 1 });
  });

  it("takes the key it is given", () => {
    const viewKey = "NRS1::median|C:/d/x.fits";
    const props = { ...docProps(), viewKey };
    pass(props);
    commitRangeStart(pass(props), 5);
    expect(spectrumViewStore.get(viewKey).range).toEqual({ z0: 5, z1: N_DOC - 1 });
    expect(spectrumViewStore.get(DOC_VIEW_KEY).range).toBeNull();
  });

  it("drops a stored range that no longer fits the series", () => {
    spectrumViewStore.patch(DOC_VIEW_KEY, {
      range: { z0: 5, z1: 400 },
      windows: [
        [0, 4],
        [390, 399],
      ],
    });
    const plot = spectrumPlotOf(pass(docProps()));
    expect(plot.range).toBeNull();
    expect(plot.windows).toBeNull();
    expect(spectrumViewStore.get(DOC_VIEW_KEY).range).toBeNull();
    expect(spectrumViewStore.get(DOC_VIEW_KEY).windows).toBeNull();
  });

  it("publishes the measured line for the viewer and takes it back", async () => {
    const props = docProps();
    await brushAndMeasure(props);
    const result = lineSection(pass(props)).result;
    expect(result).not.toBeNull();
    const overlay = spectrumViewStore.overlay(DOC_VIEW_KEY);
    expect(overlay?.result).toBe(result);
    expect(overlay?.measurementAxis).toEqual(AIR_VALUES);
    commitRangeStart(pass(props), 6);
    settledPanel(props);
    expect(spectrumViewStore.overlay(DOC_VIEW_KEY)).toBeNull();
    await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
    pass(props);
    expect(spectrumViewStore.overlay(DOC_VIEW_KEY)).not.toBeNull();
    hooks.mount();
    expect(spectrumViewStore.overlay(DOC_VIEW_KEY)).toBeNull();
  });

  it("gives the plot of a document exactly what documentPlot builds", () => {
    const props = docProps();
    const doc = props.spectrumDocument as SpectrumDocument;
    const expectBuilt = () => {
      const plot = spectrumPlotOf(pass(props));
      const built = documentPlot(doc.spectrum, spectrumViewStore.get(DOC_VIEW_KEY));
      expect(plot.model).toEqual(built.model);
      expect(plot.marks).toEqual(built.marks);
    };
    expectBuilt();
    spectrumViewStore.patch(DOC_VIEW_KEY, { mode: "wavelength_air", useDq: false });
    expectBuilt();
  });

  it("passes the view and the software name to the export", async () => {
    const writeText = vi.fn(async (text: string) => {
      void text;
    });
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    vi.stubGlobal("window", { setTimeout: () => 1, clearTimeout: () => {} });
    const fromDocument = await exportedInput(docProps());
    expect(fromDocument.software).toMatch(/^AstroBurst v\d+\.\d+\.\d+/);
    expect(fromDocument.display).toEqual(NATIVE_DISPLAY);
    hooks.run.cells = [];
    const cube = cubeProps();
    pass(cube);
    expect((await exportedInput(cube)).display).toEqual(NATIVE_DISPLAY);
    spectrumViewStore.patch(CUBE_VIEW_KEY, { wavelengthUnit: "nm", fluxUnit: "mJy", binFactor: 4 });
    expect((await exportedInput(cube)).display).toEqual({ wavelengthUnit: "nm", fluxUnit: "native", binFactor: 1, dqExcluded: null });
  });

  it("mounts the view controls for a cube too, with no document", () => {
    const tree = pass(cubeProps());
    const controls = mustFind(tree, (_p, type) => type === SpectrumViewControls, "view controls");
    expect(controls.spectrum).toBeNull();
    expect(controls.viewKey).toBe(CUBE_VIEW_KEY);
    expect(controls.model).toBe(spectrumPlotOf(tree).model);
    expect(renderToStaticMarkup(createElement(SpectroscopyPanel, cubeProps() as never))).not.toContain("data-document-frame");
  });

  it("opens an air file on its own air axis", () => {
    const props = docProps();
    const shown = () => {
      const tree = settledPanel(props);
      const model = spectrumPlotOf(tree).model as SpectrumPlotModel;
      return { label: model.xLabel, values: model.xValues, mode: axisSection(tree).mode };
    };
    expect(shown()).toEqual({ label: "Air wavelength (μm)", values: AIR_VALUES, mode: "wavelength_air" });
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "vacuum" });
    expect(shown()).toEqual({ label: "Vacuum wavelength (μm)", values: AIR_VALUES, mode: "wavelength_vac" });
    spectrumViewStore.patch(DOC_VIEW_KEY, { mode: "frequency" });
    expect(shown().mode).toBe("frequency");
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "air" });
    expect(shown().mode).toBe("frequency");
    expect(spectrumViewStore.get(DOC_VIEW_KEY).mode).toBe("frequency");
    spectrumViewStore.patch(DOC_VIEW_KEY, { frameChoice: "vacuum" });
    expect(shown().mode).toBe("frequency");
  });

  const OTHER_PATH = "C:/data/other.fits";
  const OTHER_VIEW_KEY = documentViewKey(OTHER_PATH, "0");
  const CARRIED_NOTE = "line setup carried over from ha_bess.fits, 1D array in HDU 0";
  const CARRIED_ON_B = { ...CHOSEN, range: { z0: 0, z1: 34 } };

  function docPropsAt(filePath: string, values: number[]): Props {
    return {
      documentMode: true,
      spectrumDocument: documentOf({
        source: { kind: "wcs1d", path: filePath, hdu: 0 },
        axis: { ...airAxis(), values, crval: values[0] },
      }),
      filePath,
      publishGate: createFramePublishGate(),
    };
  }

  const propsB = () => docPropsAt(OTHER_PATH, AIR_VALUES.map((v) => v + 5e-4));

  function newMount(props: Props): ReactNode {
    hooks.run.cells = [];
    pass(props);
    return pass(props);
  }

  function setupShown(tree: ReactNode) {
    const axis = axisSection(tree);
    return { mode: axis.mode, restValue: axis.restValue, redshift: lineList(tree).redshift, range: spectrumPlotOf(tree).range };
  }

  const setupNote = (tree: ReactNode) => findProps(tree, (p) => "data-setup-from" in p);

  function noteText(tree: ReactNode): unknown {
    const note = setupNote(tree);
    return note === null ? null : findProps(note.children as ReactNode, (p) => typeof p.children === "string")?.children;
  }

  function chooseOnA(): Props {
    const propsA = docProps();
    choose(propsA);
    pass(propsA);
    return propsA;
  }

  it("carries the line setup to the next spectrum", () => {
    chooseOnA();
    const tree = newMount(propsB());
    expect(setupShown(tree)).toEqual(CARRIED_ON_B);
    expect(noteText(tree)).toBe(CARRIED_NOTE);
    const clear = mustFind(tree, (p) => "data-setup-clear" in p, "clear button");
    expect(clear.title).toBe("Reset the axis, rest wavelength, redshift, line families, model, range and windows of this spectrum");
    expect(clear.children).toBe("clear");
  });

  it("does not replace the line setup a spectrum already has, and ignores what is not a line setup", () => {
    spectrumViewStore.patch(OTHER_VIEW_KEY, { redshift: 0.2 });
    chooseOnA();
    const own = newMount(propsB());
    expect(lineList(own).redshift).toBe(0.2);
    expect(axisSection(own).mode).toBe("wavelength_air");
    expect(setupNote(own)).toBeNull();

    spectrumViewStore.reset();
    hooks.run.cells = [];
    spectrumViewStore.patch(OTHER_VIEW_KEY, { xDomain: [0.657, 0.658], binFactor: 2, fluxUnit: "uJy" });
    chooseOnA();
    const seeded = newMount(propsB());
    expect(setupShown(seeded)).toEqual(CARRIED_ON_B);
    expect(noteText(seeded)).toBe(CARRIED_NOTE);
    const stored = spectrumViewStore.get(OTHER_VIEW_KEY);
    expect(stored.binFactor).toBe(2);
    expect(stored.fluxUnit).toBe("uJy");
    expect(stored.xDomain).toBeNull();
  });

  it("clears a carried setup and does not bring it back", () => {
    chooseOnA();
    const props = propsB();
    const tree = newMount(props);
    (mustFind(tree, (p) => "data-setup-clear" in p, "clear button").onClick as () => void)();
    const cleared = pass(props);
    expect(setupShown(cleared)).toEqual(DEFAULTS);
    expect(setupNote(cleared)).toBeNull();
    const again = newMount(props);
    expect(setupShown(again)).toEqual(DEFAULTS);
    expect(setupNote(again)).toBeNull();
  });

  it("drops a range that lies outside the next spectrum", () => {
    chooseOnA();
    const tree = newMount(docPropsAt("C:/data/far.fits", AIR_VALUES.map((v) => v + 0.1)));
    expect(setupShown(tree)).toEqual({ ...CHOSEN, range: null });
    expect(spectrumPlotOf(tree).windows).toBeNull();
    expect(noteText(tree)).toBe(CARRIED_NOTE);
  });

  it("carries nothing into a cube and nothing out of one", () => {
    chooseOnA();
    const cube = cubeProps();
    const cubeTree = newMount(cube);
    expect(lineList(cubeTree).redshift).toBe(0);
    expect(setupNote(cubeTree)).toBeNull();
    (lineList(cubeTree).onRedshiftChange as (z: number) => void)(0.3);
    expect(lineList(pass(cube)).redshift).toBe(0.3);
    pass(cube);
    expect(lineList(newMount(propsB())).redshift).toBe(0.05);
  });

  it("shows the note again when the panel of a seeded spectrum is mounted again", () => {
    chooseOnA();
    const props = propsB();
    newMount(props);
    const again = newMount(props);
    expect(noteText(again)).toBe(CARRIED_NOTE);
    expect(setupShown(again)).toEqual(CARRIED_ON_B);
  });

  it("passes the setup on through a spectrum the user only looked at", () => {
    chooseOnA();
    newMount(propsB());
    const tree = newMount(docPropsAt("C:/data/third.fits", AIR_VALUES.map((v) => v + 5e-4)));
    expect(setupShown(tree)).toEqual(CARRIED_ON_B);
  });

  it("carries a cleared setup no further", () => {
    chooseOnA();
    const props = propsB();
    const tree = newMount(props);
    (mustFind(tree, (p) => "data-setup-clear" in p, "clear button").onClick as () => void)();
    pass(props);
    const third = newMount(docPropsAt("C:/data/third.fits", AIR_VALUES.map((v) => v + 5e-4)));
    expect(setupShown(third)).toEqual(DEFAULTS);
    expect(setupNote(third)).toBeNull();
  });

  it("carries no line setup to a spectrum of another instrument, disperser or filter", () => {
    const withCards = (props: Props, cards: Record<string, string>): Props => {
      const doc = props.spectrumDocument as SpectrumDocument;
      return { ...props, spectrumDocument: { ...doc, spectrum: { ...doc.spectrum, cards } } };
    };
    const G235H = { INSTRUME: "NIRSPEC", GRATING: "G235H", FILTER: "F170LP", DETECTOR: "NRS1" };
    const propsA = withCards(docProps(), G235H);
    choose(propsA);
    pass(propsA);
    const miri = newMount(withCards(propsB(), { INSTRUME: "MIRI", FILTER: "P750L", DETECTOR: "MIRIMAGE" }));
    expect(setupShown(miri)).toEqual(DEFAULTS);
    expect(setupNote(miri)).toBeNull();
    const bare = newMount(docPropsAt("C:/data/bare.fits", AIR_VALUES.map((v) => v + 5e-4)));
    expect(setupShown(bare)).toEqual(DEFAULTS);
    expect(setupNote(bare)).toBeNull();
    const nrs2 = newMount(withCards(docPropsAt("C:/data/nrs2.fits", AIR_VALUES.map((v) => v + 5e-4)), { ...G235H, DETECTOR: "NRS2" }));
    expect(setupShown(nrs2)).toEqual(CARRIED_ON_B);
    expect(noteText(nrs2)).toBe(CARRIED_NOTE);
  });
});

describe("SpectroscopyPanel range boxes in the unit of the axis", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
    backend.cubeSpectrum.mockReset().mockResolvedValue({ flux_jy: null });
    backend.pixelToWorld.mockReset().mockResolvedValue({ points: [] });
  });

  const docProps = (): Props => ({ documentMode: true, spectrumDocument: documentOf(), filePath: DOC_PATH, publishGate: createFramePublishGate() });
  const EDGES = ["range:z0", "range:z1", "window:0:0", "window:0:1", "window:1:0", "window:1:1"];

  function edgeBoxes(tree: ReactNode): Props[] {
    return EDGES.map((edge) => mustFind(tree, (p, type) => type === RangeEdgeField && p.edge === edge, edge));
  }
  const edgeBox = (tree: ReactNode, edge: string) => mustFind(tree, (p, type) => type === RangeEdgeField && p.edge === edge, edge);
  const unitButton = (tree: ReactNode, unit: string) => mustFind(tree, (p) => p["data-range-fields"] === unit, `${unit} button`);
  const unitGroup = (tree: ReactNode) => findProps(tree, (p) => p["aria-label"] === "Range field unit");

  it("gives the range and window boxes of a spectrum file the axis of the plot", () => {
    const props = docProps();
    pass(props);
    const tree = pass(props);
    const model = spectrumPlotOf(tree).model as SpectrumPlotModel;
    const boxes = edgeBoxes(tree);
    for (const box of boxes) {
      const mapping = box.mapping as { n: number; xValues: number[] | null };
      expect(mapping.n).toBe(N_DOC);
      expect(mapping.xValues).toBe(model.xValues);
      expect(box.unit).toBe(model.xUnit);
    }
    expect(boxes[0].axisLabel).toBe("Range first channel, in μm");
    expect(boxes[0].channelLabel).toBe("Range first channel z0");
    (edgeBox(pass(props), "range:z0").onCommit as (channel: number) => void)(5);
    expect(spectrumViewStore.get(DOC_VIEW_KEY).range).toEqual({ z0: 5, z1: N_DOC - 1 });
    (edgeBox(pass(props), "window:0:1").onCommit as (channel: number) => void)(3);
    expect(spectrumViewStore.get(DOC_VIEW_KEY).windows?.[0][1]).toBe(3);
  });

  it("switches the boxes back to channels and says which unit is on", () => {
    const props = docProps();
    pass(props);
    const tree = pass(props);
    const group = unitGroup(tree);
    expect(group?.role).toBe("group");
    expect(unitButton(group?.children as ReactNode, "channel").children).toBe("ch");
    expect(unitButton(group?.children as ReactNode, "axis").children).toBe("μm");
    expect(unitButton(tree, "channel")["aria-pressed"]).toBe(false);
    expect(unitButton(tree, "axis")["aria-pressed"]).toBe(true);
    (unitButton(tree, "channel").onClick as () => void)();
    const inChannels = pass(props);
    for (const box of edgeBoxes(inChannels)) expect(box.mapping).toBeNull();
    expect(unitButton(inChannels, "channel")["aria-pressed"]).toBe(true);
    expect(unitButton(inChannels, "axis")["aria-pressed"]).toBe(false);
    (unitButton(inChannels, "axis").onClick as () => void)();
    for (const box of edgeBoxes(pass(props))) expect(box.mapping).not.toBeNull();
  });

  it("keeps channel boxes for a spectrum without an axis", () => {
    spectralMock.axis.mockReset().mockRejectedValue(new Error("no spectral axis"));
    const spectrum = Array.from({ length: 10 }, (_, i) => 100 + i * 5);
    const props: Props = {
      spectrum,
      wavelengths: null,
      pixelCoord: { x: 1023, y: 2446 },
      cubeDims: NON_SPECTRAL_DIMS,
      filePath: "C:/data/cube.fits",
      publishGate: createFramePublishGate(),
    };
    pass(props);
    const tree = pass(props);
    expect(unitGroup(tree)).toBeNull();
    expect(edgeBox(tree, "range:z0").mapping).toBeNull();
    expect(edgeBox(tree, "range:z1").mapping).toBeNull();
    const html = renderClickedPixel(null);
    expect(html).not.toContain('aria-label="Range field unit"');
    expect(html).toContain('aria-label="Range first channel z0"');
  });

  it("names the channels of the continuum windows under axis boxes", () => {
    spectrumViewStore.patch(DOC_VIEW_KEY, {
      windows: [
        [0, 4],
        [35, 39],
      ],
    });
    const props = docProps();
    const html = renderToStaticMarkup(createElement(SpectroscopyPanel, props as never));
    expect(html).toContain("data-continuum-channels");
    expect(html).toContain("A ch 0–4 · B ch 35–39");
    pass(props);
    const tree = pass(props);
    expect(mustFind(tree, (p) => "data-continuum-channels" in p, "continuum channels line").children).toBe("A ch 0–4 · B ch 35–39");
    (unitButton(tree, "channel").onClick as () => void)();
    expect(findProps(pass(props), (p) => "data-continuum-channels" in p)).toBeNull();
  });
});

describe("SpectroscopyPanel line features", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
    spectralMock.measure.mockReset().mockImplementation(async (_path, source) => measured(source as LineMeasurement["source"]));
    spectralMock.axis.mockReset();
    logMock.append.mockReset();
    backend.fit.mockReset().mockResolvedValue(fitResult());
    backend.inspect.mockReset();
  });

  afterEach(() => {
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
  });

  const SERIES_PATH = "C:/data/jw03375_x1dints.fits";
  const SERIES_PICK = { kind: "integration", index: 2 } as const;
  const docProps = (doc: SpectrumDocument = documentOf(), filePath: string = DOC_PATH): Props => ({
    documentMode: true,
    spectrumDocument: doc,
    filePath,
    publishGate: createFramePublishGate(),
  });
  const seriesDocument = (): SpectrumDocument => ({
    ...documentOf({ source: { kind: "x1dints", paths: [SERIES_PATH], detector: "NRS1", sporder: -1, pick: SERIES_PICK } }),
    source: { kind: "file", hdu: null },
  });
  const lineList = (tree: ReactNode) => mustFind(tree, (p) => typeof p.onRedshiftChange === "function", "line list controls");
  const H_ALPHA_MARK = { id: "h_alpha", label: "Hα", family: "optical", restVacuumUm: 0.656461, value: 0.656461 };
  const pickHalpha = (tree: ReactNode) => (spectrumPlotOf(tree).onPickMark as (mark: unknown) => void)(H_ALPHA_MARK);
  const measureNow = async (props: Props) => {
    commitRangeStart(pass(props), 5);
    await (lineSection(pass(props)).onMeasure as () => Promise<void>)();
  };
  const lastOptions = () => spectralMock.measure.mock.calls.at(-1)?.[2];
  const autoButton = (tree: ReactNode) => mustFind(tree, (p) => p.children === "auto", "auto windows button");
  const NONE_TITLE = "no windows: the measurement then takes two windows as wide as the line range, touching it";

  it("measures a pick of a time series through its series source", async () => {
    const props = docProps(seriesDocument(), SERIES_PATH);
    await brushAndMeasure(props);
    expect(spectralMock.measure).toHaveBeenCalledTimes(1);
    expect(spectralMock.measure.mock.calls[0].slice(0, 2)).toEqual([
      SERIES_PATH,
      { kind: "series", paths: [SERIES_PATH], detector: "NRS1", sporder: -1, pick: { kind: "integration", index: 2 } },
    ]);
    expect(lineSection(pass(props)).result).not.toBeNull();
    expect(logMock.append).toHaveBeenCalledTimes(1);
    const html = renderToStaticMarkup(createElement(SpectroscopyPanel, docProps(seriesDocument(), SERIES_PATH) as never));
    expect(html).toContain("time series NRS1, integration 3");
  });

  it("keeps the file source for a table and for a 1D array", async () => {
    const table: SpectrumDocument = {
      ...documentOf({ source: { kind: "x1d", path: DOC_PATH, hdu: 1, extver: null } }),
      source: { kind: "file", hdu: 1 },
    };
    await brushAndMeasure(docProps(table));
    expect(spectralMock.measure.mock.calls[0][1]).toEqual({ kind: "file", hdu: 1 });
    spectrumViewStore.reset();
    hooks.run.cells = [];
    await brushAndMeasure(docProps());
    expect(spectralMock.measure.mock.calls[1][1]).toEqual({ kind: "file", hdu: 0 });
  });

  it("names the picked line in the request", async () => {
    const props = docProps();
    pass(props);
    await measureNow(props);
    expect(lastOptions()).toMatchObject({ lineId: null, lineLabel: null });
    pickHalpha(pass(props));
    await measureNow(props);
    expect(lastOptions()).toMatchObject({ lineId: "h_alpha", lineLabel: "Hα", restUm: 0.656461 });
    (axisSection(pass(props)).onRestChange as (restUm: number) => void)(0.65);
    await measureNow(props);
    expect(lastOptions()).toMatchObject({ lineId: null, lineLabel: null, restUm: 0.65 });
    hooks.run.cells = [];
    const cube = cubeProps();
    pass(cube);
    pickHalpha(pass(cube));
    await measureNow(cube);
    expect(spectralMock.measure.mock.calls.at(-1)?.[1]).toEqual({ kind: "pixel", x: 3, y: 4 });
    expect(lastOptions()).toMatchObject({ lineId: "h_alpha", lineLabel: "Hα", restUm: 0.656461 });
  });

  it("drops a shown measurement when the picked line changes", async () => {
    const props = docProps();
    pass(props);
    pickHalpha(pass(props));
    await measureNow(props);
    expect(lineSection(pass(props)).result).not.toBeNull();
    spectrumViewStore.patch(DOC_VIEW_KEY, { pickedLineId: null });
    expect(lineSection(settledPanel(props)).result).toBeNull();
  });

  it("tells the line section the unit of the card and the file", () => {
    const props = docProps();
    const tree = settledPanel(props);
    expect(lineSection(tree).axisDisplayUnit).toBe("um");
    expect(lineSection(tree).filePath).toBe(DOC_PATH);
    spectrumViewStore.patch(DOC_VIEW_KEY, { wavelengthUnit: "A" });
    expect(lineSection(settledPanel(props)).axisDisplayUnit).toBe("A");
  });

  it("says when the measured series is not the one on screen", () => {
    const props = docProps();
    expect(lineSection(settledPanel(props)).measuredSeriesNote).toBeNull();
    spectrumViewStore.patch(DOC_VIEW_KEY, { binFactor: 4 });
    expect(lineSection(settledPanel(props)).measuredSeriesNote).toBe("measured on the unbinned channels");
    spectrumViewStore.patch(DOC_VIEW_KEY, { binFactor: 1, fluxUnit: "uJy" });
    expect(lineSection(settledPanel(props)).measuredSeriesNote).toBe("measured in the unit of the file (erg/s/cm2/A)");
    spectrumViewStore.patch(DOC_VIEW_KEY, { binFactor: 4, fluxUnit: "uJy" });
    expect(lineSection(settledPanel(props)).measuredSeriesNote).toBe(
      "measured on the unbinned channels; measured in the unit of the file (erg/s/cm2/A)",
    );

    hooks.run.cells = [];
    showRegion(regionWithErrors());
    const cube = cubeProps();
    expect(lineSection(settledPanel(cube)).measuredSeriesNote).toBeNull();
    (mustFind(pass(cube), (p) => p.children === "mean", "mean series button").onClick as () => void)();
    expect(lineSection(settledPanel(cube)).measuredSeriesNote).toBe(
      `measured on the sum series (${fluxUnitLabel("DN", "sum", "region")}), not on the mean series shown`,
    );

    hooks.run.cells = [];
    spectrumState.region = null;
    spectrumState.regionSource = null;
    expect(lineSection(settledPanel(cubeProps())).measuredSeriesNote).toBeNull();
  });

  it("sets guarded windows with the auto button and says its rule", () => {
    spectrumViewStore.patch(DOC_VIEW_KEY, { range: { z0: 10, z1: 19 } });
    const props = docProps();
    const tree = settledPanel(props);
    expect(autoButton(tree).title).toBe(AUTO_WINDOWS_RULE);
    (autoButton(tree).onClick as () => void)();
    const expected = [
      [0, 9],
      [23, 32],
    ];
    expect(spectrumViewStore.get(DOC_VIEW_KEY).windows).toEqual(expected);
    expect(guardedContinuumWindows({ z0: 10, z1: 19 }, N_DOC)).toEqual(expected);
    expect(mustFind(settledPanel(props), (p) => p.children === "none", "no windows button").title).toBe(NONE_TITLE);

    spectrumViewStore.reset();
    hooks.run.cells = [];
    spectrumViewStore.patch(DOC_VIEW_KEY, { range: { z0: 10, z1: 19 } });
    (mustFind(settledPanel(props), (p) => p.edge === "window:0:1", "window A last edge").onCommit as (value: number) => void)(5);
    expect(spectrumViewStore.get(DOC_VIEW_KEY).windows).toEqual([
      [0, 5],
      [23, 32],
    ]);

    spectrumViewStore.reset();
    hooks.run.cells = [];
    spectrumViewStore.patch(DOC_VIEW_KEY, { range: { z0: 0, z1: 9 } });
    (autoButton(settledPanel(props)).onClick as () => void)();
    expect(spectrumViewStore.get(DOC_VIEW_KEY).windows).toEqual([
      [13, 22],
      [13, 22],
    ]);

    spectrumViewStore.reset();
    hooks.run.cells = [];
    spectrumViewStore.patch(DOC_VIEW_KEY, { range: { z0: 0, z1: N_DOC - 1 } });
    (autoButton(settledPanel(props)).onClick as () => void)();
    expect(spectrumViewStore.get(DOC_VIEW_KEY).windows).toBeNull();
  });

  it("words the overlap note for a spectrum file", () => {
    const stored = {
      range: { z0: 10, z1: 19 },
      windows: [
        [5, 12],
        [25, 30],
      ] as [[number, number], [number, number]],
    };
    spectrumViewStore.patch(DOC_VIEW_KEY, stored);
    const doc = renderToStaticMarkup(createElement(SpectroscopyPanel, docProps() as never));
    expect(doc).toContain("Continuum window A overlaps the line range; Measure line would fit its continuum over line flux.");
    spectrumViewStore.patch(CUBE_VIEW_KEY, stored);
    const cube = renderToStaticMarkup(createElement(SpectroscopyPanel, cubeProps() as never));
    expect(cube).toContain(
      "Continuum window A overlaps the line range; Measure line, Moments and Fit lines would fit their continuum over line flux.",
    );
  });

  const fitConfig = () => backend.fit.mock.calls.at(-1)?.[2];

  async function fitAt(z: number | null): Promise<Props> {
    const props = { ...cubeProps(), publishGate: createFramePublishGate(), onCubeResult: vi.fn() };
    settledPanel(props);
    commitRangeStart(pass(props), 5);
    (axisSection(pass(props)).onRestChange as (restUm: number) => void)(0.6563);
    if (z !== null) (lineList(pass(props)).onRedshiftChange as (value: number) => void)(z);
    await (fitSection(settledPanel(props)).onFit as () => Promise<void>)();
    return props;
  }

  it("sends the redshift of the line list to the cube line fit", async () => {
    const props = await fitAt(0.5);
    expect(backend.fit).toHaveBeenCalledTimes(1);
    expect(fitConfig()).toMatchObject({ redshift: 0.5, rest_um: 0.6563 });
    expect(fitSection(settledPanel(props)).redshift).toBe(0.5);
    spectrumViewStore.reset();
    hooks.run.cells = [];
    await fitAt(null);
    expect(fitConfig()).toMatchObject({ redshift: 0, rest_um: 0.6563 });
  });

  it("marks a fit stale when the redshift changes after the run", async () => {
    const props = await fitAt(0.5);
    expect(fitSection(settledPanel(props)).stale).toBe(false);
    (lineList(pass(props)).onRedshiftChange as (value: number) => void)(0.2);
    expect(fitSection(settledPanel(props)).stale).toBe(true);
    (lineList(pass(props)).onRedshiftChange as (value: number) => void)(0.5);
    expect(fitSection(settledPanel(props)).stale).toBe(false);
  });

  it("shows the source-frame velocity first when the fit made one", async () => {
    const plain = fitResult();
    backend.fit.mockResolvedValue({
      ...plain,
      plane_order: [...plain.plane_order, "source_velocity"],
      planes: {
        ...plain.planes,
        source_velocity: { png_path: "C:/out/fit_src.png", fits_path: "C:/out/fit_src.fits", previewUrl: "asset://fit_src.png" },
      },
    });
    const props = await fitAt(0.5);
    const run = fitSection(pass(props)).run as LineFitRun;
    const onCubeResult = props.onCubeResult as ReturnType<typeof vi.fn>;
    expect(onCubeResult).toHaveBeenCalledTimes(1);
    expect(onCubeResult.mock.calls[0][0]).toMatchObject({ label: lineFitCubeResult(run, "source_velocity")?.label, fitsPath: "C:/out/fit_src.fits" });

    spectrumViewStore.reset();
    hooks.run.cells = [];
    backend.fit.mockResolvedValue(fitResult());
    const plainProps = await fitAt(0.5);
    const plainRun = fitSection(pass(plainProps)).run as LineFitRun;
    expect((plainProps.onCubeResult as ReturnType<typeof vi.fn>).mock.calls[0][0]).toMatchObject({
      label: lineFitCubeResult(plainRun, "velocity")?.label,
      fitsPath: "C:/out/fit.fits",
    });
  });
});

describe("SpectroscopyPanel band strips", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    regionDocState.regions = [];
  });

  const G_AXIS = Array.from({ length: 200 }, (_, i) => 3.6 + i * 0.005);
  const bandDocument = (): SpectrumDocument =>
    documentOf({
      axis: { ...airAxis("wave"), header_unit: "um", header_scale: 1, values: G_AXIS, crval: G_AXIS[0], cdelt: 0.005 },
      wavelength_frame: "vacuum",
      flux: G_AXIS.map(() => 1),
      err: null,
      flux_unit: "Jy",
      n_rows: G_AXIS.length,
    });
  const bandProps = (): Props => ({
    documentMode: true,
    spectrumDocument: bandDocument(),
    filePath: DOC_PATH,
    publishGate: createFramePublishGate(),
  });
  const lineList = (tree: ReactNode) => mustFind(tree, (p) => typeof p.onRedshiftChange === "function", "line list controls");
  const rangeBandNote = (tree: ReactNode) => findProps(tree, (p) => "data-range-band" in p);

  it("hands the band species to the line list and stores a change", () => {
    const props = bandProps();
    pass(props);
    const first = lineList(pass(props));
    expect(first.bandSpecies).toEqual([]);
    expect(first.bandDrawnCount).toBe(0);
    (first.onBandSpeciesChange as (species: string[]) => void)(["CO2"]);
    expect(spectrumViewStore.get(DOC_VIEW_KEY).bandSpecies).toEqual(["CO2"]);
    const tree = pass(props);
    expect((spectrumPlotOf(tree).bands as BandMark[]).map((b) => b.id)).toEqual(["co2_4p3"]);
    expect(lineList(tree).bandDrawnCount).toBe(1);
  });

  it("sets the line range to a clicked band and says which band", () => {
    spectrumViewStore.patch(DOC_VIEW_KEY, { bandSpecies: ["CO2"] });
    const props = bandProps();
    pass(props);
    const brush = mustFind(pass(props), (p, type) => type === "select" && p.value === "range", "brush select");
    (brush.onChange as (e: unknown) => void)({ target: { value: "left" } });
    const plot = spectrumPlotOf(pass(props));
    expect(plot.activeBrush).toBe("left");
    const bands = plot.bands as BandMark[];
    (plot.onPickBand as (band: BandMark) => void)(bands[0]);
    expect(spectrumViewStore.get(DOC_VIEW_KEY).range).toEqual({ z0: 110, z1: 180 });
    const tree = pass(props);
    expect(spectrumPlotOf(tree).range).toEqual({ z0: 110, z1: 180 });
    const note = rangeBandNote(tree);
    expect(note, "range band note").not.toBeNull();
    expect(note!.children).toBe(pickedBandLabel(bandById("co2_4p3")!));
    expect(note!.children).toBe("range ← CO2 4.3 4.15 to 4.5 μm vacuum (display range)");
    commitRangeStart(tree, 5);
    expect(rangeBandNote(pass(props))).toBeNull();
  });
});

describe("SpectroscopyPanel export notice, export name, Compare DQ and frame label", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    logMock.append.mockReset();
    saveDialog.save.mockReset();
    backend.cubeSpectrum.mockReset();
    backend.pixelToWorld.mockReset();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    renderState.processed = null;
  });

  const LONG_PATH = `C:/Users/jdoe/${"spectra/".repeat(7)}ha_bess_spectrum.csv`;
  const docProps = (filePath = DOC_PATH): Props => ({ documentMode: true, spectrumDocument: documentOf(), filePath, publishGate: createFramePublishGate() });
  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  const exportNoticeOf = (tree: ReactNode) => mustFind(tree, (_p, type) => type === ExportNotice, "export notice");
  const noticeShown = (props: Props) => {
    const notice = exportNoticeOf(pass(props));
    return { saved: notice.saved, copied: notice.copied, error: notice.error };
  };
  const click = async (props: Props, label: string) => {
    (mustFind(pass(props), (p) => p.children === label, `${label} button`).onClick as () => void)();
    await settle();
    await settle();
  };
  const savedName = () => saveDialog.save.mock.calls[0][1] as string;
  const compareOf = (tree: ReactNode) => mustFind(tree, (_p, type) => type === SpectrumComparisonSection, "comparison section");

  it("shows the whole saved path and tells a failed export from a saved one", async () => {
    expect(LONG_PATH).toHaveLength(90);
    const props = docProps();
    saveDialog.save.mockResolvedValue(LONG_PATH);
    await click(props, "Save CSV");
    expect(noticeShown(props)).toEqual({ saved: LONG_PATH, copied: false, error: null });

    vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn(async () => Promise.reject(new Error("denied"))) } });
    await click(props, "Copy CSV");
    expect(noticeShown(props)).toEqual({ saved: null, copied: false, error: "export failed: denied" });

    vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn(async () => {}) } });
    await click(props, "Copy CSV");
    expect(noticeShown(props)).toEqual({ saved: null, copied: true, error: null });

    logMock.append.mockReset();
    saveDialog.save.mockResolvedValue(null);
    await click(props, "Save CSV");
    expect(noticeShown(props)).toEqual({ saved: null, copied: false, error: null });
    expect(logMock.append).not.toHaveBeenCalled();
  });

  it("clears the export notice when the spectrum changes", async () => {
    const props = docProps();
    saveDialog.save.mockResolvedValue(LONG_PATH);
    await click(props, "Save CSV");
    expect(noticeShown(props).saved).toBe(LONG_PATH);
    pass(docProps("C:/data/other_bess.fits"));
    expect(noticeShown(docProps("C:/data/other_bess.fits"))).toEqual({ saved: null, copied: false, error: null });
  });

  it("names a binned export by its factor", async () => {
    saveDialog.save.mockResolvedValue(null);
    spectrumViewStore.patch(DOC_VIEW_KEY, { binFactor: 4 });
    await click(docProps(), "Save CSV");
    expect(savedName()).toMatch(/_spectrum_bin4\.csv$/);

    saveDialog.save.mockReset();
    saveDialog.save.mockResolvedValue(null);
    spectrumViewStore.patch(DOC_VIEW_KEY, { binFactor: 1 });
    await click(docProps(), "Save CSV");
    expect(savedName()).toMatch(/[^n]_spectrum\.csv$/);

    saveDialog.save.mockReset();
    saveDialog.save.mockResolvedValue(null);
    hooks.run.cells = [];
    backend.cubeSpectrum.mockResolvedValue({ flux_jy: null });
    backend.pixelToWorld.mockResolvedValue({ points: [] });
    await click(cubeProps(), "Save CSV");
    expect(savedName()).toMatch(/_spectrum_pixel_3_4\.csv$/);
  });

  it("passes the DQ switch of a spectrum file to Compare and leaves a cube alone", () => {
    const props = docProps();
    expect(compareOf(pass(props)).dqMasked).toBe(true);
    spectrumViewStore.patch(DOC_VIEW_KEY, { useDq: false });
    expect(compareOf(pass(props)).dqMasked).toBe(false);

    hooks.run.cells = [];
    const cube = cubeProps();
    expect(compareOf(pass(cube)).dqMasked).toBe(true);
    const select = mustFind(pass(cube), (p) => p["aria-label"] === "Science DQ policy", "shared DQ policy");
    (select.onChange as (e: unknown) => void)({ target: { value: "ignore" } });
    expect(compareOf(pass(cube)).dqMasked).toBe(true);

    hooks.run.cells = [];
    const emptyCube = { ...cubeProps(), spectrum: [], pixelCoord: null };
    expect(compareOf(settledPanel(emptyCube)).dqMasked).toBe(true);
  });

  it("prints the frame label to the channel spacing", () => {
    const values = Array.from({ length: N_DOC }, (_, i) => 0.658 + i * 2e-6);
    const dims: CubeDims = {
      ...SPECTRAL_CUBE_DIMS,
      spectral_axis: { ...airAxis("wave"), header_unit: "um", header_scale: 1, values, crval: values[0], cdelt: 2e-6 },
    };
    renderState.processed = {
      kind: "cube",
      label: "ch 1",
      fitsPath: "C:/out/cube_frame_1.fits",
      previewUrl: "asset://cube_frame_1.png",
      dimensions: [8, 8],
      inputPath: "C:/data/cube.fits",
      frameIndex: 1,
    };
    const nav = mustFind(settledPanel(cubeProps(dims)), (p) => typeof p.onFrameRequest === "function", "frame navigation");
    expect(nav.frame).toBe(1);
    expect(nav.frameLabel).toBe("ch 1 (0–39) - 0.658002 μm");
  });
});

describe("SpectroscopyPanel sweep rules", () => {
  beforeEach(() => {
    hooks.run.cells = [];
    hooks.run.effects = [];
    plotRun.cells = [];
    cubeState.ramp = null;
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionContinuum = null;
    spectrumState.regionError = null;
    backend.moments.mockReset();
  });

  afterEach(() => {
    spectrumState.region = null;
    spectrumState.regionSource = null;
    spectrumState.regionError = null;
  });

  it("holds no hand-made failure box", () => {
    expect(adHocErrorBoxLines(spectroscopyPanelSource)).toEqual([]);
  });

  it("announces a spectrum that cannot be read through the compact alert", () => {
    const html = renderDocument(null, { error: "CTYPE1 = 'WAVE-LOG': log-lambda dispersion is not supported" }).replaceAll("&#x27;", "'");
    expect(html).toContain(
      `<div class="ab-error-alert ab-error-alert-compact text-center" role="alert">This spectrum could not be read: CTYPE1 = 'WAVE-LOG': log-lambda dispersion is not supported</div>`,
    );
  });

  it("announces a failed region spectrum", () => {
    spectrumState.regionError = "region lies outside the cube";
    const html = renderToStaticMarkup(createElement(SpectroscopyPanel, cubeProps() as never));
    expect(html).toContain('role="alert">Region spectrum failed: region lies outside the cube</div>');
  });

  it("marks the active series as pressed inside a named group", () => {
    const html = renderRegion(regionWithErrors());
    const start = html.indexOf('role="group" aria-label="Region series"');
    expect(start).toBeGreaterThan(-1);
    const group = html.slice(start, html.indexOf("</span>", start));
    const buttons = [...group.matchAll(/<button([^>]*)>([^<]*)<\/button>/g)].map(([, attributes, text]) => ({ attributes, text }));
    expect(buttons.map((button) => button.text)).toEqual(["sum", "mean"]);
    const pressed = buttons.filter((button) => button.attributes.includes('aria-pressed="true"'));
    expect(pressed.map((button) => button.text)).toEqual(["sum"]);
    expect(pressed[0].attributes).toMatch(/class="[^"]*\bfont-semibold\b/);
    expect(buttons.filter((button) => button.attributes.includes('aria-pressed="false"')).map((button) => button.text)).toEqual(["mean"]);
  });

  it("marks the shown moment map as pressed inside a named group", async () => {
    backend.moments.mockResolvedValue(momentResult());
    const props = cubeProps({ ...SPECTRAL_CUBE_DIMS, spectral_axis: { ...airAxis("wave"), rest_wavelength_um: 0.6563 } });
    pass(props);
    commitRangeStart(pass(props), 5);
    await (mustFind(pass(props), (p) => p.label === "Moments", "Moments button").onClick as () => Promise<void>)();
    const momentButtons = (tree: ReactNode) =>
      (mustFind(tree, (p) => p.role === "group" && p["aria-label"] === "Moment map", "moment map group").children as { props: Props }[]).map(
        (button) => button.props,
      );
    const pressedState = (tree: ReactNode) =>
      momentButtons(tree).map((button) => [button.children, button["aria-pressed"], String(button.className).includes("font-semibold")]);
    expect(pressedState(pass(props))).toEqual([
      ["m0", true, true],
      ["m1", false, false],
      ["m2", false, false],
    ]);
    (momentButtons(pass(props)).find((button) => button.children === "m2")!.onClick as () => void)();
    expect(pressedState(pass(props))).toEqual([
      ["m0", false, false],
      ["m1", false, false],
      ["m2", true, true],
    ]);
  });

  it("draws the export buttons with the shared small button class", () => {
    const tree = pass({ documentMode: true, spectrumDocument: documentOf(), filePath: DOC_PATH, publishGate: createFramePublishGate() });
    for (const label of ["Copy CSV", "Save CSV"]) {
      const button = mustFind(tree, (p) => p.children === label, `${label} button`);
      expect(button.className).toBe(SMALL_BUTTON_CLASS);
      expect(button.style).toBeUndefined();
    }
  });
});
