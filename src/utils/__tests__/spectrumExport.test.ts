import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, it, expect } from "vitest";
import {
  axisColumnHeader,
  comparisonAxis,
  comparisonCandidates,
  fileBaseName,
  fluxUnitLabel,
  linkedAnnulus,
  numericSeries,
  regionForShape,
  regionForSource,
  spectrumCsv,
  spectrumCsvFileName,
  spectrumExportSummary,
  vacuumUmAxis,
  type SpectrumExportDisplay,
  type SpectrumExportInput,
} from "../spectrumExport";
import { fluxDisplay, scaleWavelengths } from "../spectrumUnits";
import { binAxis, binSpectrum, BINNING_BASIS } from "../spectrumBinning";
import { documentDqExclusions } from "../spectrumDocument";
import { shapeCentre } from "../regionGeometry";
import { airToVacuumUm, velocityKms, wavelengthFromVelocityUm, SPEED_OF_LIGHT_KMS } from "../spectralAxis";
import type { RadialVelocityCorrectionResult, SpectralAxisInfo, SpectralAxisKind, SpectrumSource } from "../../shared/types/spectral";
import type { Region, RegionShape } from "../../shared/types/regions";
import type { RegionSpectrum } from "../../shared/types/cube";
import type { Spectrum1d } from "../../shared/types/spectrum1d";

const H_ALPHA_UM = 0.656461;
const LINE_END = "\r\n";

function axisOf(kind: SpectralAxisKind, values: number[], extra: Partial<SpectralAxisInfo> = {}): SpectralAxisInfo {
  return {
    kind,
    ctype: kind.toUpperCase(),
    unit: kind === "freq" ? "GHz" : kind === "wave" || kind === "awav" ? "um" : "km/s",
    header_unit: "",
    header_scale: 1,
    values,
    crval: values[0] ?? 0,
    cdelt: values.length > 1 ? values[1] - values[0] : 0,
    crpix: 1,
    rest_wavelength_um: null,
    rest_frequency_hz: null,
    specsys: null,
    velosys: null,
    notes: [],
    ...extra,
  };
}

const barycentric: RadialVelocityCorrectionResult = {
  barycentric_kms: 12.5,
  heliocentric_kms: 12.4,
  jd_mid: 2460000.5,
  method: "Meeus",
  accuracy_kms: 0.01,
  notes: [],
  ra_deg: 10,
  dec_deg: 20,
  coordinate_source: "header",
};

function regionOf(id: string, shape: RegionShape, extra: Partial<Region["props"]> = {}, backgroundId: string | null = null): Region {
  return { id, shape, props: { color: null, width: null, text: null, dash: null, include: true, ...extra }, backgroundId };
}

const circle: RegionShape = { shape: "circle", x: 20, y: 20, r: 3 };
const annulus: RegionShape = { shape: "annulus", x: 20, y: 20, r_inner: 5, r_outer: 8 };

function regionSpectrum(extra: Partial<RegionSpectrum> = {}): RegionSpectrum {
  return {
    sum: [5, 6, 7],
    mean: [0.5, 0.6, 0.7],
    npix: 28.27,
    n_bg: 40,
    bg_per_pixel: null,
    wavelengths: null,
    unit: "um",
    bg_subtracted: true,
    flux_jy: [1.5e-5, 2e-5, 2.5e-5],
    err: null,
    err_scaled: null,
    mean_err: null,
    used_weight: [28.27, 28.27, 28.27],
    err_source: "none",
    bg_err_per_pixel: null,
    flux_jy_err: null,
    n_dq_excluded: null,
    noise_factor: null,
    notes: [],
    elapsed_ms: 1,
    ...extra,
  };
}

function exportInput(extra: Partial<SpectrumExportInput> = {}): SpectrumExportInput {
  const axis = axisOf("wave", [1.0, 1.001, 1.002], { specsys: "BARYCENT" });
  return {
    fileName: "cube.fits#hdu=1",
    axis,
    mode: "frequency",
    restUm: null,
    convention: "optical",
    correction: "none",
    correctionResult: null,
    source: { kind: "region", shape: circle, background: annulus },
    regionId: "r1",
    regionText: "core",
    backgroundId: "bg1",
    view: "sum",
    bunit: "MJy/sr",
    values: [5, 6, 7],
    region: regionSpectrum(),
    pixelFluxJy: null,
    pixelFluxJyError: null,
    sky: { ra: 10.5, dec: -20.25 },
    exportedAtUtc: "2026-09-25T12:00:00.000Z",
    ...extra,
  };
}

function csvLines(csv: string): string[] {
  return csv.split(LINE_END);
}

function headerRow(csv: string): string {
  return csvLines(csv).find((line) => !line.startsWith("#")) ?? "";
}

function dataRows(csv: string): string[] {
  const lines = csvLines(csv);
  const header = lines.findIndex((line) => !line.startsWith("#"));
  return lines.slice(header + 1).filter((line) => line !== "");
}

describe("comparisonAxis", () => {
  it("comparisonAxis falls back to the channel axis when the formatted length differs from the channel count", () => {
    const axis = axisOf("wave", [1, 2, 3]);
    expect(comparisonAxis(axis, "wavelength_vac", null, "optical", "none", null, 5)).toEqual({
      values: null,
      label: "Channel",
      unit: "ch",
      header: "channel",
    });
    const matching = comparisonAxis(axis, "wavelength_vac", null, "optical", "none", null, 3);
    expect(matching.values).toEqual([1, 2, 3]);
    expect(matching.header).toBe("vacuum_wavelength_um");
  });

  it("comparisonAxis shifts velocity values by the barycentric correction and names the frame in the label", () => {
    const wavelengths = [0.656, 0.6565, 0.657];
    const axis = axisOf("wave", wavelengths);
    const column = comparisonAxis(axis, "velocity", H_ALPHA_UM, "optical", "barycentric", barycentric, 3);
    expect(column.values).not.toBeNull();
    wavelengths.forEach((w, i) => {
      expect(Math.abs(column.values![i] - (velocityKms(w, H_ALPHA_UM, "optical") + 12.5))).toBeLessThanOrEqual(1e-9);
    });
    expect(column.label).toBe("Velocity, optical (km/s), barycentric");
    expect(column.unit).toBe("km/s");
    expect(column.header).toBe("velocity_optical_barycentric_kms");
  });

  it("comparisonAxis leaves wavelength values unshifted outside velocity mode", () => {
    const axis = axisOf("wave", [0.656, 0.6565, 0.657]);
    const column = comparisonAxis(axis, "wavelength_vac", H_ALPHA_UM, "optical", "barycentric", barycentric, 3);
    expect(column.values).toEqual([0.656, 0.6565, 0.657]);
    expect(column.label).toBe("Vacuum wavelength (μm)");
    expect(column.header).toBe("vacuum_wavelength_um");
  });
});

describe("axisColumnHeader", () => {
  it("axisColumnHeader slugs the label examples of section 2 including an unknown unit", () => {
    expect(axisColumnHeader("Vacuum wavelength (μm)", "μm")).toBe("vacuum_wavelength_um");
    expect(axisColumnHeader("Velocity, optical (km/s), barycentric", "km/s")).toBe("velocity_optical_barycentric_kms");
    expect(axisColumnHeader("Velocity (km/s)", "km/s")).toBe("velocity_kms");
    expect(axisColumnHeader("Channel", "ch")).toBe("channel");
    expect(axisColumnHeader("Wavelength (Angstrom)", "Angstrom")).toBe("wavelength_angstrom");
  });
});

