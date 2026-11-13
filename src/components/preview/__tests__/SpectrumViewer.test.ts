import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { LineMeasurement, SpectralAxisInfo } from "../../../shared/types/spectral";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";
import type { ActiveSpectrumDocument } from "../../../hooks/useActiveSpectrumDocument";

const activeState = vi.hoisted(() => ({ current: null as unknown }));

vi.mock("react", async (importOriginal) => (await import("../../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../../hooks/useActiveSpectrumDocument", () => ({ useActiveSpectrumDocument: () => activeState.current }));

import { hooks } from "../../../test/hookRuntime";
import SpectrumViewer from "../SpectrumViewer";
import SpectrumPlot from "../../analysis/SpectrumPlot";
import { documentViewKey, spectrumViewStore, type SpectrumLineOverlay } from "../../../hooks/useSpectrumViewStore";
import { documentPlot, type SpectrumPlotModel } from "../../../utils/spectrumPlotModel";
import { SPECTRUM_VIEWER_OPEN_CUBE } from "../../../utils/previewShell";
import type { BandMark, LineMark } from "../../../utils/lineList";
import type { ChannelRange } from "../../../utils/spectrumRange";
import type { SpectrumViewState } from "../../../hooks/useSpectrumViewStore";

const NAME = "ha_bess.fits";
const DOC_PATH = "C:/data/ha_bess.fits";
const CSV_PATH = "C:/data/ha_spectrum.csv";
const N_DOC = 40;
const AIR_VALUES = Array.from({ length: N_DOC }, (_, i) => 0.6555 + i * 1e-4);
const VIEW_KEY = documentViewKey(DOC_PATH, "0");
const HINT =
  "Drag to select a line range, wheel to zoom, Shift and drag to pan, double-click to reset the zoom. The Spectra tool measures the selected line.";
const H_ALPHA: LineMark = { id: "h_alpha", label: "Hα", family: "optical", restVacuumUm: 0.656461, value: 0.656461 };

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

function spectrumOf(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return {
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
  };
}

function activeOf(spectrum: Spectrum1d | null, over: Partial<ActiveSpectrumDocument> = {}): ActiveSpectrumDocument {
  return {
    content: "spectrum",
    fileKey: `doc|${DOC_PATH}`,
    path: DOC_PATH,
    hdu: 0,
    viewKey: VIEW_KEY,
    doc: { loading: false, spectrum, error: null },
    document: spectrum ? { source: { kind: "file", hdu: 0 }, spectrum } : null,
    kind: { kind: "wcs1d", hdu: 0 },
    selection: { kind: "table", hdu: 0, wavelengthUnit: null },
    request: { kind: "file", path: DOC_PATH, hdu: 0, wavelengthUnit: null },
    options: { tables: null, series: null, loading: false, error: null },
    ...over,
  };
}

type Props = Record<string, unknown>;
const viewerFunction = (SpectrumViewer as unknown as { type: (props: Props) => ReactNode }).type;

function findProps(node: ReactNode, match: (type: unknown) => boolean): Props | null {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findProps(child, match);
      if (found) return found;
    }
    return null;
  }
  if (!isValidElement<Props>(node)) return null;
  if (match(node.type)) return node.props;
  return findProps(node.props.children as ReactNode, match);
}

function staticViewer(name = NAME, onOpenTool: () => void = () => {}): string {
  return renderToStaticMarkup(createElement(SpectrumViewer, { name, onOpenTool }));
}

function viewerTree(onOpenTool: () => void = () => {}): ReactNode {
  return hooks.render(viewerFunction, { name: NAME, onOpenTool }) as ReactNode;
}

interface PlotElementProps {
  model: SpectrumPlotModel;
  range: ChannelRange | null;
  lineOverlay: SpectrumLineOverlay | null;
  doubleClick: string;
  cursorKeys: boolean;
  onBrush: (range: ChannelRange) => void;
  onPickMark: (mark: LineMark) => void;
  bands: readonly BandMark[];
  onPickBand: (band: BandMark) => void;
  onViewChange: (partial: Partial<SpectrumViewState>) => void;
}

function plotElement(): PlotElementProps {
  const props = findProps(viewerTree(), (type) => type === SpectrumPlot);
  expect(props, "spectrum plot element").not.toBeNull();
  return props as unknown as PlotElementProps;
}

beforeEach(() => {
  hooks.mount();
  spectrumViewStore.reset();
  activeState.current = activeOf(spectrumOf());
});

