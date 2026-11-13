import { beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ProfileSeries } from "../../regions/ProfilePlot";
import type { Region } from "../../../shared/types/regions";
import type { SpectralAxisInfo } from "../../../shared/types/spectral";
import type { Spectrum1d } from "../../../shared/types/spectrum1d";

const plots = vi.hoisted(() => ({ series: [] as ProfileSeries[][] }));

vi.mock("../../regions/ProfilePlot", () => ({
  default: (props: { series: ProfileSeries[] }) => {
    plots.series.push(props.series);
    return "[plot]";
  },
}));

import SpectrumComparisonSection from "../SpectrumComparisonSection";
import { spectrumComparisonStore } from "../../../hooks/useSpectrumComparisonStore";
import { APERTURE_NOTE, documentAsTable } from "../../../utils/x1dCompare";
import spectrumComparisonSource from "../SpectrumComparisonSection.tsx?raw";
import { localSmallButtonLines } from "../../../test/panelRules";

const DOC_PATH = "C:/data/jw_g395h_x1d.fits";
const OTHER_PATH = "C:/data/g235h_spectrum.csv";
const AXIS = Array.from({ length: 10 }, (_, i) => 2.0 + i * 0.001);

function waveAxis(values: number[]): SpectralAxisInfo {
  return {
    kind: "wave",
    ctype: "WAVELENGTH",
    unit: "um",
    header_unit: "um",
    header_scale: 1,
    values,
    crval: values[0],
    cdelt: 0.001,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
    tabulated: true,
  };
}

function spectrum(axis: number[], flux: number[], source: Spectrum1d["source"]): Spectrum1d {
  return {
    source,
    axis: waveAxis(axis),
    wavelength_frame: "vacuum",
    precision: "f64",
    flux,
    err: flux.map(() => 0.1),
    dq: null,
    dq_table: "unknown",
    flux_unit: "Jy",
    n_rows: flux.length,
    cards: {},
    notes: [],
  };
}

const REGION: Region = {
  id: "r1",
  shape: { shape: "circle", x: 3, y: 3, r: 2 },
  props: { color: null, width: null, text: null, dash: null, include: true },
  backgroundId: null,
};

function render(regions: Region[] = [], fileSpectrum: Partial<Spectrum1d> = {}, dqMasked?: boolean): string {
  const doc = { ...spectrum(AXIS, AXIS.map((_, i) => 10 + i), { kind: "x1d", path: DOC_PATH, hdu: 1, extver: 1 }), ...fileSpectrum };
  return renderToStaticMarkup(
    createElement(SpectrumComparisonSection, {
      filePath: DOC_PATH,
      regionDoc: { regions, selectedId: null, version: 3 },
      axis: doc.axis,
      mode: "wavelength_vac",
      restUm: null,
      convention: "optical",
      correction: "none",
      correctionResult: null,
      pixelCoord: null,
      view: "sum",
      bunit: "Jy",
      channelCount: AXIS.length,
      windows: null,
      fileSpectrum: doc,
      fileSpectrumSource: { kind: "file", hdu: 1 },
      ...(dqMasked === undefined ? {} : { dqMasked }),
    }),
  );
}

function commitComparedSpectrum(): void {
  const other = spectrum([1.9995, 2.0025, 2.0055, 2.0085, 2.0115], [1, 4, 7, 10, 13], { kind: "csv", path: OTHER_PATH });
  spectrumComparisonStore.requestTable(DOC_PATH, OTHER_PATH);
  const request = spectrumComparisonStore.beginTable(DOC_PATH);
  expect(request).not.toBeNull();
  spectrumComparisonStore.commitTable(DOC_PATH, request!, documentAsTable(other));
}