describe("vacuumUmAxis", () => {
  it("vacuumUmAxis converts a vopt axis with the header rest using the optical convention", () => {
    const axis = axisOf("vopt", [0, 300, 600], { rest_wavelength_um: H_ALPHA_UM });
    const vacuum = vacuumUmAxis(axis, null, "radio", 3);
    expect(vacuum.reason).toBeNull();
    expect(vacuum.values).not.toBeNull();
    expect(Math.abs(vacuum.values![1] - H_ALPHA_UM * (1 + 300 / SPEED_OF_LIGHT_KMS))).toBeLessThanOrEqual(1e-12);
    expect(vacuum.restUm).toBe(H_ALPHA_UM);
    expect(vacuum.restOrigin).toBe("header");
  });

  it("vacuumUmAxis converts a vrad axis with the radio convention and a velo axis relativistically", () => {
    const vrad = vacuumUmAxis(axisOf("vrad", [100], { rest_wavelength_um: 1.0 }), null, "optical", 1);
    expect(vrad.values![0]).toBeCloseTo(wavelengthFromVelocityUm(100, 1.0, "radio"), 12);
    const velo = vacuumUmAxis(axisOf("velo", [100], { rest_wavelength_um: 1.0 }), null, "optical", 1);
    expect(velo.values![0]).toBeCloseTo(wavelengthFromVelocityUm(100, 1.0, "relativistic"), 12);
    expect(velo.values![0]).not.toBeCloseTo(vrad.values![0], 9);
  });

  it("vacuumUmAxis prefers the header rest over the typed rest on a vopt axis and uses the typed rest only when the header has none", () => {
    const withHeader = vacuumUmAxis(axisOf("vopt", [0, 300], { rest_wavelength_um: H_ALPHA_UM }), 0.5, "optical", 2);
    expect(withHeader.values![0]).toBeCloseTo(H_ALPHA_UM, 12);
    expect(withHeader.restUm).toBe(H_ALPHA_UM);
    expect(withHeader.restOrigin).toBe("header");
    const typed = vacuumUmAxis(axisOf("vopt", [0, 300], { rest_wavelength_um: null }), 0.5, "optical", 2);
    expect(typed.values![0]).toBeCloseTo(0.5, 12);
    expect(typed.values![1]).toBeCloseTo(0.5 * (1 + 300 / SPEED_OF_LIGHT_KMS), 12);
    expect(typed.restUm).toBe(0.5);
    expect(typed.restOrigin).toBe("user");
  });

  it("vacuumUmAxis gives a reason for a velocity axis without any rest, for zopt, for a null axis and for a length mismatch", () => {
    const noRest = vacuumUmAxis(axisOf("vopt", [0, 300]), null, "optical", 2);
    expect(noRest.values).toBeNull();
    expect(noRest.reason).toBe("velocity axis without a rest wavelength (RESTFRQ/RESTWAV absent, none typed)");
    const zopt = vacuumUmAxis(axisOf("zopt", [0.001, 0.002]), 0.5, "optical", 2);
    expect(zopt.values).toBeNull();
    expect(zopt.reason).toBe("axis kind zopt has no vacuum wavelength");
    const missing = vacuumUmAxis(null, 0.5, "optical", 2);
    expect(missing.values).toBeNull();
    expect(missing.reason).toBe("no spectral axis");
    const mismatch = vacuumUmAxis(axisOf("wave", [1, 2, 3]), null, "optical", 4);
    expect(mismatch.values).toBeNull();
    expect(mismatch.reason).toBe("axis has 3 channels, spectrum has 4");
    expect(mismatch.restUm).toBeNull();
  });

  it("vacuumUmAxis converts an awav axis to vacuum", () => {
    const vacuum = vacuumUmAxis(axisOf("awav", [0.5]), null, "optical", 1);
    expect(vacuum.values![0]).toBeCloseTo(airToVacuumUm(0.5), 12);
    expect(vacuum.values![0]).toBeGreaterThan(0.5);
    expect(vacuum.restOrigin).toBeNull();
  });
});

describe("flux unit and series coercion", () => {
  it("fluxUnitLabel follows the line measurement unit rule", () => {
    expect(fluxUnitLabel("Jy/beam", "sum", "region")).toBe("Jy/beam x pix");
    expect(fluxUnitLabel("Jy/beam", "sum", "pixel")).toBe("Jy/beam");
    expect(fluxUnitLabel("Jy/beam", "mean", "region")).toBe("Jy/beam");
    expect(fluxUnitLabel("'MJy/sr'", "jy", "region")).toBe("Jy");
    expect(fluxUnitLabel("'MJy/sr'", "sum", "region")).toBe("MJy/sr x pix");
    expect(fluxUnitLabel(null, "sum", "pixel")).toBe("native");
  });

  it("numericSeries maps null, undefined and strings to NaN and keeps numbers", () => {
    const out = numericSeries([1, null, "2", undefined, NaN, 3]);
    expect(out).toHaveLength(6);
    expect(out[0]).toBe(1);
    expect(out[1]).toBeNaN();
    expect(out[2]).toBeNaN();
    expect(out[3]).toBeNaN();
    expect(out[4]).toBeNaN();
    expect(out[5]).toBe(3);
  });
});

describe("region helpers", () => {
  const regions: Region[] = [
    regionOf("r1", circle, { text: "core" }, "bg1"),
    regionOf("bg1", annulus),
    regionOf("r2", { shape: "box", x: 5, y: 5, width: 4, height: 4, angle: 0 }, {}, "r1"),
    regionOf("r3", { shape: "ellipse", x: 8, y: 8, rx: 2, ry: 1, angle: 30 }, {}, "missing"),
    regionOf("l1", { shape: "line", x1: 0, y1: 0, x2: 1, y2: 1 }),
    regionOf("p1", { shape: "point", x: 3, y: 3 }),
    regionOf("r4", { shape: "polygon", points: [[0, 0], [4, 0], [4, 4]] }, { include: false }),
  ];

  it("linkedAnnulus returns the annulus shape and null for a non-annulus or dangling link", () => {
    expect(linkedAnnulus(regions, regions[0])).toEqual(annulus);
    expect(linkedAnnulus(regions, regions[2])).toBeNull();
    expect(linkedAnnulus(regions, regions[3])).toBeNull();
    expect(linkedAnnulus(regions, regions[1])).toBeNull();
  });

  it("comparisonCandidates keeps included area regions in document order and drops annulus, line, point and excluded regions", () => {
    expect(comparisonCandidates(regions).map((r) => r.id)).toEqual(["r1", "r2", "r3"]);
  });

  it("regionForSource matches the plotted region by shape and returns null for a pixel source, and regionForShape finds the linked annulus", () => {
    const source: SpectrumSource = { kind: "region", shape: { shape: "circle", x: 20, y: 20, r: 3 }, background: annulus };
    expect(regionForSource(regions, source)?.id).toBe("r1");
    expect(regionForSource(regions, { kind: "pixel", x: 1, y: 2 })).toBeNull();
    expect(regionForShape(regions, { shape: "annulus", x: 20, y: 20, r_inner: 5, r_outer: 8 })?.id).toBe("bg1");
    expect(regionForShape(regions, { shape: "circle", x: 20, y: 20, r: 4 })).toBeNull();
  });

  it("shapeCentre averages polygon vertices, takes the line midpoint and the centre otherwise", () => {
    expect(shapeCentre({ shape: "polygon", points: [[0, 0], [4, 0], [4, 4], [0, 4]] })).toEqual({ x: 2, y: 2 });
    expect(shapeCentre({ shape: "line", x1: 0, y1: 0, x2: 4, y2: 2 })).toEqual({ x: 2, y: 1 });
    expect(shapeCentre(circle)).toEqual({ x: 20, y: 20 });
    expect(shapeCentre(annulus)).toEqual({ x: 20, y: 20 });
  });
});