describe("SpectrumViewer", () => {
  it("plots the selected spectrum instead of the empty note", () => {
    const html = staticViewer();
    expect(html).toContain("data-spectrum-viewer");
    expect(html).toContain("<canvas");
    expect(html).toContain('data-error-band="propagated"');
    expect(html).toContain(NAME);
    expect(html).toContain("40 channels, 1D array in HDU 0, flux in erg/s/cm2/A");
    expect(html).toContain(SPECTRUM_VIEWER_OPEN_CUBE);
    expect(html).toContain("data-document-frame");
    expect(html).toContain(HINT);
    expect(html).not.toContain("data-spectrum-empty");
  });

  it("says it is reading while the document loads", () => {
    activeState.current = activeOf(null, { doc: { loading: true, spectrum: null, error: null } });
    const html = staticViewer();
    expect(html).toContain(`Reading ${NAME}`);
    expect(html).toContain(SPECTRUM_VIEWER_OPEN_CUBE);
    expect(html).not.toContain("<canvas");
  });

  it("shows why a spectrum could not be read and still opens the tool", () => {
    activeState.current = activeOf(null, {
      doc: { loading: false, spectrum: null, error: "CTYPE1 = 'WAVE-LOG': log-lambda dispersion is not supported" },
    });
    const html = staticViewer();
    expect(html).toContain("This spectrum could not be read: CTYPE1 = &#x27;WAVE-LOG&#x27;: log-lambda dispersion is not supported");
    expect(html).not.toContain("<canvas");
    const onOpenTool = vi.fn();
    const button = findProps(viewerTree(onOpenTool), (type) => type === "button") as { onClick: () => void; children: ReactNode } | null;
    expect(button?.children).toBe(SPECTRUM_VIEWER_OPEN_CUBE);
    button?.onClick();
    expect(onOpenTool).toHaveBeenCalledTimes(1);
  });

  it("says when there is nothing to plot", () => {
    activeState.current = activeOf(spectrumOf({ flux: [], err: null, n_rows: 0 }));
    const html = staticViewer();
    expect(html).toContain("This spectrum has no flux values to plot");
    expect(html).not.toContain("<canvas");
  });

  it("draws what the tool draws", () => {
    const spectrum = spectrumOf();
    activeState.current = activeOf(spectrum);
    expect(plotElement().model).toEqual(documentPlot(spectrum, spectrumViewStore.get(VIEW_KEY)).model);
    expect(plotElement().model.xLabel).toBe("Air wavelength (μm)");

    spectrumViewStore.patch(VIEW_KEY, { range: { z0: 5, z1: 12 }, frameChoice: "vacuum" });
    const patched = plotElement();
    expect(patched.range).toEqual({ z0: 5, z1: 12 });
    expect(patched.model.xLabel).toBe("Vacuum wavelength (μm)");
    expect(patched.model.xValues?.[0]).toBe(0.6555);
    expect(patched.model).toEqual(documentPlot(spectrum, spectrumViewStore.get(VIEW_KEY)).model);

    const overlay: SpectrumLineOverlay = { result: { z0: 5, z1: 12 } as unknown as LineMeasurement, measurementAxis: AIR_VALUES };
    spectrumViewStore.setOverlay(VIEW_KEY, overlay);
    expect(plotElement().lineOverlay).toBe(overlay);
  });

  it("writes a drag in the viewer as the line range of the tool", () => {
    plotElement().onBrush({ z0: 5, z1: 12 });
    expect(spectrumViewStore.get(VIEW_KEY).range).toEqual({ z0: 5, z1: 12 });
  });

  it("picks a rest line from the viewer on a wavelength axis only", () => {
    plotElement().onPickMark(H_ALPHA);
    expect(spectrumViewStore.get(VIEW_KEY)).toMatchObject({ restUm: 0.656461, pickedLineId: "h_alpha" });

    const voptPath = "C:/data/velocity.fits";
    const voptKey = documentViewKey(voptPath, "0");
    const vopt = spectrumOf({ axis: { ...airAxis("vopt"), ctype: "VOPT", unit: "km/s" }, wavelength_frame: "not_wavelength" });
    activeState.current = activeOf(vopt, {
      path: voptPath,
      viewKey: voptKey,
      request: { kind: "file", path: voptPath, hdu: 0, wavelengthUnit: null },
    });
    plotElement().onPickMark(H_ALPHA);
    expect(spectrumViewStore.keys()).not.toContain(voptKey);
    expect(spectrumViewStore.get(voptKey)).toMatchObject({ restUm: null, pickedLineId: null });
  });

  it("zooms and resets from the viewer", () => {
    const plot = plotElement();
    expect(plot.doubleClick).toBe("reset");
    expect(plot.cursorKeys).toBe(true);
    plot.onViewChange({ xDomain: [0.656, 0.657] });
    expect(spectrumViewStore.get(VIEW_KEY).xDomain).toEqual([0.656, 0.657]);
  });

  it("keeps the empty note when no spectrum document is active", () => {
    activeState.current = activeOf(null, {
      content: "empty",
      path: null,
      hdu: null,
      viewKey: null,
      kind: null,
      selection: null,
      request: null,
    });
    const html = staticViewer();
    expect(html).toContain("data-spectrum-empty");
    expect(html).not.toContain("Reading");
  });

  it("names an open CSV by its own file name", () => {
    activeState.current = activeOf(spectrumOf({ source: { kind: "csv", path: CSV_PATH } }), {
      content: "csv",
      path: CSV_PATH,
      hdu: null,
      viewKey: documentViewKey(CSV_PATH, ""),
      kind: null,
      selection: null,
      request: { kind: "file", path: CSV_PATH, hdu: null, wavelengthUnit: null },
    });
    const html = staticViewer("m31.fits");
    expect(html).toContain("ha_spectrum.csv");
    expect(html).not.toContain("m31.fits");
  });

  it("draws the bands of the tool and selects one on a click", () => {
    const axis = Array.from({ length: 200 }, (_, i) => 3.6 + i * 0.005);
    const spectrum = spectrumOf({
      axis: { ...airAxis("wave"), header_unit: "um", header_scale: 1, values: axis, crval: axis[0], cdelt: 0.005 },
      wavelength_frame: "vacuum",
      flux: axis.map(() => 1),
      err: null,
      flux_unit: "Jy",
      n_rows: axis.length,
    });
    activeState.current = activeOf(spectrum);
    spectrumViewStore.patch(VIEW_KEY, { bandSpecies: ["CO2"] });
    const plot = plotElement();
    expect(plot.bands).toEqual(documentPlot(spectrum, spectrumViewStore.get(VIEW_KEY)).bands);
    expect(plot.bands.map((b) => b.id)).toEqual(["co2_4p3"]);
    plot.onPickBand(plot.bands[0]);
    expect(spectrumViewStore.get(VIEW_KEY).range).toEqual({ z0: 110, z1: 180 });
  });
});
