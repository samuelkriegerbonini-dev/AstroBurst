import { describe, it, expect } from "vitest";
import {
  SPEED_OF_LIGHT_KMS,
  AIR_FORMULA_MIN_UM,
  AIR_VACUUM_FORMULA,
  airRefractiveIndex,
  vacuumToAirUm,
  airToVacuumUm,
  velocityKms,
  wavelengthFromVelocityUm,
  frequencyGhzFromWavelengthUm,
  wavelengthUmFromFrequencyGhz,
  availableModes,
  formatAxis,
  modeLabel,
  modeUnit,
  conventionLabel,
  formatAxisValue,
  applyCorrectionKms,
  parseRestInput,
  defaultRestUm,
} from "../spectralAxis";
import type { SpectralAxisInfo, SpectralAxisKind } from "../../shared/types/spectral";

const GOLDEN = {
  refractiveIndexAt0_5: 1.00029434858,
  vacuumFromAir0_5: 0.500147172245,
  airFromVacuum0_6563: 0.656108763679,
  vacuumFromAir0_6563: 0.656491290559,
  hAlphaOptical: 1004.94195886,
  hAlphaRadio: 1001.584521792,
  hAlphaRelativistic: 1003.257622482,
  coRestUm: 1300.403655796,
};

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

describe("air and vacuum wavelengths", () => {
  it("matches the Rust golden values for the Greisen formula", () => {
    expect(airRefractiveIndex(0.5)).toBeCloseTo(GOLDEN.refractiveIndexAt0_5, 10);
    expect(airToVacuumUm(0.5)).toBeCloseTo(GOLDEN.vacuumFromAir0_5, 10);
    expect(airToVacuumUm(0.5) * 1e4).toBeCloseTo(5001.472, 2);
    expect(vacuumToAirUm(0.6563)).toBeCloseTo(GOLDEN.airFromVacuum0_6563, 10);
    expect(airToVacuumUm(0.6563)).toBeCloseTo(GOLDEN.vacuumFromAir0_6563, 10);
    expect(AIR_VACUUM_FORMULA).toContain("Greisen");
  });

  it("round trips within 1e-6 and passes short wavelengths through unchanged", () => {
    for (const lambda of [0.2001, 0.3, 0.5, 0.6563, 1.0, 2.2, 5.0, 20.0]) {
      expect(Math.abs(vacuumToAirUm(airToVacuumUm(lambda)) - lambda)).toBeLessThan(1e-6 * lambda);
      expect(Math.abs(airToVacuumUm(vacuumToAirUm(lambda)) - lambda)).toBeLessThan(1e-6 * lambda);
      expect(airToVacuumUm(lambda)).toBeGreaterThan(lambda);
    }
    expect(AIR_FORMULA_MIN_UM).toBe(0.2);
    expect(airToVacuumUm(0.15)).toBe(0.15);
    expect(vacuumToAirUm(0.1)).toBe(0.1);
    expect(Number.isNaN(airToVacuumUm(NaN))).toBe(true);
  });
});

describe("velocity conventions", () => {
  it("matches the Rust golden values for H-alpha shifted to 0.6585 um", () => {
    expect(SPEED_OF_LIGHT_KMS).toBe(299792.458);
    expect(velocityKms(0.6585, 0.6563, "optical")).toBeCloseTo(GOLDEN.hAlphaOptical, 8);
    expect(velocityKms(0.6585, 0.6563, "radio")).toBeCloseTo(GOLDEN.hAlphaRadio, 8);
    expect(velocityKms(0.6585, 0.6563, "relativistic")).toBeCloseTo(GOLDEN.hAlphaRelativistic, 8);
    expect(velocityKms(0.6585, 0.6563, "optical")).toBeCloseTo((SPEED_OF_LIGHT_KMS * 0.0022) / 0.6563, 9);
  });

  it("inverts each convention and is zero at the rest wavelength", () => {
    for (const convention of ["optical", "radio", "relativistic"] as const) {
      const v = velocityKms(0.6585, 0.6563, convention);
      expect(wavelengthFromVelocityUm(v, 0.6563, convention)).toBeCloseTo(0.6585, 12);
      expect(velocityKms(0.6563, 0.6563, convention)).toBe(0);
    }
  });

  it("converts frequency and wavelength both ways", () => {
    expect(wavelengthUmFromFrequencyGhz(230.538)).toBeCloseTo(GOLDEN.coRestUm, 8);
    expect(frequencyGhzFromWavelengthUm(GOLDEN.coRestUm)).toBeCloseTo(230.538, 8);
    expect(wavelengthUmFromFrequencyGhz(frequencyGhzFromWavelengthUm(2.5))).toBeCloseTo(2.5, 12);
  });
});