describe("spectrumCsv", () => {
  it("spectrumCsv writes provenance lines then channel, vacuum, displayed axis, flux, unit, Jy and region columns", () => {
    const csv = spectrumCsv(exportInput());
    const lines = csvLines(csv);
    expect(csv).toContain(LINE_END);
    expect(lines[0]).toBe("# file: cube.fits#hdu=1");
    expect(lines).toContain("# stored_frame: BARYCENT");
    expect(lines.some((line) => line.startsWith("# velocity_frame: as stored; shift_kms: 0; method: -"))).toBe(true);
    expect(lines.some((line) => line.startsWith("# flux_jy_basis:"))).toBe(true);
    expect(lines).toContain("# sky_frame: ICRS");
    expect(lines).toContain("# sky_ra_deg: 10.5");
    expect(lines).toContain("# sky_dec_deg: -20.25");
    expect(lines).toContain("# view: sum");
    expect(lines).toContain("# flux_unit: MJy/sr x pix");
    expect(lines).toContain("# exported_utc: 2026-09-25T12:00:00.000Z");
    expect(lines.some((line) => line.startsWith("# source: circle region with background; circle (20.0, 20.0) r=3.0; region: r1; text: core; background: annulus bg1"))).toBe(true);
    expect(headerRow(csv)).toBe("channel,wavelength_vacuum_um,frequency_ghz,flux,flux_unit,flux_jy,npix,bg_subtracted,n_bg");
    const rows = dataRows(csv);
    expect(rows).toHaveLength(3);
    expect(rows[0]).toBe("0,1,299792.458,5,MJy/sr x pix,0.000015,28.27,true,40");
    expect(lines.indexOf(headerRow(csv))).toBe(lines.filter((line) => line.startsWith("#")).length);
  });

  it("spectrumCsv omits the displayed column on an unknown axis and the Jy column without calibration, blanks NaN flux and writes both notes", () => {
    const csv = spectrumCsv(
      exportInput({
        axis: axisOf("unknown", [0, 10, 20], { ctype: "TIME", unit: "s", header_unit: "s" }),
        mode: "wavelength_vac",
        bunit: "Jy/beam",
        values: [1, NaN, 3],
        region: regionSpectrum({ sum: [1, NaN, 3], flux_jy: null, npix: 10, n_bg: 0, bg_subtracted: false }),
        source: { kind: "region", shape: circle, background: null },
        backgroundId: null,
      }),
    );
    const lines = csvLines(csv);
    expect(headerRow(csv)).toBe("channel,flux,flux_unit,npix,bg_subtracted,n_bg");
    expect(lines).toContain("# wavelength_note: axis kind unknown has no vacuum wavelength");
    expect(lines).toContain("# flux_jy_note: no Jy calibration (BUNIT is not MJy/sr)");
    expect(dataRows(csv)[1]).toBe("1,,Jy/beam x pix,10,false,0");
  });

  it("spectrumCsv omits the vacuum and displayed columns when the axis is null", () => {
    const csv = spectrumCsv(
      exportInput({
        axis: null,
        source: { kind: "pixel", x: 3, y: 4 },
        region: null,
        regionId: null,
        regionText: null,
        backgroundId: null,
        bunit: "Jy/beam",
        values: [1, 2, 3],
      }),
    );
    expect(headerRow(csv)).toBe("channel,flux,flux_unit");
    expect(csvLines(csv)).toContain("# wavelength_note: no spectral axis");
    expect(csvLines(csv)).toContain("# stored_frame: not stated");
    expect(dataRows(csv)[2]).toBe("2,3,Jy/beam");
  });

  it("spectrumCsv leaves sky cells empty with a note when there is no WCS", () => {
    const lines = csvLines(spectrumCsv(exportInput({ sky: null })));
    expect(lines).toContain("# sky_frame: ICRS");
    expect(lines).toContain("# sky_ra_deg:");
    expect(lines).toContain("# sky_dec_deg:");
    expect(lines).toContain("# sky_note: no WCS or off-sky position");
  });

  it("spectrumCsv names the linked annulus in the background line and the Jy refetch failure in flux_jy_note", () => {
    const region = csvLines(spectrumCsv(exportInput()));
    const source = region.find((line) => line.startsWith("# source:")) ?? "";
    expect(source).toContain("background: annulus bg1 (annulus (20.0, 20.0) r=5.0–8.0)");

    const pixelCsv = spectrumCsv(
      exportInput({
        source: { kind: "pixel", x: 3, y: 4 },
        region: null,
        regionId: null,
        regionText: null,
        backgroundId: null,
        bunit: "MJy/sr",
        values: [5, 6, 7],
        pixelFluxJy: null,
        pixelFluxJyError: "cube closed",
      }),
    );
    expect(csvLines(pixelCsv)).toContain("# flux_jy_note: Jy refetch failed: cube closed");
    expect(headerRow(pixelCsv)).toBe("channel,wavelength_vacuum_um,frequency_ghz,flux,flux_unit");
    const pixelSource = csvLines(pixelCsv).find((line) => line.startsWith("# source:")) ?? "";
    expect(pixelSource).toBe("# source: pixel (3, 4); pixel (3, 4); region: -; text: -; background: none");

    const pixelJy = spectrumCsv(
      exportInput({
        source: { kind: "pixel", x: 3, y: 4 },
        region: null,
        regionId: null,
        regionText: null,
        backgroundId: null,
        values: [5, 6, 7],
        pixelFluxJy: [1e-6, 2e-6, 3e-6],
      }),
    );
    expect(headerRow(pixelJy)).toBe("channel,wavelength_vacuum_um,frequency_ghz,flux,flux_unit,flux_jy");
    expect(csvLines(pixelJy)).toContain("# flux_jy_basis: pixel value x PIXAR_SR x 1e6");
    expect(dataRows(pixelJy)[0]).toBe("0,1,299792.458,5,MJy/sr,0.000001");
  });

  it("spectrumCsv writes the applied velocity frame only when a correction result exists", () => {
    const axis = axisOf("wave", [0.656, 0.6565, 0.657], { specsys: "TOPOCENT" });
    const pending = spectrumCsv(
      exportInput({ axis, mode: "velocity", restUm: H_ALPHA_UM, correction: "barycentric", correctionResult: null }),
    );
    expect(csvLines(pending).some((line) => line.startsWith("# velocity_frame: as stored; shift_kms: 0; method: -"))).toBe(true);
    expect(headerRow(pending)).toBe("channel,wavelength_vacuum_um,velocity_optical_kms,flux,flux_unit,flux_jy,npix,bg_subtracted,n_bg");
    const applied = spectrumCsv(
      exportInput({ axis, mode: "velocity", restUm: H_ALPHA_UM, correction: "barycentric", correctionResult: barycentric }),
    );
    expect(csvLines(applied).some((line) => line.startsWith("# velocity_frame: barycentric; shift_kms: 12.5; method: Meeus"))).toBe(true);
    expect(headerRow(applied)).toBe(
      "channel,wavelength_vacuum_um,velocity_optical_barycentric_kms,flux,flux_unit,flux_jy,npix,bg_subtracted,n_bg",
    );
    const shifted = velocityKms(0.656, H_ALPHA_UM, "optical") + 12.5;
    expect(dataRows(applied)[0].split(",")[2]).toBe(String(shifted));
    expect(csvLines(applied)).not.toContain("# rest_um: 0.656461 (user)");
  });

  it("spectrumCsv writes no displayed column in velocity mode without a rest on freq and awav axes instead of the header-unit fallback", () => {
    const freq = spectrumCsv(
      exportInput({
        axis: axisOf("freq", [230.538, 230.539, 230.54], { ctype: "FREQ", header_unit: "Hz", header_scale: 1e-9 }),
        mode: "velocity",
        restUm: null,
        bunit: "Jy/beam",
        region: regionSpectrum({ flux_jy: null }),
      }),
    );
    expect(headerRow(freq)).toBe("channel,wavelength_vacuum_um,flux,flux_unit,npix,bg_subtracted,n_bg");
    expect(dataRows(freq)[0].split(",")[1]).toBe(String(SPEED_OF_LIGHT_KMS / 230.538));

    const air = [0.5, 0.5001, 0.5002];
    const awav = spectrumCsv(
      exportInput({
        axis: axisOf("awav", air, { ctype: "AWAV", header_unit: "nm", header_scale: 1e-3 }),
        mode: "velocity",
        restUm: null,
        source: { kind: "pixel", x: 3, y: 4 },
        region: null,
        regionId: null,
        regionText: null,
        backgroundId: null,
        bunit: "Jy/beam",
      }),
    );
    expect(headerRow(awav)).toBe("channel,wavelength_vacuum_um,flux,flux_unit");
    expect(Number(dataRows(awav)[0].split(",")[1])).toBeCloseTo(airToVacuumUm(0.5), 12);
    expect(csvLines(awav).some((line) => line.startsWith("# wavelength_note:"))).toBe(false);
  });

  it("spectrumCsvFileName strips the plane fragment and names the source", () => {
    expect(spectrumCsvFileName("C:/data/cube_s3d.fits#hdu=1", { kind: "pixel", x: 3, y: 4 })).toBe("cube_s3d_spectrum_pixel_3_4.csv");
    expect(spectrumCsvFileName("C:\\data\\cube.fits", { kind: "region", shape: circle, background: null })).toBe("cube_spectrum_region.csv");
    expect(spectrumCsvFileName(null, { kind: "pixel", x: 3, y: 4 })).toBe("spectrum.csv");
    expect(fileBaseName("C:/data/cube_s3d.fits#hdu=1")).toBe("cube_s3d.fits#hdu=1");
  });

  it("spectrumExportSummary reports the parameters and the finite flux maximum", () => {
    const values = [1, NaN, 3, null as unknown as number];
    const summary = spectrumExportSummary(
      exportInput({
        axis: axisOf("vopt", [0, 100, 200, 300], { rest_wavelength_um: H_ALPHA_UM, specsys: "LSRK" }),
        mode: "velocity",
        values,
        region: regionSpectrum({ sum: values, flux_jy: [1, 2, 3, 4] }),
      }),
    );
    expect(summary.params.n_channels).toBe(4);
    expect(summary.params.source).toBe("circle region with background");
    expect(summary.params.view).toBe("sum");
    expect(summary.params.stored_frame).toBe("LSRK");
    expect(summary.params.velocity_frame).toBe("as stored");
    expect(summary.params.shift_kms).toBe(0);
    expect(summary.params.rest_um).toBe(H_ALPHA_UM);
    expect(summary.params.rest_origin).toBe("header");
    expect(summary.params.flux_unit).toBe("MJy/sr x pix");
    expect(summary.params.npix).toBe(28.27);
    expect(summary.params.bg_subtracted).toBe(true);
    expect(summary.values.flux_max).toBe(3);
    expect(summary.values.flux_sum).toBe(4);
    expect(summary.values.flux_jy_max).toBe(4);
    expect(summary.values.ra_deg).toBe(10.5);
    expect(summary.values.dec_deg).toBe(-20.25);
  });
});

const EXPORT_FIXTURE = fileURLToPath(new URL("../../../src-tauri/tests/fixtures/spectrum1d/document_export.csv", import.meta.url));
const DOC_ROWS = 12;
const DOC_PATH = "C:/data/jw_test_x1d.fits";

function docWavelength(i: number): number {
  return 1.8 + i * 0.00137;
}

function docFlux(i: number): number {
  return i === 5 ? NaN : 0.0123 + i * 4.56e-5;
}

function docErr(i: number): number {
  return i === 7 ? NaN : 1e-4 + i * 1e-6;
}

function docDq(i: number): number {
  if (i === 3) return 1;
  if (i === 9) return 2147483648;
  return i === 10 ? 2 : 0;
}

function x1dDocument(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  const rows = Array.from({ length: DOC_ROWS }, (_, i) => i);
  return {
    source: { kind: "x1d", path: DOC_PATH, hdu: 1, extver: 1 },
    axis: axisOf("wave", rows.map(docWavelength), { ctype: "WAVELENGTH", tabulated: true }),
    wavelength_frame: "vacuum",
    precision: "f64",
    flux: rows.map(docFlux),
    err: rows.map(docErr),
    dq: rows.map(docDq),
    dq_table: "jwst",
    flux_unit: "Jy",
    n_rows: DOC_ROWS,
    cards: {},
    notes: [],
    ...extra,
  };
}

