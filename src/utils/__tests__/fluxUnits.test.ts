import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  AB_MAG_ZERO_POINT,
  FLUX_DISPLAY_UNITS,
  FLUX_DISPLAY_UNIT_LABEL,
  MAG_ERR_PER_RELATIVE_FLUX_ERR,
  SPEED_OF_LIGHT_M_PER_S,
  abMagErr,
  abToFnuJy,
  availableFluxDisplayUnits,
  convertFlux,
  flambdaCgsToFnuJy,
  fnuJyToAb,
  fnuJyToFlambdaCgs,
  fnuJyToFlambdaSiUm,
  parseSpectralFluxUnit,
  type SpectralFluxUnit,
} from "../fluxUnits";

interface ConversionRow {
  lambda_um: number;
  fnu_jy: number;
  flambda_cgs: number;
  flambda_si_um: number;
  ab_mag: number;
}

interface SpellingRow {
  text: string;
  kind: SpectralFluxUnit["kind"];
  jy_per_unit?: number;
  cgs_per_unit?: number;
}

interface Golden {
  tolerance: { relative: number; ab_mag_absolute: number };
  conversions: ConversionRow[];
  astropy_spellings: SpellingRow[];
  defined_spellings: SpellingRow[];
}

const GOLDEN_PATH = fileURLToPath(
  new URL("../../../src-tauri/tests/fixtures/flux_units/astropy_flux_units.json", import.meta.url),
);
const GOLDEN_SHA256 = "03fb71210bd84141785faf34c2195087b6f3693fdd01705790d93673e0d225f2";
const GOLDEN_TEXT = readFileSync(GOLDEN_PATH, "utf8");
const GOLDEN = JSON.parse(GOLDEN_TEXT) as Golden;
const RELATIVE_TOLERANCE = GOLDEN.tolerance.relative;
const AB_MAG_TOLERANCE = GOLDEN.tolerance.ab_mag_absolute;

const MILLIJANSKY: SpectralFluxUnit = { kind: "flux_density_nu", jy_per_unit: 1e-3 };
const FLAMBDA_1E17: SpectralFluxUnit = { kind: "flux_density_lambda", cgs_per_unit: 1e-17 };
const UNCONVERTIBLE: SpectralFluxUnit[] = [{ kind: "surface_brightness" }, { kind: "rate" }, { kind: "unknown" }];
const VALUES = [1, 2, null, NaN];
const ERRORS = [0.1, null, 0.2, 0.3];
const FLAMBDA_CGS_OF_1_MJY_AT_2_UM = 7.49481145e-17;

function rel(a: number, b: number): number {
  return Math.abs(a - b) / Math.abs(b);
}

function factorOf(unit: SpectralFluxUnit): number | null {
  if (unit.kind === "flux_density_nu") return unit.jy_per_unit;
  if (unit.kind === "flux_density_lambda") return unit.cgs_per_unit;
  return null;
}

function spellingsParsedDifferently(rows: SpellingRow[]): string[] {
  return rows
    .filter((row) => {
      const parsed = parseSpectralFluxUnit(row.text);
      const factor = factorOf(parsed);
      const wanted = row.jy_per_unit ?? row.cgs_per_unit ?? null;
      const sameFactor =
        factor === null || wanted === null ? factor === wanted : rel(factor, wanted) <= RELATIVE_TOLERANCE;
      return parsed.kind !== row.kind || !sameFactor;
    })
    .map((row) => `${JSON.stringify(row.text)}: parsed ${JSON.stringify(parseSpectralFluxUnit(row.text))}`);
}

function expectCells(
  got: (number | null)[] | null,
  wanted: (number | null)[],
  tolerance: (a: number, b: number) => boolean,
): void {
  expect(got).not.toBeNull();
  expect(got!.length).toBe(wanted.length);
  wanted.forEach((cell, i) => {
    const actual = got![i];
    if (cell === null) {
      expect(actual, `cell ${i}`).toBeNull();
      return;
    }
    expect(actual, `cell ${i}`).not.toBeNull();
    expect(tolerance(actual!, cell), `cell ${i}: ${actual} against ${cell}`).toBe(true);
  });
}

