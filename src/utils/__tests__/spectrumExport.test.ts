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
  type SpectrumExportInput,
} from "../spectrumExport";
import { shapeCentre } from "../regionGeometry";
import { airToVacuumUm, velocityKms, wavelengthFromVelocityUm, SPEED_OF_LIGHT_KMS } from "../spectralAxis";
import type { RadialVelocityCorrectionResult, SpectralAxisInfo, SpectralAxisKind, SpectrumSource } from "../../shared/types/spectral";
import type { Region, RegionShape } from "../../shared/types/regions";
import type { RegionSpectrum } from "../../shared/types/cube";

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