function documentExportInput(document: Spectrum1d = x1dDocument(), extra: Partial<SpectrumExportInput> = {}): SpectrumExportInput {
  return {
    fileName: "jw_test_x1d.fits",
    axis: document.axis,
    mode: "wavelength_vac",
    restUm: null,
    convention: "optical",
    correction: "none",
    correctionResult: null,
    source: { kind: "file", hdu: 1 },
    regionId: null,
    regionText: null,
    backgroundId: null,
    view: "sum",
    bunit: document.flux_unit,
    values: document.flux,
    region: null,
    pixelFluxJy: null,
    pixelFluxJyError: null,
    sky: null,
    exportedAtUtc: "2026-10-17T12:00:00.000Z",
    fileSpectrum: document,
    ...extra,
  };
}

describe("spectrumCsv for a spectrum file", () => {
  it("writes the file's flux with its err and dq columns exactly as the backend CSV reader expects", () => {
    const csv = spectrumCsv(documentExportInput());
    expect(csv).toContain("\r\n");
    const golden = readFileSync(EXPORT_FIXTURE, "utf8").replace(/\r\n/g, "\n");
    expect(csv.replace(/\r\n/g, "\n")).toBe(golden);
  });

  it("leaves out the err and dq columns and the DQ table line of a file without them", () => {
    const csv = spectrumCsv(documentExportInput(x1dDocument({ err: null, dq: null })));
    expect(headerRow(csv)).toBe("channel,wavelength_vacuum_um,vacuum_wavelength_um,flux,flux_unit");
    expect(csv).not.toContain("# dq_table:");
    expect(dataRows(csv)[0]).toBe("0,1.8,1.8,0.0123,Jy");
  });

  it("names the export after the file without a pixel or region suffix", () => {
    expect(spectrumCsvFileName(DOC_PATH, { kind: "file", hdu: 1 })).toBe("jw_test_x1d_spectrum.csv");
    expect(spectrumCsvFileName("C:/data/exported_spectrum.csv", { kind: "file", hdu: null })).toBe("exported_spectrum_spectrum.csv");
  });

  it("summarises the file export with its source, unit and error columns", () => {
    const summary = spectrumExportSummary(documentExportInput());
    expect(summary.params.source).toBe("spectrum file, HDU 1");
    expect(summary.params.flux_unit).toBe("Jy");
    expect(summary.params.err_column).toBe(true);
    expect(summary.params.dq_column).toBe(true);
    expect(summary.params.dq_table).toBe("jwst");
    expect(summary.params.npix).toBeNull();
    expect(summary.values.flux_max).toBe(docFlux(11));
    expect(summary.values.flux_jy_max).toBeNull();
    expect(summary.values.ra_deg).toBeNull();
    const bare = spectrumExportSummary(documentExportInput(x1dDocument({ err: null, dq: null })));
    expect(bare.params.err_column).toBe(false);
    expect(bare.params.dq_column).toBe(false);
    expect(bare.params.dq_table).toBeNull();
  });

  it("keeps the cube export summary free of the file-only parameters", () => {
    const summary = spectrumExportSummary(exportInput());
    expect(summary.params).not.toHaveProperty("err_column");
    expect(summary.params).not.toHaveProperty("dq_column");
    expect(summary.params).not.toHaveProperty("dq_table");
  });
});

const AIR_UM = [0.65628, 0.65629, 0.6563];
const AIR_NOTE_LINE =
  "# wavelength_air_note: wavelength_vacuum_um converted from the spectrum's air wavelengths with Morton (2000, ApJS 130, 403); wavelength_air_um holds the spectrum's own air axis";

function airDocument(extra: Partial<Spectrum1d> = {}): Spectrum1d {
  return {
    source: { kind: "wcs1d", path: "C:/data/ha.fits", hdu: 0 },
    axis: axisOf("awav", AIR_UM, { ctype: "LINEAR" }),
    wavelength_frame: "air",
    precision: "f64",
    flux: [1.5, 2.5, 3.5],
    err: null,
    dq: null,
    dq_table: "unknown",
    flux_unit: "erg/s/cm2/A",
    n_rows: AIR_UM.length,
    cards: {},
    notes: [],
    ...extra,
  };
}

function airExportInput(document: Spectrum1d = airDocument(), extra: Partial<SpectrumExportInput> = {}): SpectrumExportInput {
  return documentExportInput(document, { fileName: "ha.fits", source: { kind: "file", hdu: 0 }, ...extra });
}

describe("spectrumCsv for other spectrum file shapes", () => {
  it("writes an air spectrum's own air axis next to its Morton (2000) vacuum wavelengths and says which formula made them", () => {
    const csv = spectrumCsv(airExportInput());
    expect(headerRow(csv)).toBe("channel,wavelength_vacuum_um,wavelength_air_um,vacuum_wavelength_um,flux,flux_unit");
    expect(airToVacuumUm(0.65628)).toBe(0.6564612979997992);
    expect(dataRows(csv)).toEqual(
      AIR_UM.map((air, i) => `${i},${airToVacuumUm(air)},${air},${airToVacuumUm(air)},${[1.5, 2.5, 3.5][i]},erg/s/cm2/A`),
    );
    expect(csvLines(csv)).toContain(AIR_NOTE_LINE);
  });

  it("writes the air, f32 and unitless spectrum the backend reader test reads back as air", () => {
    const fixture = fileURLToPath(new URL("../../../src-tauri/tests/fixtures/spectrum1d/air_document_export.csv", import.meta.url));
    const csv = spectrumCsv(airExportInput(airDocument({ precision: "f32_widened", flux_unit: null }), { mode: "wavelength_air" }));
    expect(csv.replace(/\r\n/g, "\n")).toBe(readFileSync(fixture, "utf8").replace(/\r\n/g, "\n"));
  });

  it("writes no air column for a spectrum measured in vacuum, nor for a cube on an air axis", () => {
    const asVacuum = airDocument();
    const vacuumCsv = spectrumCsv(airExportInput(asVacuum, { axis: { ...asVacuum.axis, kind: "wave" } }));
    expect(headerRow(vacuumCsv)).toBe("channel,wavelength_vacuum_um,vacuum_wavelength_um,flux,flux_unit");
    expect(vacuumCsv).not.toContain("wavelength_air_note");
    const cubeCsv = spectrumCsv(exportInput({ axis: axisOf("awav", [0.65628, 0.65629, 0.6563]), mode: "wavelength_vac" }));
    expect(headerRow(cubeCsv)).not.toContain("wavelength_air_um");
    expect(cubeCsv).not.toContain("wavelength_air_note");
  });

  it("writes the f32 precision of a widened spectrum as a provenance line and nothing for an f64 one", () => {
    expect(csvLines(spectrumCsv(airExportInput(airDocument({ precision: "f32_widened" }))))).toContain("# precision: f32_widened");
    expect(spectrumCsv(airExportInput())).not.toContain("# precision:");
  });

  it("re-exports an opened CSV as a spectrum file without an HDU", () => {
    const opened = x1dDocument({ source: { kind: "csv", path: "C:/data/jw_test_x1d_spectrum.csv" } });
    const lines = csvLines(
      spectrumCsv(documentExportInput(opened, { fileName: "jw_test_x1d_spectrum.csv", source: { kind: "file", hdu: null } })),
    );
    expect(lines).toContain("# image: jw_test_x1d_spectrum.csv (spectrum file, read from disk)");
    expect(lines).toContain("# source: spectrum file; CSV file; region: -; text: -; background: none");
    expect(lines).toContain("# dq_table: jwst");
    expect(spectrumCsvFileName("C:/data/jw_test_x1d_spectrum.csv", { kind: "file", hdu: null })).toBe("jw_test_x1d_spectrum_spectrum.csv");
  });

  it("writes no wavelength column for a velocity spectrum without a rest wavelength and says why", () => {
    const velocity = airDocument({
      axis: axisOf("vopt", [-10, 0, 10], { ctype: "VOPT" }),
      wavelength_frame: "not_wavelength",
    });
    const csv = spectrumCsv(airExportInput(velocity, { mode: "velocity" }));
    expect(headerRow(csv)).toBe("channel,velocity_kms,flux,flux_unit");
    expect(csvLines(csv)).toContain(
      "# wavelength_note: velocity axis without a rest wavelength (RESTFRQ/RESTWAV absent, none typed)",
    );
  });
});

const REGION_ROWS = 6;
const REGION_WEIGHT = 12.5;
const REGION_JY_PER_SUM = 2.35e-5;
const REGION_FACTOR = 1.31;
const REGION_NAN_FLUX_ROW = 2;
const REGION_NAN_ERR_ROW = 4;
const REGION_WARNING =
  "the empirical noise is 1.31 +- 0.05 x the propagated error over 12 line-free channels, outside [0.8, 1.25] by more than 2 standard errors: the ERR plane does not describe the scatter of this spectrum";

function regionSum(i: number): number {
  return i === REGION_NAN_FLUX_ROW ? NaN : 3.5 + i * 0.25;
}

function regionErr(i: number): number {
  return i === REGION_NAN_ERR_ROW ? NaN : 0.02 + i * 0.001;
}

