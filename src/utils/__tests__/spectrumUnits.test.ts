import { describe, it, expect } from "vitest";
import {
  WAVELENGTH_DISPLAY_UNITS,
  WAVELENGTH_UNIT_FACTOR,
  WAVELENGTH_UNIT_LABEL,
  WAVELENGTH_UNIT_ASCII,
  scaleWavelengths,
  wavelengthUnitApplies,
  nativeFluxLabel,
  fluxDisplayChoices,
  fluxChoiceLabel,
  fluxDisplay,
  fluxAxisTitle,
} from "../spectrumUnits";
import type { FluxDisplayInput } from "../spectrumUnits";
import { convertFlux, parseSpectralFluxUnit } from "../fluxUnits";
import { fluxUnitLabel } from "../spectrumExport";
import type { FluxDisplayChoice } from "../../shared/types/spectrum1d";

const RELATIVE_TOLERANCE = 1e-12;
const FLAM_CGS_PER_JY_UM2 = 2.99792458e-13;
const MAG_ERR_PER_RELATIVE_FLUX_ERR = 1.085736204758129;
const ALL_CHOICES: FluxDisplayChoice[] = ["native", "Jy", "mJy", "uJy", "flam_cgs", "flam_si", "ab"];
const FNU_TO_FLAMBDA_BASIS =
  "F_lambda = F_nu x c / lambda^2 with the vacuum wavelength of each sample (c = 299792458 m/s)";
const FLAMBDA_TO_FNU_BASIS =
  "F_nu = F_lambda x lambda^2 / c with the vacuum wavelength of each sample (c = 299792458 m/s)";
const AB_BASIS = "AB = -2.5 log10(F_nu / Jy) + 8.90; error = 1.0857 x err / flux";

function expectClose(got: number, want: number, tolerance = RELATIVE_TOLERANCE): void {
  expect(Number.isFinite(got)).toBe(true);
  expect(Math.abs(got - want)).toBeLessThanOrEqual(tolerance * Math.abs(want));
}

function expectAllClose(got: readonly number[] | null, want: readonly number[]): void {
  expect(got).not.toBeNull();
  const values = got ?? [];
  expect(values).toHaveLength(want.length);
  want.forEach((expected, i) => {
    if (Number.isNaN(expected)) expect(Number.isNaN(values[i])).toBe(true);
    else expectClose(values[i], expected);
  });
}

function nullAsNaN(values: readonly (number | null)[] | null): number[] | null {
  return values === null ? null : values.map((value) => value ?? NaN);
}

const NIRSPEC_JY: FluxDisplayInput = {
  flux: [0.0194, 0.0123],
  err: [0.0002, NaN],
  vacuumUm: [2.4153, 1.8],
  fluxUnit: "Jy",
};

const HALPHA_FLAM: FluxDisplayInput = {
  flux: [1e-17, 1.5e-16],
  err: null,
  vacuumUm: [0.6563, 0.6575],
  fluxUnit: "erg/s/cm2/A",
};

