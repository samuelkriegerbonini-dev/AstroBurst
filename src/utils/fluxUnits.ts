export const SPEED_OF_LIGHT_M_PER_S = 299792458;
export const AB_MAG_ZERO_POINT = 8.9;
export const MAG_ERR_PER_RELATIVE_FLUX_ERR = 1.085736204758129;

const FLAMBDA_CGS_PER_JY_UM2 = SPEED_OF_LIGHT_M_PER_S * 1e-21;
const FLAMBDA_SI_PER_JY_UM2 = SPEED_OF_LIGHT_M_PER_S * 1e-20;

export type SpectralFluxUnit =
  | { kind: "flux_density_nu"; jy_per_unit: number }
  | { kind: "flux_density_lambda"; cgs_per_unit: number }
  | { kind: "surface_brightness" }
  | { kind: "rate" }
  | { kind: "unknown" };

export type FluxDisplayUnit = "native" | "Jy" | "mJy" | "uJy" | "flam_cgs" | "flam_si" | "ab";

export const FLUX_DISPLAY_UNITS: readonly FluxDisplayUnit[] = ["native", "Jy", "mJy", "uJy", "flam_cgs", "flam_si", "ab"];

export const FLUX_DISPLAY_UNIT_LABEL: Readonly<Record<Exclude<FluxDisplayUnit, "native">, string>> = {
  Jy: "Jy",
  mJy: "mJy",
  uJy: "uJy",
  flam_cgs: "erg s-1 cm-2 Angstrom-1",
  flam_si: "W m-2 um-1",
  ab: "AB mag",
};

type FluxDensityUnit = Extract<SpectralFluxUnit, { kind: "flux_density_nu" | "flux_density_lambda" }>;
type JanskyDisplayUnit = "Jy" | "mJy" | "uJy";
type FlambdaDisplayUnit = "flam_cgs" | "flam_si";
type FluxConverter = (native: number, lambdaUm: number) => number;

type Base =
  | "jy"
  | "erg"
  | "s"
  | "cm"
  | "m"
  | "angstrom"
  | "um"
  | "nm"
  | "hz"
  | "w"
  | "count"
  | "sr"
  | "arcsec"
  | "pix"
  | "beam";
type Signature = Readonly<Partial<Record<Base, number>>>;
type Group = "closed" | "multiplying" | "dividing";

const MICRO_SIGN = String.fromCharCode(0xb5);
const GREEK_MU = String.fromCharCode(0x3bc);
const A_RING_UPPER = String.fromCharCode(0xc5);
const A_RING_LOWER = String.fromCharCode(0xe5);
const MIDDLE_DOT = String.fromCharCode(0xb7);

const UNIT_LETTERS = `A-Za-z${MICRO_SIGN}${GREEK_MU}${A_RING_UPPER}${A_RING_LOWER}`;
const RE_POWER_OF_TEN = /^10 *(?:\*\*|\^) *\(?([+-]?[0-9]+)\)?/;
const RE_DECIMAL = /^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?/;
const RE_EXPONENT = /(?:\*\*|\^) *\(?([+-]?[0-9]+)\)?/g;
const RE_AFTER_SCALE = new RegExp(`^ *\\*? *(?=[(${UNIT_LETTERS}])`);
const RE_FACTOR = new RegExp(`([${UNIT_LETTERS}]+)(?:\\^?([+-]?[0-9]+))?`, "y");
const PRODUCT_SIGNS = [" ", "\t", ".", "*", MIDDLE_DOT];
const I32_MIN = -2147483648;
const I32_MAX = 2147483647;

const JY_PER_JANSKY_SPELLING: ReadonlyMap<string, number> = new Map([
  ["Jy", 1],
  ["JY", 1],
  ["jy", 1],
  ["Jansky", 1],
  ["jansky", 1],
  ["mJy", 1e-3],
  ["uJy", 1e-6],
  [`${MICRO_SIGN}Jy`, 1e-6],
  [`${GREEK_MU}Jy`, 1e-6],
  ["nJy", 1e-9],
  ["MJy", 1e6],
  ["kJy", 1e3],
]);