function erroredRegion(extra: Partial<RegionSpectrum> = {}): RegionSpectrum {
  const rows = Array.from({ length: REGION_ROWS }, (_, i) => i);
  return regionSpectrum({
    sum: rows.map(regionSum),
    mean: rows.map((i) => regionSum(i) / REGION_WEIGHT),
    npix: REGION_WEIGHT,
    n_bg: 0,
    bg_subtracted: false,
    flux_jy: rows.map((i) => regionSum(i) * REGION_JY_PER_SUM),
    err: rows.map(regionErr),
    err_scaled: rows.map((i) => regionErr(i) * REGION_FACTOR),
    mean_err: rows.map((i) => regionErr(i) / REGION_WEIGHT),
    used_weight: rows.map(() => REGION_WEIGHT),
    err_source: "ERR HDU 2",
    flux_jy_err: rows.map((i) => regionErr(i) * REGION_JY_PER_SUM),
    n_dq_excluded: rows.map((i) => (i === 1 ? 2 : 0)),
    noise_factor: {
      factor: REGION_FACTOR,
      factor_se: 0.05,
      n_channels: 12,
      windows: [
        [0, 1],
        [4, 5],
      ],
      warning: null,
    },
    ...extra,
  });
}

function regionErrorInput(view: "sum" | "mean" | "jy", region: RegionSpectrum = erroredRegion()): SpectrumExportInput {
  const rows = Array.from({ length: REGION_ROWS }, (_, i) => i);
  const values = view === "sum" ? region.sum : view === "mean" ? region.mean : region.flux_jy!;
  return exportInput({
    fileName: "jw_test_s3d.fits",
    axis: axisOf("wave", rows.map((i) => 2 + i * 0.001)),
    mode: "wavelength_vac",
    source: { kind: "region", shape: circle, background: null },
    regionId: "r1",
    regionText: null,
    backgroundId: null,
    view,
    values,
    region,
    exportedAtUtc: "2026-10-17T12:00:00.000Z",
  });
}

function regionFixture(view: "sum" | "mean" | "jy"): string {
  const path = fileURLToPath(new URL(`../../../src-tauri/tests/fixtures/spectrum1d/region_${view}_export.csv`, import.meta.url));
  return readFileSync(path, "utf8").replace(/\r\n/g, "\n");
}

function column(csv: string, header: string): string[] {
  const index = headerRow(csv).split(",").indexOf(header);
  expect(index, `${header} in ${headerRow(csv)}`).toBeGreaterThanOrEqual(0);
  return dataRows(csv).map((row) => row.split(",")[index]);
}

function cells(values: number[]): string[] {
  return values.map((v) => (Number.isFinite(v) ? String(v) : ""));
}

describe("spectrumCsv for a region spectrum with propagated errors", () => {
  it("writes each view exactly as the backend CSV reader test reads it back, err pairing the flux column of that view", () => {
    for (const view of ["sum", "mean", "jy"] as const) {
      const csv = spectrumCsv(regionErrorInput(view));
      expect(csv.replace(/\r\n/g, "\n"), view).toBe(regionFixture(view));
    }
  });

  it("writes err as the error of the exported flux: the sum error, the mean error or the Jy error", () => {
    const region = erroredRegion();
    expect(column(spectrumCsv(regionErrorInput("sum")), "err")).toEqual(cells(region.err!));
    expect(column(spectrumCsv(regionErrorInput("mean")), "err")).toEqual(cells(region.mean_err!));
    expect(column(spectrumCsv(regionErrorInput("jy")), "err")).toEqual(cells(region.flux_jy_err!));
  });

  it("writes err_scaled right after err, and only when the backend measured a noise factor", () => {
    const scaled = spectrumCsv(regionErrorInput("sum"));
    expect(headerRow(scaled).endsWith(",err,err_scaled")).toBe(true);
    expect(column(scaled, "err_scaled")).toEqual(cells(erroredRegion().err_scaled!));
    const unscaled = spectrumCsv(regionErrorInput("sum", erroredRegion({ err_scaled: null, noise_factor: null })));
    expect(headerRow(unscaled).endsWith(",n_bg,err")).toBe(true);
    expect(unscaled).not.toContain("err_scaled");
    expect(unscaled).not.toContain("# noise_factor:");
  });

  it("states the error source, what err holds in this view and the noise factor in the provenance", () => {
    const sum = csvLines(spectrumCsv(regionErrorInput("sum")));
    expect(sum).toContain("# err_source: ERR HDU 2");
    expect(sum).toContain("# err_basis: err is the propagated error of the flux column, the aperture sum; err_scaled = err x the noise factor");
    expect(sum).toContain("# noise_factor: 1.31 +- 0.05 (12 line-free channels, continuum 0-1 and 4-5)");
    const mean = csvLines(spectrumCsv(regionErrorInput("mean")));
    expect(mean).toContain(
      "# err_basis: err is the propagated error of the flux column, the aperture mean (mean_err = aperture-sum error / used weight); err_scaled = err x the noise factor",
    );
    expect(sum).not.toContain(`# noise_factor_warning: ${REGION_WARNING}`);
    const warned = erroredRegion();
    warned.noise_factor = { ...warned.noise_factor!, warning: REGION_WARNING };
    expect(csvLines(spectrumCsv(regionErrorInput("sum", warned)))).toContain(`# noise_factor_warning: ${REGION_WARNING}`);
  });

  it("keeps the export of a region without ERR free of error columns and error lines", () => {
    const csv = spectrumCsv(exportInput());
    expect(headerRow(csv)).toBe("channel,wavelength_vacuum_um,frequency_ghz,flux,flux_unit,flux_jy,npix,bg_subtracted,n_bg");
    expect(csv).not.toContain("# err_source:");
    expect(csv).not.toContain("# err_basis:");
  });

  it("summarises the error source and the noise factor of a region export, and nothing of them for a pixel", () => {
    const summary = spectrumExportSummary(regionErrorInput("mean"));
    expect(summary.params.err_source).toBe("ERR HDU 2");
    expect(summary.params.noise_factor).toBe(REGION_FACTOR);
    const bare = spectrumExportSummary(exportInput());
    expect(bare.params.err_source).toBe("none");
    expect(bare.params.noise_factor).toBeNull();
    const pixel = spectrumExportSummary(exportInput({ source: { kind: "pixel", x: 3, y: 4 }, region: null }));
    expect(pixel.params).not.toHaveProperty("err_source");
    expect(pixel.params).not.toHaveProperty("noise_factor");
  });
});

const SOFTWARE = "AstroBurst v0.8.0";
const DISPLAY: SpectrumExportDisplay = { wavelengthUnit: "um", fluxUnit: "native", binFactor: 1, dqExcluded: null };
const DOC_EXPORTED = "# exported_utc: 2026-10-17T12:00:00.000Z";
const INSTRUMENT_KEYS = ["instrument", "disperser", "filter", "detector", "telescope", "target", "source_type", "date_obs"];
const NIRSPEC_CARDS = {
  INSTRUME: "NIRSPEC",
  GRATING: "G235H",
  FILTER: "F170LP",
  DETECTOR: "NRS1",
  TARGNAME: "NGC 7469",
  SRCTYPE: "POINT",
};
const NIRSPEC_LINES = [
  "# instrument: NIRSPEC",
  "# disperser: G235H",
  "# filter: F170LP",
  "# detector: NRS1",
  "# target: NGC 7469",
  "# source_type: POINT",
];

function headerBlock(csv: string): string[] {
  const lines = csvLines(csv);
  return lines.slice(0, lines.findIndex((line) => !line.startsWith("#")));
}

function unitLinesOf(csv: string): string[] {
  return headerBlock(csv).filter((line) => line.startsWith("# unit "));
}

function instrumentLinesOf(csv: string): string[] {
  return headerBlock(csv).filter((line) => INSTRUMENT_KEYS.some((key) => line.startsWith(`# ${key}:`)));
}

function closeTo(cell: string, expected: number, relative: number): boolean {
  return Math.abs(Number(cell) - expected) <= relative * Math.abs(expected);
}

const DOC_INDICES = Array.from({ length: DOC_ROWS }, (_, i) => i);