describe("SpectrumComparisonSection on a spectrum file", () => {
  beforeEach(() => {
    plots.series = [];
    spectrumComparisonStore.forget(DOC_PATH);
  });

  it("offers a spectrum pick and no cube, region or sibling x1d action", () => {
    const html = render([REGION]);
    expect(html).toContain("Compare spectra");
    expect(html).not.toContain('disabled=""');
    expect(html).toContain("pick an x1d, 1D FITS or CSV spectrum to compare");
    spectrumComparisonStore.patch(DOC_PATH, { enabled: true });
    const open = render([REGION]);
    expect(open).toContain("Pick spectrum…");
    expect(open).not.toContain("Compare with pipeline x1d");
    expect(open).not.toContain("Refresh");
    expect(open).not.toContain("refreshing");
    expect(open).not.toContain("R1");
  });

  it("plots the spectrum and a second spectrum resampled onto the spectrum's own axis", () => {
    const other = spectrum([1.9995, 2.0025, 2.0055, 2.0085, 2.0115], [1, 4, 7, 10, 13], { kind: "csv", path: OTHER_PATH });
    spectrumComparisonStore.requestTable(DOC_PATH, OTHER_PATH);
    const request = spectrumComparisonStore.beginTable(DOC_PATH);
    expect(request).not.toBeNull();
    spectrumComparisonStore.commitTable(DOC_PATH, request!, documentAsTable(other));
    const html = render();
    expect(html).toContain("jw_g395h_x1d.fits");
    expect(html).toContain("g235h_spectrum.csv");
    expect(html).not.toContain(APERTURE_NOTE);
    const series = plots.series[plots.series.length - 1];
    expect(series.map((s) => s.label)).toEqual(["jw_g395h_x1d.fits", "g235h_spectrum.csv"]);
    expect(series[0].y).toEqual(AXIS.map((_, i) => 10 + i));
    series[1].y.forEach((y, i) => expect(y).toBeCloseTo(1 + 1000 * (AXIS[i] - 1.9995), 9));
    expect(series[1].x).toBe(series[0].x);
  });

  it("masks the spectrum file's series unless the panel says not to", () => {
    spectrumComparisonStore.patch(DOC_PATH, { enabled: true });
    const flagged: Partial<Spectrum1d> = { dq: AXIS.map((_, i) => (i === 1 ? 1 : 0)), dq_table: "jwst" };
    const masked = render([], flagged);
    const maskedSeries = plots.series[plots.series.length - 1];
    expect(maskedSeries[0].y[1]).toBeNull();
    expect(maskedSeries[0].y[0]).toBe(10);
    expect(masked).toContain("1 DQ rows dropped");
    const unmasked = render([], flagged, false);
    const unmaskedSeries = plots.series[plots.series.length - 1];
    expect(unmaskedSeries[0].y[1]).toBe(11);
    expect(unmasked).not.toContain("DQ rows dropped");
  });

  it("draws the compared spectrum dashed, in the plot and in the list", () => {
    commitComparedSpectrum();
    const html = render();
    const series = plots.series[plots.series.length - 1];
    expect(series[0].dashed).toBe(false);
    expect(series[1].dashed).toBe(true);
    expect(html.match(/data-series-dash/g)).toHaveLength(1);
    expect(html.match(/inline-block w-2\.5 h-2\.5 rounded-sm shrink-0/g)).toHaveLength(1);
    expect(html.indexOf("w-2.5 h-2.5 rounded-sm")).toBeLessThan(html.indexOf("data-series-dash"));
  });
});

function buttonClasses(html: string, attribute: string): string[][] {
  return [...html.matchAll(/<button[^>]*>/g)]
    .map(([tag]) => tag)
    .filter((tag) => tag.includes(attribute))
    .map((tag) => (/class="([^"]*)"/.exec(tag)?.[1] ?? "").split(" "));
}

describe("SpectrumComparisonSection sweep rules", () => {
  beforeEach(() => {
    plots.series = [];
    spectrumComparisonStore.forget(DOC_PATH);
  });

  it("keeps no private copy of the small button class", () => {
    expect(localSmallButtonLines(spectrumComparisonSource)).toEqual([]);
    expect(spectrumComparisonSource).toMatch(/import \{[^}]*\bSMALL_BUTTON_CLASS\b[^}]*\} from "\.\.\/ui";/);
  });

  it("gives the remove and cancel buttons a 24 px target", () => {
    commitComparedSpectrum();
    const remove = buttonClasses(render(), 'aria-label="Remove the x1d from the comparison"');
    expect(remove).toHaveLength(1);
    expect(remove[0]).toEqual(expect.arrayContaining(["min-w-6", "min-h-6"]));
    spectrumComparisonStore.requestTable(DOC_PATH, OTHER_PATH);
    expect(spectrumComparisonStore.beginTable(DOC_PATH)).not.toBeNull();
    const cancel = buttonClasses(render(), 'title="Stop loading the x1d"');
    expect(cancel).toHaveLength(1);
    expect(cancel[0]).toEqual(expect.arrayContaining(["min-w-6", "min-h-6"]));
  });
});
