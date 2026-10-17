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

function render(regions: Region[] = []): string {
  const doc = spectrum(AXIS, AXIS.map((_, i) => 10 + i), { kind: "x1d", path: DOC_PATH, hdu: 1, extver: 1 });
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
    }),
  );
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
});