describe("axis modes", () => {
  it("offers every mode for wavelength and frequency axes and only velocity for velocity axes", () => {
    expect(availableModes(axisOf("wave", [1]))).toEqual(["wavelength_vac", "wavelength_air", "frequency", "velocity"]);
    expect(availableModes(axisOf("awav", [1]))).toEqual(["wavelength_vac", "wavelength_air", "frequency", "velocity"]);
    expect(availableModes(axisOf("freq", [100]))).toEqual(["wavelength_vac", "wavelength_air", "frequency", "velocity"]);
    expect(availableModes(axisOf("vrad", [1]))).toEqual(["velocity"]);
    expect(availableModes(axisOf("velo", [1]))).toEqual(["velocity"]);
    expect(availableModes(axisOf("zopt", [0.01]))).toEqual(["velocity"]);
    expect(availableModes(axisOf("unknown", [1]))).toEqual([]);
    expect(availableModes(null)).toEqual([]);
  });

  it("labels modes and units", () => {
    expect(modeLabel("wavelength_vac")).toBe("Vacuum wavelength");
    expect(modeLabel("wavelength_air")).toBe("Air wavelength");
    expect(modeLabel("frequency")).toBe("Frequency");
    expect(modeLabel("velocity")).toBe("Velocity");
    expect(modeUnit("wavelength_vac")).toBe("μm");
    expect(modeUnit("frequency")).toBe("GHz");
    expect(modeUnit("velocity")).toBe("km/s");
    expect(conventionLabel("relativistic")).toBe("Relativistic");
  });
});

describe("formatAxis", () => {
  const wave = axisOf("wave", [0.6563, 0.6585]);

  it("returns vacuum wavelengths as stored and converts to air, frequency and velocity", () => {
    const vac = formatAxis(wave, "wavelength_vac", null, "optical");
    expect(vac).not.toBeNull();
    expect(vac!.values).toEqual([0.6563, 0.6585]);
    expect(vac!.unit).toBe("μm");
    expect(vac!.label).toBe("Vacuum wavelength (μm)");
    const air = formatAxis(wave, "wavelength_air", null, "optical")!;
    expect(air.values[0]).toBeCloseTo(GOLDEN.airFromVacuum0_6563, 10);
    expect(air.notes.some((n) => n.includes("Greisen"))).toBe(true);
    const freq = formatAxis(wave, "frequency", null, "optical")!;
    expect(freq.values[0]).toBeCloseTo(SPEED_OF_LIGHT_KMS / 0.6563, 8);
    expect(freq.unit).toBe("GHz");
    const vel = formatAxis(wave, "velocity", 0.6563, "radio")!;
    expect(vel.values[0]).toBe(0);
    expect(vel.values[1]).toBeCloseTo(GOLDEN.hAlphaRadio, 8);
    expect(vel.label).toBe("Velocity, radio (km/s)");
    expect(vel.notes.some((n) => n.includes("0.6563 um taken as a vacuum wavelength"))).toBe(true);
  });

  it("converts a redshift axis to velocity from 1 + z without a rest wavelength", () => {
    const zopt = axisOf("zopt", [0, 0.001]);
    const optical = formatAxis(zopt, "velocity", null, "optical")!;
    expect(optical.values[0]).toBe(0);
    expect(optical.values[1]).toBeCloseTo(SPEED_OF_LIGHT_KMS * 0.001, 8);
    expect(optical.unit).toBe("km/s");
    expect(optical.label).toBe("Velocity, optical (km/s)");
    expect(optical.notes.some((n) => n.includes("1 + z"))).toBe(true);
    const relativistic = formatAxis(zopt, "wavelength_vac", null, "relativistic")!;
    expect(relativistic.values[1]).toBeCloseTo(velocityKms(1.001, 1, "relativistic"), 10);
    expect(relativistic.unit).toBe("km/s");
  });

  it("flags air wavelengths below the formula cutoff that are left unchanged", () => {
    const uv = axisOf("awav", [0.15, 0.25]);
    const vac = formatAxis(uv, "wavelength_vac", null, "optical")!;
    expect(vac.values[0]).toBe(0.15);
    expect(vac.values[1]).toBeCloseTo(airToVacuumUm(0.25), 12);
    expect(vac.notes.some((n) => n.includes(`below ${AIR_FORMULA_MIN_UM} um`) && n.includes("left unchanged"))).toBe(true);
    const visible = formatAxis(axisOf("awav", [0.5, NaN]), "wavelength_vac", null, "optical")!;
    expect(visible.notes.some((n) => n.includes("left unchanged"))).toBe(false);
    expect(visible.notes.some((n) => n.includes("Greisen"))).toBe(true);
  });

  it("needs a rest wavelength for velocity and converts air axes to vacuum first", () => {
    expect(formatAxis(wave, "velocity", null, "optical")).toBeNull();
    expect(formatAxis(wave, "velocity", 0, "optical")).toBeNull();
    const awav = axisOf("awav", [GOLDEN.airFromVacuum0_6563]);
    const vac = formatAxis(awav, "wavelength_vac", null, "optical")!;
    expect(vac.values[0]).toBeCloseTo(0.6563, 9);
    expect(vac.notes.some((n) => n.includes("Greisen"))).toBe(true);
    const air = formatAxis(awav, "wavelength_air", null, "optical")!;
    expect(air.values[0]).toBe(GOLDEN.airFromVacuum0_6563);
    const freq = axisOf("freq", [230.538]);
    const vel = formatAxis(freq, "velocity", GOLDEN.coRestUm, "radio")!;
    expect(Math.abs(vel.values[0])).toBeLessThan(1e-6);
    const wl = formatAxis(freq, "wavelength_vac", null, "optical")!;
    expect(wl.values[0]).toBeCloseTo(GOLDEN.coRestUm, 8);
  });

  it("passes velocity axes through in km/s whatever the mode and refuses unknown axes", () => {
    const vrad = axisOf("vrad", [-100, -98]);
    const asVelocity = formatAxis(vrad, "velocity", 0.6563, "optical")!;
    expect(asVelocity.values).toEqual([-100, -98]);
    expect(asVelocity.notes.some((n) => n.includes("already a velocity axis"))).toBe(true);
    const asWavelength = formatAxis(vrad, "wavelength_vac", null, "optical")!;
    expect(asWavelength.values).toEqual([-100, -98]);
    expect(asWavelength.unit).toBe("km/s");
    expect(formatAxis(axisOf("unknown", [1, 2]), "wavelength_vac", null, "optical")).toBeNull();
    expect(formatAxis(null, "wavelength_vac", null, "optical")).toBeNull();
  });

  it("keeps NaN channels as NaN", () => {
    const withGap = axisOf("wave", [0.5, NaN, 0.6]);
    const air = formatAxis(withGap, "wavelength_air", null, "optical")!;
    expect(Number.isNaN(air.values[1])).toBe(true);
    expect(air.values[0]).toBeCloseTo(vacuumToAirUm(0.5), 12);
  });
});