describe("spectrumCsv provenance, instrument and units of the round", () => {
  it("keeps the 0.8.0 layout when no software name is given", () => {
    const golden = readFileSync(EXPORT_FIXTURE, "utf8").replace(/\r\n/g, "\n");
    for (const software of [undefined, "", "  "]) {
      const csv = spectrumCsv(documentExportInput(x1dDocument(), { software }));
      const block = headerBlock(csv);
      expect(block[0], String(software)).toBe("# file: jw_test_x1d.fits");
      expect(block[block.length - 1]).toBe(DOC_EXPORTED);
      expect(block.some((line) => line.startsWith("# unit "))).toBe(false);
      expect(csv.replace(/\r\n/g, "\n")).toBe(golden);
    }
  });

  it("starts with program, time and file and ends the header with the units", () => {
    const oldCsv = spectrumCsv(documentExportInput());
    const csv = spectrumCsv(documentExportInput(x1dDocument(), { software: SOFTWARE }));
    const lines = csvLines(csv);
    expect(lines.slice(0, 3)).toEqual(["# software: AstroBurst v0.8.0", DOC_EXPORTED, "# file: jw_test_x1d.fits"]);
    expect(lines.filter((line) => line.startsWith("# file:"))).toHaveLength(1);
    expect(lines.filter((line) => line.startsWith("# exported_utc:"))).toHaveLength(1);
    const firstUnit = lines.findIndex((line) => line.startsWith("# unit "));
    const oldOwn = headerBlock(oldCsv).filter((line) => !line.startsWith("# file:") && !line.startsWith("# exported_utc:"));
    expect(lines.slice(3, firstUnit)).toEqual(oldOwn);
    expect(headerBlock(csv).slice(-4)).toEqual([
      "# unit wavelength_vacuum_um: um",
      "# unit vacuum_wavelength_um: um",
      "# unit flux: Jy",
      "# unit err: Jy",
    ]);
    expect(headerRow(csv)).toBe(headerRow(oldCsv));
    expect(dataRows(csv)).toEqual(dataRows(oldCsv));

    const empty = spectrumCsv(documentExportInput(x1dDocument(), { software: SOFTWARE, values: [] }));
    expect(csvLines(empty).slice(0, 3)).toEqual(lines.slice(0, 3));
    expect(unitLinesOf(empty)).toEqual(["# unit flux: Jy", "# unit err: Jy"]);
    expect(headerRow(empty)).toBe("channel,flux,flux_unit,err,dq");
    expect(dataRows(empty)).toEqual([]);
  });

  it("writes no unit for a flux that has none", () => {
    const csv = spectrumCsv(airExportInput(airDocument({ flux_unit: null }), { software: SOFTWARE, mode: "wavelength_air" }));
    expect(unitLinesOf(csv)).toEqual([
      "# unit wavelength_vacuum_um: um",
      "# unit wavelength_air_um: um",
      "# unit air_wavelength_um: um",
    ]);
  });

  it("states the units of a region export", () => {
    expect(unitLinesOf(spectrumCsv({ ...regionErrorInput("sum"), software: SOFTWARE }))).toEqual([
      "# unit wavelength_vacuum_um: um",
      "# unit vacuum_wavelength_um: um",
      "# unit flux: MJy/sr x pix",
      "# unit flux_jy: Jy",
      "# unit err: MJy/sr x pix",
      "# unit err_scaled: MJy/sr x pix",
    ]);
    const counts = spectrumCsv({ ...regionErrorInput("sum"), software: SOFTWARE, bunit: "DN/s" });
    expect(unitLinesOf(counts)).toContain("# unit flux: DN/s x pix");
    expect(column(counts, "flux_unit")[0]).toBe("DN/s x pix");
    expect(headerBlock(counts)).toContain("# flux_unit: DN/s x pix");
    const velocity = spectrumCsv(documentExportInput(x1dDocument(), { software: SOFTWARE, mode: "velocity", restUm: 1.87 }));
    expect(headerRow(velocity)).toContain(",velocity_optical_kms,");
    expect(unitLinesOf(velocity)).toContain("# unit velocity_optical_kms: km/s");
  });

  it("names instrument, disperser and target of a spectrum file", () => {
    const nirspec = x1dDocument({ cards: NIRSPEC_CARDS });
    const round = headerBlock(spectrumCsv(documentExportInput(nirspec, { software: SOFTWARE })));
    const dq = round.indexOf("# dq_table: jwst");
    expect(dq).toBeGreaterThan(0);
    expect(round.slice(dq + 1, dq + 7)).toEqual(NIRSPEC_LINES);
    expect(round[dq + 7].startsWith("# unit ")).toBe(true);

    const old = headerBlock(spectrumCsv(documentExportInput(nirspec)));
    const exported = old.indexOf(DOC_EXPORTED);
    expect(exported).toBe(old.length - 1);
    expect(old.slice(exported - 6, exported)).toEqual(NIRSPEC_LINES);

    const ground = airDocument({ cards: { OBJECT: "'HD 37795 '", TELESCOP: "OHP", "DATE-OBS": "2019-01-05" } });
    expect(instrumentLinesOf(spectrumCsv(airExportInput(ground, { software: SOFTWARE })))).toEqual([
      "# telescope: OHP",
      "# target: HD 37795",
      "# date_obs: 2019-01-05",
    ]);

    const reopened = x1dDocument({
      source: { kind: "csv", path: "C:/data/jw_test_x1d_spectrum.csv" },
      cards: { instrument: "NIRSPEC", disperser: "G395H", target: "WASP-39" },
    });
    const reopenedCsv = spectrumCsv(
      documentExportInput(reopened, { fileName: "jw_test_x1d_spectrum.csv", source: { kind: "file", hdu: null }, software: SOFTWARE }),
    );
    expect(instrumentLinesOf(reopenedCsv)).toEqual(["# instrument: NIRSPEC", "# disperser: G395H", "# target: WASP-39"]);

    expect(instrumentLinesOf(spectrumCsv(documentExportInput(x1dDocument({ cards: {} }), { software: SOFTWARE })))).toEqual([]);
    expect(instrumentLinesOf(spectrumCsv({ ...regionErrorInput("sum"), software: SOFTWARE }))).toEqual([]);
    const pixel = exportInput({ source: { kind: "pixel", x: 3, y: 4 }, region: null, software: SOFTWARE });
    expect(instrumentLinesOf(spectrumCsv(pixel))).toEqual([]);
  });

  it("writes the displayed wavelength in the unit of the view", () => {
    const nm = spectrumCsv(documentExportInput(x1dDocument(), { display: { ...DISPLAY, wavelengthUnit: "nm" } }));
    expect(headerRow(nm)).toBe("channel,wavelength_vacuum_um,vacuum_wavelength_nm,flux,flux_unit,err,dq");
    expect(column(nm, "vacuum_wavelength_nm")).toEqual(cells(scaleWavelengths(DOC_INDICES.map(docWavelength), "nm")));
    expect(closeTo(column(nm, "vacuum_wavelength_nm")[0], 1800, 1e-12)).toBe(true);
    expect(column(nm, "wavelength_vacuum_um")).toEqual(column(spectrumCsv(documentExportInput()), "wavelength_vacuum_um"));
    const nmRound = spectrumCsv(
      documentExportInput(x1dDocument(), { software: SOFTWARE, display: { ...DISPLAY, wavelengthUnit: "nm" } }),
    );
    expect(unitLinesOf(nmRound)).toContain("# unit vacuum_wavelength_nm: nm");

    const angstrom = spectrumCsv(
      documentExportInput(x1dDocument(), { software: SOFTWARE, display: { ...DISPLAY, wavelengthUnit: "A" } }),
    );
    expect(headerRow(angstrom)).toBe("channel,wavelength_vacuum_um,vacuum_wavelength_angstrom,flux,flux_unit,err,dq");
    expect(unitLinesOf(angstrom)).toContain("# unit vacuum_wavelength_angstrom: Angstrom");
    expect(column(angstrom, "vacuum_wavelength_angstrom")).toEqual(cells(scaleWavelengths(DOC_INDICES.map(docWavelength), "A")));

    const air = spectrumCsv(airExportInput(airDocument(), { mode: "wavelength_air", display: { ...DISPLAY, wavelengthUnit: "A" } }));
    expect(headerRow(air)).toBe("channel,wavelength_vacuum_um,wavelength_air_um,air_wavelength_angstrom,flux,flux_unit");
    expect(column(air, "wavelength_air_um")).toEqual(cells(AIR_UM));

    for (const mode of ["frequency", "velocity"] as const) {
      const plain = spectrumCsv(documentExportInput(x1dDocument(), { mode, restUm: 1.87 }));
      const shown = spectrumCsv(documentExportInput(x1dDocument(), { mode, restUm: 1.87, display: { ...DISPLAY, wavelengthUnit: "nm" } }));
      expect(shown, mode).toBe(plain);
    }

    const region = spectrumCsv({ ...regionErrorInput("sum"), display: { ...DISPLAY, wavelengthUnit: "nm" } });
    expect(headerRow(region).split(",")).toContain("vacuum_wavelength_nm");

    const golden = readFileSync(EXPORT_FIXTURE, "utf8").replace(/\r\n/g, "\n");
    expect(spectrumCsv(documentExportInput(x1dDocument(), { display: null })).replace(/\r\n/g, "\n")).toBe(golden);
    expect(spectrumCsv(documentExportInput(x1dDocument(), { display: DISPLAY })).replace(/\r\n/g, "\n")).toBe(golden);
  });

  it("adds the flux of the view beside the flux of the file", () => {
    const doc = x1dDocument();
    const oldCsv = spectrumCsv(documentExportInput(doc));
    const csv = spectrumCsv(documentExportInput(doc, { display: { ...DISPLAY, fluxUnit: "mJy" } }));
    expect(headerRow(csv).endsWith(",err,dq,flux_display,err_display")).toBe(true);
    const fluxInput = { flux: doc.flux, err: doc.err, vacuumUm: doc.axis.values, fluxUnit: "Jy" };
    const d = fluxDisplay(fluxInput, "mJy");
    const shownFlux = column(csv, "flux_display");
    const shownErr = column(csv, "err_display");
    expect(shownFlux).toEqual(cells(d.values));
    expect(shownErr).toEqual(cells(d.errors!));
    expect(closeTo(shownFlux[0], 12.3, 1e-12)).toBe(true);
    expect(closeTo(shownErr[0], 0.1, 1e-12)).toBe(true);
    expect(shownFlux[5]).toBe("");
    expect(shownErr[7]).toBe("");
    expect(column(csv, "flux")).toEqual(column(oldCsv, "flux"));
    expect(column(csv, "err")).toEqual(column(oldCsv, "err"));
    expect(headerBlock(csv)).toContain("# display_flux_unit: mJy");
    expect(headerBlock(csv)).toContain("# display_flux_basis: Jy x 1000");
    const round = spectrumCsv(documentExportInput(doc, { software: SOFTWARE, display: { ...DISPLAY, fluxUnit: "mJy" } }));
    expect(unitLinesOf(round).slice(-2)).toEqual(["# unit flux_display: mJy", "# unit err_display: mJy"]);

    const flam = spectrumCsv(documentExportInput(doc, { display: { ...DISPLAY, fluxUnit: "flam_cgs" } }));
    const flamFlux = column(flam, "flux_display");
    expect(flamFlux).toEqual(cells(fluxDisplay(fluxInput, "flam_cgs").values));
    expect(closeTo(flamFlux[0], 1.1381009979629634e-15, 1e-12)).toBe(true);
    expect(headerBlock(flam)).toContain(
      "# display_flux_basis: F_lambda = F_nu x c / lambda^2 with the vacuum wavelength of each sample (c = 299792458 m/s)",
    );

    const ab = spectrumCsv(documentExportInput(doc, { display: { ...DISPLAY, fluxUnit: "ab" } }));
    expect(Math.abs(Number(column(ab, "flux_display")[0]) - 13.675237221401506)).toBeLessThanOrEqual(1e-12);

    const noErr = spectrumCsv(documentExportInput(x1dDocument({ err: null }), { display: { ...DISPLAY, fluxUnit: "mJy" } }));
    expect(headerRow(noErr)).toBe("channel,wavelength_vacuum_um,vacuum_wavelength_um,flux,flux_unit,dq,flux_display");
  });

  it("says why a display unit was not written", () => {
    const counts = x1dDocument({ flux_unit: "DN/s" });
    const csv = spectrumCsv(documentExportInput(counts, { software: SOFTWARE, display: { ...DISPLAY, fluxUnit: "Jy" } }));
    expect(headerRow(csv)).not.toContain("flux_display");
    expect(headerBlock(csv)).toContain("# display_flux_note: Jy is not available for flux in DN/s");
    expect(headerBlock(csv).some((line) => line.startsWith("# display_flux_unit:"))).toBe(false);
    expect(unitLinesOf(csv).some((line) => line.includes("_display"))).toBe(false);
    const old = headerBlock(spectrumCsv(documentExportInput(counts, { display: { ...DISPLAY, fluxUnit: "Jy" } })));
    expect(old[old.length - 2]).toBe("# display_flux_note: Jy is not available for flux in DN/s");
  });

  it("gives a cube export no display flux", () => {
    const shown: SpectrumExportDisplay = { ...DISPLAY, fluxUnit: "mJy" };
    expect(spectrumCsv({ ...regionErrorInput("jy"), display: shown })).toBe(spectrumCsv(regionErrorInput("jy")));
    expect(spectrumCsv({ ...regionErrorInput("jy"), software: SOFTWARE, display: shown })).toBe(
      spectrumCsv({ ...regionErrorInput("jy"), software: SOFTWARE }),
    );
  });

  it("adds the view to the export summary only when it changed the file", () => {
    const changed = spectrumExportSummary(
      documentExportInput(x1dDocument(), { display: { ...DISPLAY, wavelengthUnit: "nm", fluxUnit: "mJy" } }),
    );
    expect(changed.params.wavelength_unit).toBe("nm");
    expect(changed.params.display_flux_unit).toBe("mJy");
    for (const display of [DISPLAY, null, undefined]) {
      const summary = spectrumExportSummary(documentExportInput(x1dDocument(), { display }));
      expect(summary.params).not.toHaveProperty("wavelength_unit");
      expect(summary.params).not.toHaveProperty("display_flux_unit");
    }
  });

  it("stays a file the spectrum reader accepts", () => {
    const csv = spectrumCsv(
      documentExportInput(x1dDocument({ cards: NIRSPEC_CARDS }), {
        software: SOFTWARE,
        display: { ...DISPLAY, wavelengthUnit: "nm", fluxUnit: "mJy" },
      }),
    );
    const block = headerBlock(csv);
    expect(block.every((line) => line.startsWith("# "))).toBe(true);
    const header = headerRow(csv).split(",");
    for (const name of ["wavelength_vacuum_um", "flux", "err", "dq"]) {
      expect(header.filter((cell) => cell === name), name).toHaveLength(1);
    }
    const rows = dataRows(csv);
    expect(rows).toHaveLength(DOC_ROWS);
    for (const row of rows) expect(row.split(",")).toHaveLength(header.length);
    expect(block).toContain("# flux_unit: Jy");
    expect(block).toContain("# dq_table: jwst");
    expect(block.filter((line) => INSTRUMENT_KEYS.some((key) => line.startsWith(`# ${key}:`)))).toEqual(NIRSPEC_LINES);
  });
});