const BASE_OF_LOWERCASE_NAME: ReadonlyMap<string, Base> = new Map<string, Base>([
  ["erg", "erg"],
  ["ergs", "erg"],
  ["s", "s"],
  ["sec", "s"],
  ["second", "s"],
  ["cm", "cm"],
  ["m", "m"],
  ["a", "angstrom"],
  ["aa", "angstrom"],
  ["ang", "angstrom"],
  ["angstrom", "angstrom"],
  ["angstroms", "angstrom"],
  [A_RING_LOWER, "angstrom"],
  ["um", "um"],
  ["micron", "um"],
  ["microns", "um"],
  ["micrometer", "um"],
  [`${MICRO_SIGN}m`, "um"],
  [`${GREEK_MU}m`, "um"],
  ["nm", "nm"],
  ["hz", "hz"],
  ["w", "w"],
  ["watt", "w"],
  ["sr", "sr"],
  ["steradian", "sr"],
  ["arcsec", "arcsec"],
  ["arcsecond", "arcsec"],
  ["pix", "pix"],
  ["pixel", "pix"],
  ["pixels", "pix"],
  ["beam", "beam"],
  ["dn", "count"],
  ["adu", "count"],
  ["count", "count"],
  ["counts", "count"],
  ["ct", "count"],
  ["cts", "count"],
  ["electron", "count"],
  ["electrons", "count"],
]);

const AREAL_BASES: readonly Base[] = ["sr", "arcsec", "pix", "beam"];

const JY_PER_UNIT_BY_SIGNATURE: readonly (readonly [Signature, number])[] = [
  [{ jy: 1 }, 1],
  [{ erg: 1, s: -1, cm: -2, hz: -1 }, 1e23],
  [{ w: 1, m: -2, hz: -1 }, 1e26],
];
const CGS_PER_UNIT_BY_SIGNATURE: readonly (readonly [Signature, number])[] = [
  [{ erg: 1, s: -1, cm: -2, angstrom: -1 }, 1],
  [{ erg: 1, s: -1, cm: -2, um: -1 }, 1e-4],
  [{ erg: 1, s: -1, cm: -2, nm: -1 }, 0.1],
  [{ w: 1, m: -2, um: -1 }, 0.1],
  [{ w: 1, m: -2, nm: -1 }, 100],
  [{ w: 1, m: -2, angstrom: -1 }, 1000],
];
const RATE_SIGNATURE: Signature = { count: 1, s: -1 };

const JY_DISPLAY_PER_JY: Readonly<Record<JanskyDisplayUnit, number>> = { Jy: 1, mJy: 1e3, uJy: 1e6 };
const FLAMBDA_DISPLAY_PER_CGS: Readonly<Record<FlambdaDisplayUnit, number>> = { flam_cgs: 1, flam_si: 10 };
const FLAMBDA_DISPLAY_OF_FNU_JY: Readonly<Record<FlambdaDisplayUnit, FluxConverter>> = {
  flam_cgs: fnuJyToFlambdaCgs,
  flam_si: fnuJyToFlambdaSiUm,
};
const REFUSAL_BY_KIND: Readonly<Record<Exclude<SpectralFluxUnit["kind"], FluxDensityUnit["kind"]>, string>> = {
  surface_brightness: "surface brightness has no flux density without a solid angle",
  rate: "a count rate has no physical flux scale",
  unknown: "the flux unit is not recognised",
};

function splitLeadingScale(text: string): { scale: number; unit: string } {
  const unscaled = { scale: 1, unit: text };
  const powerOfTen = RE_POWER_OF_TEN.exec(text);
  const number = powerOfTen ?? RE_DECIMAL.exec(text);
  if (number === null) return unscaled;
  const afterNumber = text.slice(number[0].length);
  const separator = RE_AFTER_SCALE.exec(afterNumber);
  if (separator === null) return unscaled;
  const digits = powerOfTen ? `1e${powerOfTen[1]}` : number[0];
  return { scale: Number(digits), unit: afterNumber.slice(separator[0].length) };
}