describe("spectrumUnits", () => {
  it("scales wavelengths to nanometres and Angstrom", () => {
    const input = [0.6555, 2.4153, NaN];
    const nm = scaleWavelengths(input, "nm");
    expect(nm).toHaveLength(3);
    expectClose(nm[0], 655.5);
    expectClose(nm[1], 2415.3);
    expect(Number.isNaN(nm[2])).toBe(true);
    const angstrom = scaleWavelengths(input, "A");
    expect(angstrom).toHaveLength(3);
    expectClose(angstrom[0], 6555);
    expectClose(angstrom[1], 24153);
    expect(Number.isNaN(angstrom[2])).toBe(true);
    const um = scaleWavelengths(input, "um");
    expect(um).toEqual(input);
    expect(um).not.toBe(input);
    expect(WAVELENGTH_DISPLAY_UNITS).toEqual(["um", "nm", "A"]);
    expect(WAVELENGTH_UNIT_FACTOR).toEqual({ um: 1, nm: 1e3, A: 1e4 });
    expect(WAVELENGTH_UNIT_LABEL).toEqual({ um: "μm", nm: "nm", A: "Å" });
    expect(WAVELENGTH_UNIT_ASCII).toEqual({ um: "um", nm: "nm", A: "Angstrom" });
  });

  it("applies a wavelength unit only to a wavelength axis in micrometres", () => {
    expect(wavelengthUnitApplies("wavelength_vac", "μm")).toBe(true);
    expect(wavelengthUnitApplies("wavelength_air", "μm")).toBe(true);
    expect(wavelengthUnitApplies("frequency", "GHz")).toBe(false);
    expect(wavelengthUnitApplies("velocity", "km/s")).toBe(false);
    expect(wavelengthUnitApplies("wavelength_vac", "nm")).toBe(false);
    expect(wavelengthUnitApplies("wavelength_vac", "ch")).toBe(false);
  });

  it("names the unit of the file as the export does", () => {
    const cases: [string | null, string][] = [
      ["Jy", "Jy"],
      [" Jy ", "Jy"],
      ["'erg/s/cm2/A'", "erg/s/cm2/A"],
      ["", "native"],
      [null, "native"],
    ];
    for (const [unit, expected] of cases) {
      expect(nativeFluxLabel(unit)).toBe(expected);
      expect(nativeFluxLabel(unit)).toBe(fluxUnitLabel(unit, "sum", "file"));
    }
  });

  it("offers the units a file can be shown in", () => {
    expect(fluxDisplayChoices("Jy", true)).toEqual(ALL_CHOICES);
    expect(fluxDisplayChoices("Jy", false)).toEqual(["native", "Jy", "mJy", "uJy", "ab"]);
    expect(fluxDisplayChoices("erg/s/cm2/A", true)).toEqual(ALL_CHOICES);
    expect(fluxDisplayChoices("erg/s/cm2/A", false)).toEqual(["native", "flam_cgs", "flam_si"]);
    expect(fluxDisplayChoices("MJy", true)).toEqual(ALL_CHOICES);
    for (const unit of ["MJy/sr", "DN/s", "erg/s/cm2/A, x1e-17", "", null]) {
      expect(fluxDisplayChoices(unit, true)).toEqual(["native"]);
    }
  });

  it("labels the choices", () => {
    expect(fluxChoiceLabel("native", "Jy")).toBe("file unit (Jy)");
    expect(fluxChoiceLabel("native", null)).toBe("file unit (native)");
    expect(fluxChoiceLabel("flam_cgs", "Jy")).toBe("erg s-1 cm-2 Angstrom-1");
    expect(fluxChoiceLabel("ab", "Jy")).toBe("AB mag");
  });

  it("leaves the file's numbers alone for the native choice", () => {
    const input: FluxDisplayInput = { flux: [1, NaN], err: [0.1, 0.2], vacuumUm: null, fluxUnit: "Jy" };
    const shown = fluxDisplay(input, "native");
    expect(shown).toEqual({
      choice: "native",
      values: [1, NaN],
      errors: [0.1, 0.2],
      unit: "Jy",
      scale: 1,
      invertY: false,
      basis: null,
      reason: null,
    });
    expect(shown.values).not.toBe(input.flux);
    expect(shown.errors).not.toBe(input.err);
    expect(fluxDisplay({ ...input, err: null }, "native").errors).toBeNull();
    const empty = fluxDisplay({ flux: [], err: null, vacuumUm: null, fluxUnit: "Jy" }, "native");
    expect(empty.values).toEqual([]);
    expect(empty.scale).toBe(1);
  });

  it("scales Jy to mJy and uJy by one factor", () => {
    const mjy = fluxDisplay(NIRSPEC_JY, "mJy");
    expect(mjy.choice).toBe("mJy");
    expectAllClose(mjy.values, [19.4, 12.3]);
    expectAllClose(mjy.errors, [0.2, NaN]);
    expect(mjy.unit).toBe("mJy");
    expect(mjy.scale).toBe(1000);
    expect(mjy.basis).toBe("Jy x 1000");
    expect(mjy.invertY).toBe(false);
    expect(mjy.reason).toBeNull();
    const ujy = fluxDisplay(NIRSPEC_JY, "uJy");
    expectAllClose(ujy.values, [19400, 12300]);
    expect(ujy.scale).toBe(1e6);
    expect(ujy.basis).toBe("Jy x 1000000");
    const megajansky = fluxDisplay({ ...NIRSPEC_JY, fluxUnit: "MJy" }, "Jy");
    expect(megajansky.scale).toBe(1e6);
    expectAllClose(megajansky.values, [19400, 12300]);
  });

  it("converts F_nu to F_lambda at the wavelength of each sample", () => {
    const cgs = fluxDisplay(NIRSPEC_JY, "flam_cgs");
    expect(cgs.choice).toBe("flam_cgs");
    expectAllClose(cgs.values, [9.969658221342295e-16, 1.1381009979629634e-15]);
    const expectedError = (0.0002 * FLAM_CGS_PER_JY_UM2) / (2.4153 * 2.4153);
    expectAllClose(cgs.errors, [expectedError, NaN]);
    expectClose(expectedError, 1.0278e-17, 1e-4);
    expect(cgs.unit).toBe("erg s-1 cm-2 Angstrom-1");
    expect(cgs.scale).toBeNull();
    expect(cgs.basis).toBe(FNU_TO_FLAMBDA_BASIS);
    expect(cgs.invertY).toBe(false);
    const si = fluxDisplay(NIRSPEC_JY, "flam_si");
    expectAllClose(si.values, [9.969658221342295e-15, 1.1381009979629634e-14]);
    expect(si.unit).toBe("W m-2 um-1");
    expect(si.scale).toBeNull();
    expect(si.basis).toBe(FNU_TO_FLAMBDA_BASIS);
    const anchor: FluxDisplayInput = { flux: [1], err: null, vacuumUm: [1], fluxUnit: "Jy" };
    expectAllClose(fluxDisplay(anchor, "flam_cgs").values, [2.99792458e-13]);
    expectAllClose(fluxDisplay(anchor, "flam_si").values, [2.99792458e-12]);
  });

  it("converts F_lambda to F_nu and to AB", () => {
    const jy = fluxDisplay(HALPHA_FLAM, "Jy");
    expectAllClose(jy.values, [1.436759593198305e-5, 0.0002163027646946341]);
    expect(jy.errors).toBeNull();
    expect(jy.scale).toBeNull();
    expect(jy.basis).toBe(FLAMBDA_TO_FNU_BASIS);
    expectAllClose(fluxDisplay(HALPHA_FLAM, "uJy").values, [14.36759593198305, 216.30276469463408]);
    const si = fluxDisplay(HALPHA_FLAM, "flam_si");
    expectAllClose(si.values, [1e-16, 1.5e-15]);
    expect(si.scale).toBe(10);
    expect(si.basis).toBe("erg/s/cm2/A x 10");
    const ab = fluxDisplay(HALPHA_FLAM, "ab");
    expectAllClose(ab.values, [21.0065397360352, 18.06234482387164]);
    expect(ab.invertY).toBe(true);
    expect(ab.scale).toBeNull();
    expect(ab.unit).toBe("AB mag");
    expect(ab.basis).toBe(`${FLAMBDA_TO_FNU_BASIS}; ${AB_BASIS}`);
  });

  it("gives AB magnitudes with a magnitude error and no value at or below zero", () => {
    const ab = fluxDisplay(
      { flux: [0.0194, 0, -1, NaN], err: [0.0002, 0.1, 0.1, 0.1], vacuumUm: null, fluxUnit: "Jy" },
      "ab",
    );
    expect(ab.choice).toBe("ab");
    expectAllClose(ab.values, [13.180495675174434, NaN, NaN, NaN]);
    expectAllClose(ab.errors, [0.011193156750083803, NaN, NaN, NaN]);
    expectClose((ab.errors ?? [])[0], (MAG_ERR_PER_RELATIVE_FLUX_ERR * 0.0002) / 0.0194);
    expect(ab.invertY).toBe(true);
    expect(ab.scale).toBeNull();
    expect(ab.basis).toBe(AB_BASIS);
  });

  it("falls back to the file unit and says why", () => {
    const noAxis = fluxDisplay({ flux: [1], err: null, vacuumUm: null, fluxUnit: "Jy" }, "flam_cgs");
    expect(noAxis).toEqual({
      choice: "native",
      values: [1],
      errors: null,
      unit: "Jy",
      scale: 1,
      invertY: false,
      basis: null,
      reason: "erg s-1 cm-2 Angstrom-1 is not available for flux in Jy without a wavelength axis",
    });
    const rate = fluxDisplay({ flux: [1], err: null, vacuumUm: [1], fluxUnit: "DN/s" }, "Jy");
    expect(rate.choice).toBe("native");
    expect(rate.unit).toBe("DN/s");
    expect(rate.reason).toBe("Jy is not available for flux in DN/s");
    const mismatched = fluxDisplay({ flux: [1], err: null, vacuumUm: [1, 2], fluxUnit: "Jy" }, "flam_cgs");
    expect(mismatched.choice).toBe("native");
    expect(mismatched.values).toEqual([1]);
    expect(mismatched.scale).toBe(1);
    expect(mismatched.reason).toBe("flux and wavelength arrays differ in length");
  });

  it("titles the y axis", () => {
    expect(fluxAxisTitle({ unit: "mJy", invertY: false })).toBe("Flux (mJy)");
    expect(fluxAxisTitle({ unit: "AB mag", invertY: true })).toBe("AB magnitude");
  });

  it("agrees with convertFlux sample by sample", () => {
    const indices = Array.from({ length: 40 }, (_, i) => i);
    const input: FluxDisplayInput = {
      flux: indices.map((i) => 1 + Math.exp(-((i - 20) ** 2) / 8)),
      err: indices.map(() => 0.05),
      vacuumUm: indices.map((i) => 0.6555 + i * 1e-4),
      fluxUnit: "erg/s/cm2/A",
    };
    const from = parseSpectralFluxUnit("erg/s/cm2/A");
    const choices = fluxDisplayChoices(input.fluxUnit, true).filter((choice) => choice !== "native");
    expect(choices).toHaveLength(6);
    for (const choice of choices) {
      const direct = convertFlux(input.flux, input.err, input.vacuumUm ?? [], from, choice);
      const shown = fluxDisplay(input, choice);
      expect(direct.ok).toBe(true);
      expect(shown.choice).toBe(choice);
      expect(shown.values).toEqual(nullAsNaN(direct.values));
      expect(shown.errors).toEqual(nullAsNaN(direct.errors));
    }
  });
});