const BINNED_DISPLAY: SpectrumExportDisplay = { ...DISPLAY, binFactor: 4 };
const BINNING_LINE_4 =
  "# binning: factor 4, 3 bins of 12 channels; flux = mean of the samples used in a bin; err = sqrt(sum err^2) / n_used, the channels treated as independent";
const BINNING_LINES_4 = [BINNING_LINE_4, "# binning_samples: 11 used, 0 excluded by DQ, 1 not finite", "# binning_dq: not applied"];

function binningLinesOf(csv: string): string[] {
  return headerBlock(csv).filter((line) => line.startsWith("# binning"));
}

function expectNear(actual: string[], expected: readonly (number | null)[]): void {
  expect(actual).toHaveLength(expected.length);
  expected.forEach((value, i) => {
    if (value === null) expect(actual[i], `cell ${i}`).toBe("");
    else expect(closeTo(actual[i], value, 1e-12), `cell ${i}: ${actual[i]} vs ${value}`).toBe(true);
  });
}

describe("spectrumCsv of a binned spectrum file", () => {
  it("writes the bins of a binned spectrum file instead of its channels", () => {
    const doc = x1dDocument();
    const csv = spectrumCsv(documentExportInput(doc, { display: BINNED_DISPLAY }));
    expect(headerRow(csv)).toBe("bin,channel_first,channel_last,n_used,wavelength_vacuum_um,vacuum_wavelength_um,flux,flux_unit,err");
    expect(dataRows(csv)).toHaveLength(3);
    expect(column(csv, "bin")).toEqual(["0", "1", "2"]);
    expect(column(csv, "channel_first")).toEqual(["0", "4", "8"]);
    expect(column(csv, "channel_last")).toEqual(["3", "7", "11"]);
    expect(column(csv, "n_used")).toEqual(["4", "3", "4"]);
    expectNear(column(csv, "flux"), [0.0123684, 0.0125584, 0.0127332]);
    expectNear(column(csv, "err"), [5.0753078724349326e-5, null, 5.475285380690216e-5]);
    expectNear(column(csv, "wavelength_vacuum_um"), [1.802055, 1.807535, 1.813015]);
    const bins = binSpectrum(doc.flux, doc.err, null, 4);
    const centres = cells(binAxis(doc.axis.values, 4));
    expect(column(csv, "flux")).toEqual(cells(bins.flux));
    expect(column(csv, "err")).toEqual(cells(bins.err!));
    expect(column(csv, "wavelength_vacuum_um")).toEqual(centres);
    expect(column(csv, "vacuum_wavelength_um")).toEqual(centres);
    expect(column(csv, "flux_unit")).toEqual(["Jy", "Jy", "Jy"]);
    expect(headerRow(csv).split(",")).not.toContain("dq");
    expect(csvLines(csv).some((line) => line.startsWith("# dq_table:"))).toBe(false);
  });

  it("applies the DQ exclusions of the view before binning and says so", () => {
    const doc = x1dDocument();
    const mask = documentDqExclusions(doc);
    expect(mask).toEqual(DOC_INDICES.map((i) => i === 3));
    const csv = spectrumCsv(documentExportInput(doc, { display: { ...BINNED_DISPLAY, dqExcluded: mask } }));
    expect(column(csv, "n_used")).toEqual(["3", "3", "4"]);
    expectNear(column(csv, "flux").slice(0, 1), [0.0123456]);
    expectNear(column(csv, "err").slice(0, 1), [5.831428260345449e-5]);
    expect(column(csv, "flux")).toEqual(cells(binSpectrum(doc.flux, doc.err, mask, 4).flux));
    expect(headerBlock(csv)).toContain("# binning_dq: jwst exclusions applied before binning");
    expect(headerBlock(csv)).toContain("# binning_samples: 10 used, 1 excluded by DQ, 1 not finite");
    const off = spectrumCsv(documentExportInput(doc, { display: { ...BINNED_DISPLAY, dqExcluded: null } }));
    expect(binningLinesOf(off)).toEqual(BINNING_LINES_4);
    expect(column(off, "n_used")).toEqual(["4", "3", "4"]);
    const wrongLength = spectrumCsv(
      documentExportInput(doc, { display: { ...BINNED_DISPLAY, dqExcluded: [true, true, true, true, true] } }),
    );
    expect(wrongLength).toBe(off);

    const emptyBin = spectrumCsv(
      documentExportInput(doc, { display: { ...BINNED_DISPLAY, dqExcluded: DOC_INDICES.map((i) => i <= 3) } }),
    );
    expect(dataRows(emptyBin)).toHaveLength(3);
    expect(column(emptyBin, "n_used")[0]).toBe("0");
    expect(column(emptyBin, "flux")[0]).toBe("");
    expect(column(emptyBin, "err")[0]).toBe("");
    expect(headerBlock(emptyBin)).toContain("# binning_samples: 7 used, 4 excluded by DQ, 1 not finite");
  });

  it("states the binning in the header", () => {
    expect(binningLinesOf(spectrumCsv(documentExportInput(x1dDocument(), { display: BINNED_DISPLAY })))).toEqual(BINNING_LINES_4);
    expect(BINNING_LINE_4.endsWith(`; ${BINNING_BASIS}`)).toBe(true);

    const five = spectrumCsv(documentExportInput(x1dDocument(), { display: { ...DISPLAY, binFactor: 5 } }));
    expect(binningLinesOf(five)[0]).toBe(`# binning: factor 5, 3 bins of 12 channels, the last bin holds 2; ${BINNING_BASIS}`);
    expect(column(five, "n_used")).toEqual(["5", "4", "2"]);
    expectNear(column(five, "flux"), [0.0123912, 0.012642, 0.0127788]);

    const whole = spectrumCsv(documentExportInput(x1dDocument(), { display: { ...DISPLAY, binFactor: 50 } }));
    expect(binningLinesOf(whole)[0]).toBe(`# binning: factor 12, 1 bin of 12 channels; ${BINNING_BASIS}`);
    expect(column(whole, "channel_first")).toEqual(["0"]);
    expect(column(whole, "channel_last")).toEqual(["11"]);
    expect(column(whole, "n_used")).toEqual(["11"]);

    const nirspec = x1dDocument({ cards: NIRSPEC_CARDS });
    const block = headerBlock(spectrumCsv(documentExportInput(nirspec, { software: SOFTWARE, display: BINNED_DISPLAY })));
    const firstUnit = block.findIndex((line) => line.startsWith("# unit "));
    expect(block.slice(firstUnit - 3, firstUnit)).toEqual(BINNING_LINES_4);
    expect(block.slice(firstUnit - 9, firstUnit - 3)).toEqual(NIRSPEC_LINES);
    expect(block.slice(firstUnit)).toEqual([
      "# unit wavelength_vacuum_um: um",
      "# unit vacuum_wavelength_um: um",
      "# unit flux: Jy",
      "# unit err: Jy",
    ]);
    const channels = headerBlock(spectrumCsv(documentExportInput(nirspec, { software: SOFTWARE })));
    const channelsOwn = channels.slice(0, channels.findIndex((line) => line.startsWith("# unit ")));
    expect(block.slice(0, firstUnit - 3)).toEqual(channelsOwn.filter((line) => !line.startsWith("# dq_table:")));

    const bare = headerBlock(spectrumCsv(documentExportInput(x1dDocument(), { software: SOFTWARE, display: BINNED_DISPLAY })));
    const bareUnit = bare.findIndex((line) => line.startsWith("# unit "));
    expect(bare.slice(bareUnit - 4, bareUnit)).toEqual([
      "# flux_jy_note: spectrum file: flux exported in its own unit (flux_unit)",
      ...BINNING_LINES_4,
    ]);
  });

  it("bins every wavelength column by its own mean", () => {
    const csv = spectrumCsv(airExportInput(airDocument(), { mode: "wavelength_air", display: { ...DISPLAY, binFactor: 2 } }));
    expect(headerRow(csv)).toBe(
      "bin,channel_first,channel_last,n_used,wavelength_vacuum_um,wavelength_air_um,air_wavelength_um,flux,flux_unit",
    );
    expectNear(column(csv, "wavelength_air_um"), [0.656285, 0.6563]);
    expectNear(column(csv, "air_wavelength_um"), [0.656285, 0.6563]);
    expect(column(csv, "wavelength_vacuum_um")).toEqual(cells(binAxis(AIR_UM.map(airToVacuumUm), 2)));
    expect(column(csv, "flux")).toEqual(["2", "3.5"]);
    expect(column(csv, "n_used")).toEqual(["2", "1"]);
    expect(binningLinesOf(csv)[0]).toBe(`# binning: factor 2, 2 bins of 3 channels, the last bin holds 1; ${BINNING_BASIS}`);

    const angstrom = spectrumCsv(
      airExportInput(airDocument(), { mode: "wavelength_air", display: { ...DISPLAY, wavelengthUnit: "A", binFactor: 2 } }),
    );
    expect(headerRow(angstrom)).toBe(
      "bin,channel_first,channel_last,n_used,wavelength_vacuum_um,wavelength_air_um,air_wavelength_angstrom,flux,flux_unit",
    );
    expectNear(column(angstrom, "air_wavelength_angstrom"), [6562.85, 6563]);
    expect(column(angstrom, "wavelength_air_um")).toEqual(column(csv, "wavelength_air_um"));
  });

  it("converts the bins for the display columns", () => {
    const doc = x1dDocument();
    const csv = spectrumCsv(documentExportInput(doc, { display: { ...BINNED_DISPLAY, fluxUnit: "mJy" } }));
    expect(headerRow(csv).endsWith(",err,flux_display,err_display")).toBe(true);
    expectNear(column(csv, "flux_display"), [12.3684, 12.5584, 12.7332]);
    expectNear(column(csv, "err_display"), [0.050753078724349324, null, 0.05475285380690216]);
    const block = headerBlock(csv);
    expect(block).toContain("# display_flux_unit: mJy");
    expect(block.indexOf("# display_flux_unit: mJy")).toBe(block.indexOf("# binning_dq: not applied") + 1);

    const bins = binSpectrum(doc.flux, doc.err, null, 4);
    const flam = spectrumCsv(documentExportInput(doc, { display: { ...BINNED_DISPLAY, fluxUnit: "flam_cgs" } }));
    const expected = fluxDisplay({ flux: bins.flux, err: bins.err, vacuumUm: binAxis(doc.axis.values, 4), fluxUnit: "Jy" }, "flam_cgs");
    expect(column(flam, "flux_display")).toEqual(cells(expected.values));
    expect(column(flam, "flux")).toEqual(column(csv, "flux"));
  });

  it("leaves every other export as it was", () => {
    const plain = spectrumCsv(documentExportInput());
    for (const display of [{ ...DISPLAY, binFactor: 1 }, { ...DISPLAY, binFactor: 0 }, null]) {
      expect(spectrumCsv(documentExportInput(x1dDocument(), { display })), JSON.stringify(display)).toBe(plain);
    }
    expect(spectrumCsv(documentExportInput(x1dDocument(), { software: SOFTWARE, display: DISPLAY }))).toBe(
      spectrumCsv(documentExportInput(x1dDocument(), { software: SOFTWARE })),
    );
    expect(spectrumCsv({ ...regionErrorInput("sum"), display: BINNED_DISPLAY })).toBe(spectrumCsv(regionErrorInput("sum")));
    expect(spectrumCsv({ ...regionErrorInput("sum"), display: BINNED_DISPLAY }).replace(/\r\n/g, "\n")).toBe(regionFixture("sum"));
    const pixel = exportInput({ source: { kind: "pixel", x: 3, y: 4 }, region: null });
    expect(spectrumCsv({ ...pixel, display: BINNED_DISPLAY })).toBe(spectrumCsv(pixel));

    const empty = spectrumCsv(documentExportInput(x1dDocument(), { values: [], display: BINNED_DISPLAY }));
    expect(headerRow(empty)).toBe("channel,flux,flux_unit,err,dq");
    expect(dataRows(empty)).toEqual([]);
    expect(binningLinesOf(empty)).toEqual([]);
  });

  it("names a binned export", () => {
    const file = { kind: "file", hdu: 1 } as const;
    expect(spectrumCsvFileName(DOC_PATH, file, BINNED_DISPLAY)).toBe("jw_test_x1d_spectrum_bin4.csv");
    expect(spectrumCsvFileName(DOC_PATH, file, { ...DISPLAY, binFactor: 1 })).toBe("jw_test_x1d_spectrum.csv");
    expect(spectrumCsvFileName(DOC_PATH, file, null)).toBe("jw_test_x1d_spectrum.csv");
    expect(spectrumCsvFileName(DOC_PATH, file)).toBe("jw_test_x1d_spectrum.csv");
    expect(spectrumCsvFileName("C:/d/c.fits", { kind: "pixel", x: 3, y: 4 }, BINNED_DISPLAY)).toBe("c_spectrum_pixel_3_4.csv");
  });

  it("records the binning in the export summary", () => {
    const doc = x1dDocument();
    const summary = spectrumExportSummary(
      documentExportInput(doc, { display: { ...BINNED_DISPLAY, dqExcluded: documentDqExclusions(doc) } }),
    );
    expect(summary.params).toMatchObject({
      bin_factor: 4,
      n_bins: 3,
      bin_dq_requested: true,
      bin_dq_excluded: 1,
      n_channels: 12,
      dq_column: false,
      dq_table: null,
    });
    expect(Math.abs((summary.values.flux_max as number) - 0.0127332)).toBeLessThanOrEqual(1e-12 * 0.0127332);
    const sum = 0.0123456 + 0.0125584 + 0.0127332;
    expect(Math.abs((summary.values.flux_sum as number) - sum)).toBeLessThanOrEqual(1e-12 * sum);
    const channels = spectrumExportSummary(documentExportInput(doc, { display: DISPLAY }));
    for (const key of ["bin_factor", "n_bins", "bin_dq_requested", "bin_dq_excluded"]) {
      expect(channels.params, key).not.toHaveProperty(key);
    }
    expect(channels.params.dq_column).toBe(true);
  });
});
