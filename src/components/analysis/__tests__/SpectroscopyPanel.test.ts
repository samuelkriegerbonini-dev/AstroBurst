import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createFramePublishGate } from "../../../utils/cubeNavigation";
import type { CollapseRangeResult, CubeDims, MomentMapsResult, NoiseFactor, RegionSpectrum } from "../../../shared/types/cube";
import type { Region, RegionShape } from "../../../shared/types/regions";
import { defaultContinuumWindows } from "../../../utils/spectrumRange";
import type { RampInfo } from "../../../shared/types/ramp";
import type { LineMeasurement, SpectralAxisInfo } from "../../../shared/types/spectral";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";
import type { SpectrumDocument } from "../../../utils/spectrumDocument";

const cubeState = vi.hoisted(() => ({ ramp: null as unknown }));

const hooks = vi.hoisted(() => {
  type Cell = { value?: unknown; deps?: readonly unknown[]; set?: (next: unknown) => void; cleanup?: unknown };
  const run = { active: false, cells: [] as Cell[], cursor: 0, effects: [] as (() => void)[] };
  const sameDeps = (a: readonly unknown[] | undefined, b: readonly unknown[] | undefined) =>
    a !== undefined && b !== undefined && a.length === b.length && a.every((v, i) => Object.is(v, b[i]));
  const cell = (): Cell => (run.cells[run.cursor++] ??= {});
  const remember = (compute: () => unknown, deps?: readonly unknown[]) => {
    const c = cell();
    if (!("value" in c) || !sameDeps(c.deps, deps)) {
      c.value = compute();
      c.deps = deps;
    }
    return c.value;
  };
  const impl = {
    useState(init: unknown) {
      const c = cell();
      if (!("value" in c)) c.value = typeof init === "function" ? (init as () => unknown)() : init;
      c.set ??= (next: unknown) => {
        c.value = typeof next === "function" ? (next as (prev: unknown) => unknown)(c.value) : next;
      };
      return [c.value, c.set];
    },
    useRef: (init: unknown) => remember(() => ({ current: init }), []),
    useMemo: remember,
    useCallback: (fn: unknown, deps?: readonly unknown[]) => remember(() => fn, deps),
    useId: () => `id-${run.cursor++}`,
    useEffect(effect: () => unknown, deps?: readonly unknown[]) {
      const c = cell();
      if (sameDeps(c.deps, deps)) return;
      c.deps = deps;
      run.effects.push(() => {
        if (typeof c.cleanup === "function") c.cleanup();
        c.cleanup = effect();
      });
    },
  };
  return { run, impl };
});

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
}));
const spectrumState = vi.hoisted(() => ({ region: null as unknown, regionSource: null as unknown, regionContinuum: null as unknown }));
const regionDocState = vi.hoisted(() => ({ regions: [] as unknown[] }));
const bandCalls = vi.hoisted(() => ({ calls: [] as { y: readonly number[]; err: readonly number[] | null }[] }));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<Record<string, unknown>>();
  const fakes = hooks.impl as unknown as Record<string, (...args: unknown[]) => unknown>;
  const switched = Object.fromEntries(
    Object.entries(fakes).map(([name, fake]) => [
      name,
      (...args: unknown[]) => (hooks.run.active ? fake : (actual[name] as (...a: unknown[]) => unknown))(...args),
    ]),
  );
  return { ...actual, ...switched };
});
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