function scanFactors(unit: string): { scale: number; powers: Map<Base, number> } | null {
  let scale = 1;
  const powers = new Map<Base, number>();
  let divideNext = false;
  let group: Group = "closed";
  let at = 0;
  while (at < unit.length) {
    const c = unit[at];
    if (PRODUCT_SIGNS.includes(c)) {
      at += 1;
    } else if (c === "/") {
      if (divideNext) return null;
      divideNext = true;
      at += 1;
    } else if (c === "(") {
      if (group !== "closed") return null;
      group = divideNext ? "dividing" : "multiplying";
      divideNext = false;
      at += 1;
    } else if (c === ")") {
      if (group === "closed" || divideNext) return null;
      group = "closed";
      at += 1;
    } else {
      RE_FACTOR.lastIndex = at;
      const factor = RE_FACTOR.exec(unit);
      if (factor === null) return null;
      at = RE_FACTOR.lastIndex;
      const name = factor[1];
      const exponent: string | undefined = factor[2];
      if (name === "x" && exponent === undefined) continue;
      let power = exponent === undefined ? 1 : Number(exponent);
      if (power < I32_MIN || power > I32_MAX) return null;
      if (group === "dividing") power = -power;
      if (divideNext) {
        power = -power;
        divideNext = false;
      }
      const jyPerName = JY_PER_JANSKY_SPELLING.get(name);
      const base = jyPerName === undefined ? BASE_OF_LOWERCASE_NAME.get(name.toLowerCase()) : "jy";
      if (base === undefined) return null;
      scale *= (jyPerName ?? 1) ** power;
      powers.set(base, (powers.get(base) ?? 0) + power);
    }
  }
  return group === "closed" && !divideNext ? { scale, powers } : null;
}

function hasSignature(powers: ReadonlyMap<Base, number>, signature: Signature): boolean {
  const expected = Object.entries(signature);
  return powers.size === expected.length && expected.every(([base, power]) => powers.get(base as Base) === power);
}

function factorOf(powers: ReadonlyMap<Base, number>, table: readonly (readonly [Signature, number])[]): number | null {
  return table.find(([signature]) => hasSignature(powers, signature))?.[1] ?? null;
}

function fluxDensityOf(powers: ReadonlyMap<Base, number>, scale: number): FluxDensityUnit | null {
  const jyPerUnit = factorOf(powers, JY_PER_UNIT_BY_SIGNATURE);
  if (jyPerUnit !== null) return { kind: "flux_density_nu", jy_per_unit: scale * jyPerUnit };
  const cgsPerUnit = factorOf(powers, CGS_PER_UNIT_BY_SIGNATURE);
  if (cgsPerUnit !== null) return { kind: "flux_density_lambda", cgs_per_unit: scale * cgsPerUnit };
  return null;
}

export function parseSpectralFluxUnit(text: string): SpectralFluxUnit {
  const leading = splitLeadingScale(text.trim());
  const scanned = scanFactors(leading.unit.replace(RE_EXPONENT, "^$1"));
  if (scanned === null) return { kind: "unknown" };
  const present = [...scanned.powers].filter(([, power]) => power !== 0);
  const areal = present.some(([base]) => AREAL_BASES.includes(base));
  const rest = new Map(present.filter(([base]) => !AREAL_BASES.includes(base)));
  const fluxDensity = fluxDensityOf(rest, leading.scale * scanned.scale);
  if (fluxDensity !== null) return areal ? { kind: "surface_brightness" } : fluxDensity;
  if (!areal && hasSignature(rest, RATE_SIGNATURE)) return { kind: "rate" };
  return { kind: "unknown" };
}

function isPositiveFinite(value: number): boolean {
  return Number.isFinite(value) && value > 0;
}

export function fnuJyToFlambdaCgs(fnuJy: number, lambdaUm: number): number {
  if (!isPositiveFinite(lambdaUm)) return NaN;
  return (fnuJy * FLAMBDA_CGS_PER_JY_UM2) / (lambdaUm * lambdaUm);
}

export function flambdaCgsToFnuJy(flambda: number, lambdaUm: number): number {
  if (!isPositiveFinite(lambdaUm)) return NaN;
  return (flambda * (lambdaUm * lambdaUm)) / FLAMBDA_CGS_PER_JY_UM2;
}

export function fnuJyToFlambdaSiUm(fnuJy: number, lambdaUm: number): number {
  if (!isPositiveFinite(lambdaUm)) return NaN;
  return (fnuJy * FLAMBDA_SI_PER_JY_UM2) / (lambdaUm * lambdaUm);
}

export function fnuJyToAb(fnuJy: number): number | null {
  return isPositiveFinite(fnuJy) ? -2.5 * Math.log10(fnuJy) + AB_MAG_ZERO_POINT : null;
}

export function abToFnuJy(mag: number): number {
  return 10 ** ((AB_MAG_ZERO_POINT - mag) / 2.5);
}