function withinRelative(a: number, b: number): boolean {
  return rel(a, b) <= RELATIVE_TOLERANCE;
}

function withinMagnitude(a: number, b: number): boolean {
  return Math.abs(a - b) <= AB_MAG_TOLERANCE;
}

describe("fluxUnits", () => {
  it("reads the golden table the Rust suite reads", () => {
    const hash = createHash("sha256").update(GOLDEN_TEXT.replaceAll("\r\n", "\n"), "utf8").digest("hex");
    expect(hash).toBe(GOLDEN_SHA256);
    expect(GOLDEN.conversions.length).toBe(63);
    expect(GOLDEN.astropy_spellings.length).toBe(22);
    expect(GOLDEN.defined_spellings.length).toBe(62);
  });

  it("converts F_nu to F_lambda and back like astropy spectral_density", () => {
    expect(SPEED_OF_LIGHT_M_PER_S).toBe(299792458);
    for (const row of GOLDEN.conversions) {
      const at = JSON.stringify(row);
      expect(rel(fnuJyToFlambdaCgs(row.fnu_jy, row.lambda_um), row.flambda_cgs), `cgs of ${at}`).toBeLessThanOrEqual(
        RELATIVE_TOLERANCE,
      );
      expect(
        rel(fnuJyToFlambdaSiUm(row.fnu_jy, row.lambda_um), row.flambda_si_um),
        `SI of ${at}`,
      ).toBeLessThanOrEqual(RELATIVE_TOLERANCE);
      expect(rel(flambdaCgsToFnuJy(row.flambda_cgs, row.lambda_um), row.fnu_jy), `Jy of ${at}`).toBeLessThanOrEqual(
        RELATIVE_TOLERANCE,
      );
    }
  });

  it("converts Jy to AB and back like astropy ABmag", () => {
    expect(AB_MAG_ZERO_POINT).toBe(8.9);
    for (const row of GOLDEN.conversions) {
      const at = JSON.stringify(row);
      expect(Math.abs(fnuJyToAb(row.fnu_jy)! - row.ab_mag), `magnitude of ${at}`).toBeLessThanOrEqual(AB_MAG_TOLERANCE);
      expect(rel(abToFnuJy(row.ab_mag), row.fnu_jy), `Jy of ${at}`).toBeLessThanOrEqual(RELATIVE_TOLERANCE);
    }
  });

  it("parses the spellings astropy reads to the same kind and factor", () => {
    expect(spellingsParsedDifferently(GOLDEN.astropy_spellings)).toEqual([]);
  });

  it("parses the spellings defined by the project as listed", () => {
    expect(spellingsParsedDifferently(GOLDEN.defined_spellings)).toEqual([]);
    expect(parseSpectralFluxUnit("constructor")).toEqual({ kind: "unknown" });
    expect(parseSpectralFluxUnit("erg/s/cm2/toString")).toEqual({ kind: "unknown" });
  });

  it("refuses magnitudes and wavelengths that have no answer", () => {
    for (const fnuJy of [0, -1, NaN, Infinity]) {
      expect(fnuJyToAb(fnuJy), `magnitude of ${fnuJy} Jy`).toBeNull();
    }
    for (const lambdaUm of [0, -1, NaN]) {
      expect(fnuJyToFlambdaCgs(1, lambdaUm), `cgs at ${lambdaUm} um`).toBeNaN();
      expect(flambdaCgsToFnuJy(1, lambdaUm), `Jy at ${lambdaUm} um`).toBeNaN();
      expect(fnuJyToFlambdaSiUm(1, lambdaUm), `SI at ${lambdaUm} um`).toBeNaN();
    }
    expect(MAG_ERR_PER_RELATIVE_FLUX_ERR).toBe(1.085736204758129);
    expect(abMagErr(1, 0.01)).toBe(MAG_ERR_PER_RELATIVE_FLUX_ERR * 0.01);
    expect(abMagErr(0, 0.01)).toBeNull();
    expect(abMagErr(1, -0.1)).toBeNull();
    expect(abMagErr(1, NaN)).toBeNull();
    expect(abMagErr(2, 0)).toBe(0);
  });

  it("convertFlux scales F_nu inside its family and keeps missing cells missing", () => {
    const wavelengthUm = [2, 2, 2, 2];
    const jy = convertFlux(VALUES, ERRORS, wavelengthUm, MILLIJANSKY, "Jy");
    expect(jy.ok).toBe(true);
    expect(jy.unit).toBe("Jy");
    expect(jy.reason).toBeNull();
    expectCells(jy.values, [0.001, 0.002, null, null], withinRelative);
    expectCells(jy.errors, [0.0001, null, 0.0002, 0.0003], withinRelative);

    const uJy = convertFlux(VALUES, ERRORS, wavelengthUm, MILLIJANSKY, "uJy");
    expect(uJy.ok).toBe(true);
    expect(uJy.unit).toBe("uJy");
    expectCells(uJy.values, [1000, 2000, null, null], withinRelative);

    const native = convertFlux(VALUES, ERRORS, wavelengthUm, MILLIJANSKY, "native");
    expect(native.ok).toBe(true);
    expect(native.values).toEqual([1, 2, null, null]);
    expect(native.errors).toEqual([0.1, null, 0.2, 0.3]);
    expect(native.unit).toBeNull();
    expect(native.reason).toBeNull();
    expect(native.values).not.toBe(VALUES);
    expect(native.errors).not.toBe(ERRORS);
  });

  it("convertFlux crosses to F_lambda only where a wavelength exists", () => {
    const wavelengthUm = [2, null, 2, 2];
    const cgs = convertFlux(VALUES, ERRORS, wavelengthUm, MILLIJANSKY, "flam_cgs");
    expect(cgs.ok).toBe(true);
    expect(cgs.unit).toBe("erg s-1 cm-2 Angstrom-1");
    expectCells(cgs.values, [FLAMBDA_CGS_OF_1_MJY_AT_2_UM, null, null, null], withinRelative);
    expectCells(
      cgs.errors,
      [7.49481145e-18, null, 2 * 7.49481145e-18, 3 * 7.49481145e-18],
      withinRelative,
    );

    const si = convertFlux(VALUES, ERRORS, wavelengthUm, MILLIJANSKY, "flam_si");
    expect(si.ok).toBe(true);
    expect(si.unit).toBe("W m-2 um-1");
    expectCells(si.values, [7.49481145e-16, null, null, null], withinRelative);

    const noWavelength = convertFlux([1, 1, 1], [0.1, 0.1, 0.1], [0, -1, Infinity], MILLIJANSKY, "flam_cgs");
    expect(noWavelength.ok).toBe(true);
    expect(noWavelength.values).toEqual([null, null, null]);
    expect(noWavelength.errors).toEqual([null, null, null]);
  });

  it("convertFlux gives AB magnitudes with the magnitude error", () => {
    const ab = convertFlux(VALUES, ERRORS, [2, 2, 2, 2], MILLIJANSKY, "ab");
    expect(ab.ok).toBe(true);
    expect(ab.unit).toBe("AB mag");
    expectCells(ab.values, [16.4, 15.647425010840047, null, null], withinMagnitude);
    expectCells(ab.errors, [0.1085736204758129, null, null, null], withinMagnitude);

    const negative = convertFlux([-1], [0.1], [2], MILLIJANSKY, "ab");
    expect(negative.ok).toBe(true);
    expect(negative.values).toEqual([null]);
    expect(negative.errors).toEqual([null]);
  });

  it("convertFlux converts F_lambda to SI, to Jy and to AB", () => {
    const convert = (to: Parameters<typeof convertFlux>[4]) => convertFlux([2], null, [0.5], FLAMBDA_1E17, to);
    const cgs = convert("flam_cgs");
    expectCells(cgs.values, [2e-17], withinRelative);
    expect(cgs.errors).toBeNull();
    expect(cgs.unit).toBe("erg s-1 cm-2 Angstrom-1");

    const si = convert("flam_si");
    expectCells(si.values, [2e-16], withinRelative);
    expect(si.errors).toBeNull();

    const jy = convert("Jy");
    expectCells(jy.values, [1.6678204759907606e-5], withinRelative);
    expect(jy.errors).toBeNull();

    const uJy = convert("uJy");
    expectCells(uJy.values, [16.678204759907604], withinRelative);
    expect(uJy.errors).toBeNull();

    const ab = convert("ab");
    expectCells(ab.values, [20.844626746479772], withinMagnitude);
    expect(ab.errors).toBeNull();
    expect(ab.unit).toBe("AB mag");
  });

  it("convertFlux says why a unit cannot be converted", () => {
    const reasons = [
      "surface brightness has no flux density without a solid angle",
      "a count rate has no physical flux scale",
      "the flux unit is not recognised",
    ];
    UNCONVERTIBLE.forEach((from, i) => {
      const refused = convertFlux(VALUES, ERRORS, [2, 2, 2, 2], from, "Jy");
      expect(refused.ok).toBe(false);
      expect(refused.reason).toBe(reasons[i]);
      expect(refused.values).toEqual([1, 2, null, null]);
      expect(refused.errors).toEqual([0.1, null, 0.2, 0.3]);
      expect(refused.unit).toBeNull();

      const native = convertFlux(VALUES, ERRORS, [2, 2, 2, 2], from, "native");
      expect(native.ok).toBe(true);
      expect(native.reason).toBeNull();
      expect(native.values).toEqual([1, 2, null, null]);
    });

    const shortWavelength = convertFlux(VALUES, ERRORS, [2, 2], MILLIJANSKY, "flam_cgs");
    expect(shortWavelength.ok).toBe(false);
    expect(shortWavelength.reason).toBe("flux and wavelength arrays differ in length");
    expect(shortWavelength.values).toEqual([1, 2, null, null]);
    expect(shortWavelength.errors).toEqual([0.1, null, 0.2, 0.3]);
    expect(shortWavelength.unit).toBeNull();

    const sameFamily = convertFlux(VALUES, ERRORS, [2, 2], MILLIJANSKY, "mJy");
    expect(sameFamily.ok).toBe(true);
    expect(sameFamily.unit).toBe("mJy");
    expectCells(sameFamily.values, [1, 2, null, null], withinRelative);

    const shortErrors = convertFlux(VALUES, [0.1, 0.2], [2, 2, 2, 2], MILLIJANSKY, "Jy");
    expect(shortErrors.ok).toBe(false);
    expect(shortErrors.reason).toBe("flux and error arrays differ in length");
    expect(shortErrors.values).toEqual([1, 2, null, null]);
    expect(shortErrors.errors).toEqual([0.1, 0.2]);
    expect(shortErrors.unit).toBeNull();
  });

  it("lists the display units a unit can reach and their labels", () => {
    expect(FLUX_DISPLAY_UNITS).toEqual(["native", "Jy", "mJy", "uJy", "flam_cgs", "flam_si", "ab"]);
    expect(availableFluxDisplayUnits(MILLIJANSKY)).toEqual(FLUX_DISPLAY_UNITS);
    expect(availableFluxDisplayUnits(FLAMBDA_1E17)).toEqual(FLUX_DISPLAY_UNITS);
    for (const from of UNCONVERTIBLE) {
      expect(availableFluxDisplayUnits(from)).toEqual(["native"]);
    }
    expect(FLUX_DISPLAY_UNIT_LABEL).toEqual({
      Jy: "Jy",
      mJy: "mJy",
      uJy: "uJy",
      flam_cgs: "erg s-1 cm-2 Angstrom-1",
      flam_si: "W m-2 um-1",
      ab: "AB mag",
    });
  });
});