vi.mock("../../../context/PreviewContext", () => ({
  useCubeContext: () => ({ isCube: true, ramp: cubeState.ramp, rampSource: cubeState.ramp ? "fits" : null }),
  useFileContext: () => ({ file: { path: "C:/data/cube.fits", result: { header: { BUNIT: "DN" } } } }),
  useRenderContext: () => ({ processed: null }),
}));
vi.mock("../../../hooks/useSpectrumStore", () => ({
  useSpectrum: () => ({
    region: spectrumState.region,
    regionSource: spectrumState.regionSource,
    regionContinuum: spectrumState.regionContinuum,
    regionLoading: false,
    regionError: null,
  }),
  beginRegionSpectrum: () => {},
  clearRegionSpectrum: () => {},
  commitRegionSpectrum: (region: unknown, source: unknown, continuum: unknown = null) => {
    spectrumState.region = region;
    spectrumState.regionSource = source;
    spectrumState.regionContinuum = continuum;
  },
  failRegionSpectrum: () => {},
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

import SpectroscopyPanel from "../SpectroscopyPanel";

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
    expect(html).toContain('aria-label="Range first channel z0"');
    expect(html).toContain('aria-label="Continuum B last channel"');
    expect(html.match(/inputMode="decimal"/g)?.length).toBe(6);
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

function findProps(node: ReactNode, match: (props: Props) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findProps(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Props>(node)) return null;
  if (match(node.props)) return node.props;
  return findProps(node.props.children as ReactNode, match);
}

function mustFind(node: ReactNode, match: (props: Props) => boolean, what: string): Props {
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
  const box = mustFind(tree, (p) => p.label === "Range first channel z0", "range z0 box");
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
    const overlay = mustFind(tree, (p) => p.className === "absolute inset-0 w-full pointer-events-none", "overlay canvas");
    const ctx = new Proxy({} as Record<string | symbol, unknown>, {
      get: (target, key) => (key in target ? target[key] : key === "measureText" ? () => ({ width: 10 }) : () => {}),
      set: (target, key, value) => {
        target[key] = value;
        return true;
      },
    });
    (overlay.ref as { current: unknown }).current = { width: 400, height: 180, getContext: () => ctx };
    overlayAxes.calls = [];
    await (lineSection(tree).onMeasure as () => Promise<void>)();
    pass(props);
    expect(overlayAxes.calls.length).toBeGreaterThan(0);
    expect(overlayAxes.calls[overlayAxes.calls.length - 1]).toEqual(AIR_VALUES);
  });

  it("logs the file measurement as a spectrum file with the DQ request", async () => {
    await brushAndMeasure(docProps());
    expect(logMock.append).toHaveBeenCalledTimes(1);
    expect(logMock.append.mock.calls[0][0]).toMatchObject({ kind: "line", image: "spectrum file", dq: "requested", file: "ha_bess.fits" });
  });

  it("sends the frame chosen in the panel and the ERR and DQ switches of the line section", async () => {
    const props = docProps();
    pass(props);
    const vacuum = mustFind(pass(props), (p) => p["data-frame-choice"] === "vacuum", "vacuum frame button");
    (vacuum.onClick as () => void)();
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
    const air = mustFind(pass(props), (p) => p["data-frame-choice"] === "air", "air frame button");
    (air.onClick as () => void)();
    expect(lineSection(pass(props)).result).not.toBeNull();
    const vacuum = mustFind(pass(props), (p) => p["data-frame-choice"] === "vacuum", "vacuum frame button");
    (vacuum.onClick as () => void)();
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
    const vacuum = mustFind(tree, (p) => p["data-frame-choice"] === "vacuum", "vacuum frame button");
    expect(vacuum.disabled).toBe(true);
    (vacuum.onClick as () => void)();
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
    expect(lines[headerIndex]).toBe("channel,wavelength_vacuum_um,wavelength_air_um,vacuum_wavelength_um,flux,flux_unit,err");
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
    (mustFind(tree, (p) => p.label === label, label).onCommit as (value: number) => void)(value);
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
    const sent = defaultContinuumWindows({ z0: 3, z1: N_DOC - 1 }, N_DOC);
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
    (mustFind(tree, (p) => p.className === "w-full rounded-md cursor-crosshair", "plot canvas").ref as { current: unknown }).current = plot.canvas;
    (mustFind(pass(props), (p) => p.children === "mean", "mean series button").onClick as () => void)();
    pass(props);
    (mustFind(pass(props), (p) => p.children === "sum", "sum series button").onClick as () => void)();
    pass(props);
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
    const expected = defaultContinuumWindows({ z0: 5, z1: N_DOC - 1 }, N_DOC);
    expect(expected).not.toBeNull();
    expect(backend.regionSpectrum).toHaveBeenLastCalledWith("C:/data/cube.fits", REGION_CIRCLE, null, expected);
  });

  it("draws the mean view's band from mean_err and the sum view's band from err, each with its scaled band", () => {
    const region = regionWithErrors({ factor: 1.3 });
    showRegion(region);
    const props = cubeProps();
    const tree = pass(props);
    const canvas = mustFind(tree, (p) => p.className === "w-full rounded-md cursor-crosshair", "plot canvas");
    (canvas.ref as { current: unknown }).current = fakeCanvas();
    (mustFind(pass(props), (p) => p.children === "mean", "mean series button").onClick as () => void)();
    pass(props);
    expect(bandCalls.calls.length).toBeGreaterThan(0);
    const meanErrs = bandCalls.calls.map((call) => call.err);
    expect(meanErrs).toContainEqual(region.mean_err);
    expect(meanErrs).toContainEqual(region.mean_err!.map((e) => e * 1.3));
    expect(meanErrs).not.toContainEqual(region.err);
    expect(bandCalls.calls.every((call) => call.y === bandCalls.calls[0].y)).toBe(true);
    expect(bandCalls.calls[0].y).toEqual(region.mean);
    bandCalls.calls = [];
    (mustFind(pass(props), (p) => p.children === "sum", "sum series button").onClick as () => void)();
    pass(props);
    const sumErrs = bandCalls.calls.map((call) => call.err);
    expect(sumErrs).toContainEqual(region.err);
    expect(sumErrs).toContainEqual(region.err_scaled);
    expect(sumErrs).not.toContainEqual(region.mean_err);
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