export function abMagErr(fnuJy: number, fnuErrJy: number): number | null {
  if (!isPositiveFinite(fnuJy) || !Number.isFinite(fnuErrJy) || fnuErrJy < 0) return null;
  return (MAG_ERR_PER_RELATIVE_FLUX_ERR * fnuErrJy) / fnuJy;
}

function isFluxDensity(unit: SpectralFluxUnit): unit is FluxDensityUnit {
  return unit.kind === "flux_density_nu" || unit.kind === "flux_density_lambda";
}

function isFlambdaDisplayUnit(unit: FluxDisplayUnit): unit is FlambdaDisplayUnit {
  return unit === "flam_cgs" || unit === "flam_si";
}

export function availableFluxDisplayUnits(from: SpectralFluxUnit): FluxDisplayUnit[] {
  return isFluxDensity(from) ? [...FLUX_DISPLAY_UNITS] : ["native"];
}

export interface ConvertedFlux {
  ok: boolean;
  values: (number | null)[];
  errors: (number | null)[] | null;
  unit: string | null;
  reason: string | null;
}

function finiteOrNull(cell: number | null): number | null {
  return cell !== null && Number.isFinite(cell) ? cell : null;
}

function janskyConverter(from: FluxDensityUnit): FluxConverter {
  if (from.kind === "flux_density_nu") return (native) => native * from.jy_per_unit;
  return (native, lambdaUm) => flambdaCgsToFnuJy(native * from.cgs_per_unit, lambdaUm);
}

function flambdaConverter(from: FluxDensityUnit, to: FlambdaDisplayUnit): FluxConverter {
  if (from.kind === "flux_density_lambda") return (native) => native * from.cgs_per_unit * FLAMBDA_DISPLAY_PER_CGS[to];
  return (native, lambdaUm) => FLAMBDA_DISPLAY_OF_FNU_JY[to](native * from.jy_per_unit, lambdaUm);
}

export function convertFlux(
  values: readonly (number | null)[],
  errors: readonly (number | null)[] | null,
  wavelengthUm: readonly (number | null)[],
  from: SpectralFluxUnit,
  to: FluxDisplayUnit,
): ConvertedFlux {
  const nativeValues = values.map(finiteOrNull);
  const nativeErrors = errors === null ? null : errors.map(finiteOrNull);
  const unconverted = { values: nativeValues, errors: nativeErrors, unit: null };
  if (to === "native") return { ok: true, ...unconverted, reason: null };
  if (nativeErrors !== null && nativeErrors.length !== nativeValues.length) {
    return { ok: false, ...unconverted, reason: "flux and error arrays differ in length" };
  }
  if (!isFluxDensity(from)) return { ok: false, ...unconverted, reason: REFUSAL_BY_KIND[from.kind] };
  const crossesFamilies = (from.kind === "flux_density_nu") === isFlambdaDisplayUnit(to);
  if (crossesFamilies && wavelengthUm.length !== nativeValues.length) {
    return { ok: false, ...unconverted, reason: "flux and wavelength arrays differ in length" };
  }
  const lambdaAt = (i: number): number => wavelengthUm[i] ?? NaN;
  const toJy = janskyConverter(from);
  const converted = { ok: true, unit: FLUX_DISPLAY_UNIT_LABEL[to], reason: null };
  if (to === "ab") {
    return {
      ...converted,
      values: nativeValues.map((value, i) => (value === null ? null : fnuJyToAb(toJy(value, lambdaAt(i))))),
      errors:
        nativeErrors?.map((error, i) => {
          const value = nativeValues[i];
          if (value === null || error === null) return null;
          return finiteOrNull(abMagErr(toJy(value, lambdaAt(i)), toJy(error, lambdaAt(i))));
        }) ?? null,
    };
  }
  const convert: FluxConverter = isFlambdaDisplayUnit(to)
    ? flambdaConverter(from, to)
    : (native, lambdaUm) => toJy(native, lambdaUm) * JY_DISPLAY_PER_JY[to];
  return {
    ...converted,
    values: nativeValues.map((value, i) => (value === null ? null : finiteOrNull(convert(value, lambdaAt(i))))),
    errors:
      nativeErrors?.map((error, i) => (error === null ? null : finiteOrNull(Math.abs(convert(error, lambdaAt(i)))))) ??
      null,
  };
}