describe("rest wavelength and corrections", () => {
  it("prefills the rest wavelength from the axis and parses user input", () => {
    expect(defaultRestUm(axisOf("wave", [1], { rest_wavelength_um: 0.6563 }))).toBe(0.6563);
    expect(defaultRestUm(axisOf("wave", [1]))).toBeNull();
    expect(defaultRestUm(null)).toBeNull();
    expect(parseRestInput("0.6563")).toBe(0.6563);
    expect(parseRestInput(" 1e-1 ")).toBe(0.1);
    expect(parseRestInput("")).toBeNull();
    expect(parseRestInput("abc")).toBeNull();
    expect(parseRestInput("-1")).toBeNull();
    expect(parseRestInput("0")).toBeNull();
  });

  it("adds the chosen frame's correction to a velocity axis and leaves other frames alone", () => {
    const correction = {
      barycentric_kms: 12.5,
      heliocentric_kms: 12.49,
      jd_mid: 2461302.5,
      method: "Meeus low-precision ephemeris",
      accuracy_kms: 0.02,
      notes: [],
      ra_deg: 180,
      dec_deg: 0,
      coordinate_source: "CRVAL1/CRVAL2",
    };
    expect(applyCorrectionKms([0, 10], correction, "barycentric")).toEqual([12.5, 22.5]);
    const heliocentric = applyCorrectionKms([0, 10], correction, "heliocentric");
    expect(heliocentric[0]).toBeCloseTo(12.49, 12);
    expect(heliocentric[1]).toBeCloseTo(22.49, 12);
    expect(applyCorrectionKms([0, 10], correction, "none")).toEqual([0, 10]);
    expect(applyCorrectionKms([0, 10], null, "barycentric")).toEqual([0, 10]);
    expect(applyCorrectionKms([0, NaN], correction, "barycentric")[1]).toBeNaN();
  });

  it("formats axis values with a precision that suits the unit", () => {
    expect(formatAxisValue(0.65630001, "μm")).toBe("0.6563");
    expect(formatAxisValue(230.538, "GHz")).toBe("230.5380");
    expect(formatAxisValue(1004.94195886, "km/s")).toBe("1004.9");
    expect(formatAxisValue(NaN, "km/s")).toBe("—");
  });
});
