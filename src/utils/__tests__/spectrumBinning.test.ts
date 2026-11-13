import { describe, expect, it } from "vitest";
import {
  BINNING_BASIS,
  binAxis,
  binCount,
  binSpectrum,
  binningNote,
  effectiveBinFactor,
} from "../spectrumBinning";

const RELATIVE_TOLERANCE = 1e-12;
const NOTE_TAIL = "; mean of the unmasked samples, errors assume independent channels";

function expectClose(actual: number, expected: number): void {
  expect(Number.isFinite(actual)).toBe(true);
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(RELATIVE_TOLERANCE * Math.abs(expected));
}

function expectAllClose(actual: readonly number[] | null, expected: readonly number[]): void {
  expect(actual).not.toBeNull();
  const values = actual as readonly number[];
  expect(values.length).toBe(expected.length);
  expected.forEach((value, i) => {
    if (Number.isNaN(value)) expect(values[i]).toBeNaN();
    else expectClose(values[i], value);
  });
}

function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

const FLUX_A = [1, 2, 3, 4, 5, 6, 7, 8];
const ERR_A = [0.1, 0.2, 0.1, 0.2, 0.3, 0.1, 0.2, 0.2];
const FLUX_B = Array.from({ length: 10 }, (_, i) => i);
const ERR_B = FLUX_B.map(() => 0.5);
const FLUX_G = Array.from({ length: 2000 }, (_, i) => 1e-3 + 5e-5 * Math.sin(0.37 * i));
const ERR_G = Array.from({ length: 2000 }, (_, i) => 1e-5 * (1 + 0.5 * Math.cos(0.11 * i)));
const MASK_G = Array.from({ length: 2000 }, (_, i) => i % 13 === 0);
const FLUX_G_GAP = FLUX_G.map((value, i) => (i === 700 ? NaN : value));
const FLUX_K = Array.from({ length: 40 }, (_, i) => 1 + Math.exp(-((i - 20) ** 2) / 8));
const ERR_K = FLUX_K.map(() => 0.05);

describe("binSpectrum", () => {
  it("bins by the mean and propagates the errors in quadrature", () => {
    const binned = binSpectrum(FLUX_A, ERR_A, null, 4);
    expectAllClose(binned.flux, [2.5, 6.5]);
    expectAllClose(binned.err, [0.07905694150420949, 0.10606601717798214]);
    expect(binned.first).toEqual([0, 4]);
    expect(binned.last).toEqual([3, 7]);
    expect(binned.nUsed).toEqual([4, 4]);
    expect(binned.factor).toBe(4);
    expect(binned.channels).toBe(8);
    expect(binned.nExcluded).toBe(0);
    expect(binned.nNonFinite).toBe(0);
  });

  it("keeps a shorter last bin", () => {
    const binned = binSpectrum(FLUX_B, ERR_B, null, 4);
    expectAllClose(binned.flux, [1.5, 5.5, 8.5]);
    expectAllClose(binned.err, [0.25, 0.25, 0.3535533905932738]);
    expect(binned.nUsed).toEqual([4, 4, 2]);
    expect(binned.first).toEqual([0, 4, 8]);
    expect(binned.last).toEqual([3, 7, 9]);
  });

  it("leaves masked and non-finite samples out of the mean and of the error", () => {
    const binned = binSpectrum([1, NaN, 3, 4], [0.3, 0.1, 0.1, 0.4], [false, false, true, false], 4);
    expectAllClose(binned.flux, [2.5]);
    expectAllClose(binned.err, [0.25]);
    expect(binned.nUsed).toEqual([2]);
    expect(binned.nExcluded).toBe(1);
    expect(binned.nNonFinite).toBe(1);
  });

  it("gives no error to a bin that used a sample without one", () => {
    for (const unknown of [0, NaN, -1]) {
      const binned = binSpectrum([1, 2, 3, 4], [0.3, unknown, 0.1, 0.4], null, 2);
      expectAllClose(binned.flux, [1.5, 3.5]);
      expectAllClose(binned.err, [NaN, 0.20615528128088306]);
    }
  });

  it("gives an empty bin no value", () => {
    const err = [0.1, 0.1, 0.1, 0.1];
    const nonFinite = binSpectrum([NaN, NaN, 1, 2], err, null, 2);
    expectAllClose(nonFinite.flux, [NaN, 1.5]);
    expect(nonFinite.err?.[0]).toBeNaN();
    expect(nonFinite.nUsed).toEqual([0, 2]);
    expect(nonFinite.nNonFinite).toBe(2);
    expect(nonFinite.nExcluded).toBe(0);

    const masked = binSpectrum([5, NaN, 1, 2], err, [true, true, false, false], 2);
    expectAllClose(masked.flux, [NaN, 1.5]);
    expect(masked.err?.[0]).toBeNaN();
    expect(masked.nUsed).toEqual([0, 2]);
    expect(masked.nExcluded).toBe(2);
    expect(masked.nNonFinite).toBe(0);
  });

  it("returns the spectrum itself for a factor of one, with masked samples blanked", () => {
    const binned = binSpectrum([1, 2, 3], [0.1, 0.2, 0.3], [false, true, false], 1);
    expectAllClose(binned.flux, [1, NaN, 3]);
    expectAllClose(binned.err, [0.1, NaN, 0.3]);
    expect(binned.first).toEqual([0, 1, 2]);
    expect(binned.last).toEqual([0, 1, 2]);
    expect(binned.nUsed).toEqual([1, 0, 1]);
  });

  it("bins without errors", () => {
    const binned = binSpectrum(FLUX_A, null, null, 4);
    expect(binned.err).toBeNull();
    expectAllClose(binned.flux, [2.5, 6.5]);
  });

  it("matches numpy on two thousand samples", () => {
    const plain = binSpectrum(FLUX_G, ERR_G, null, 7);
    expect(plain.flux.length).toBe(286);
    expectClose(plain.flux[0], 0.00103346659677873);
    expectClose((plain.err as number[])[0], 5.526256402580079e-6);
    expect([plain.first[100], plain.last[100]]).toEqual([700, 706]);
    expectClose(plain.flux[100], 0.0010223790043510767);
    expectClose((plain.err as number[])[100], 3.151666235852187e-6);
    expect([plain.first[285], plain.last[285], plain.nUsed[285]]).toEqual([1995, 1999, 5]);
    expectClose(plain.flux[285], 0.000974925063593267);
    expectClose((plain.err as number[])[285], 6.6176920961510254e-6);

    const masked = binSpectrum(FLUX_G_GAP, ERR_G, MASK_G, 7);
    expectClose(masked.flux[0], 0.0010390443629085184);
    expectClose((masked.err as number[])[0], 5.942866829527199e-6);
    expect(masked.nUsed[0]).toBe(6);
    expectClose(masked.flux[100], 0.0010130147270586544);
    expectClose((masked.err as number[])[100], 3.5388352393971305e-6);
    expect(masked.nUsed[100]).toBe(5);
    expect(masked.nExcluded).toBe(154);
    expect(masked.nNonFinite).toBe(1);
    expect(sum(masked.nUsed)).toBe(1845);
  });

  it("bins the fixture spectrum of the panel tests", () => {
    const byFour = binSpectrum(FLUX_K, ERR_K, null, 4);
    expectClose(byFour.flux[4], 1.4872538282230479);
    expectClose(byFour.flux[5], 1.7034200074138945);
    expectAllClose(byFour.err, byFour.flux.map(() => 0.025));

    const byEight = binSpectrum(FLUX_K, ERR_K, null, 8);
    expectClose(byEight.flux[2], 1.595336917818471);
    expectClose((byEight.err as number[])[2], 0.01767766952966369);

    const byThree = binSpectrum(FLUX_K, ERR_K, null, 3);
    expect(byThree.flux.length).toBe(14);
    expectClose(byThree.flux[6], 1.8296758540990765);
    expectClose((byThree.err as number[])[6], 0.028867513459481294);
    expect(byThree.first[13]).toBe(39);
    expect(byThree.last[13]).toBe(39);
    expect(byThree.nUsed[13]).toBe(1);
    expectClose(byThree.flux[13], 1);
    expectClose((byThree.err as number[])[13], 0.05);
  });

  it("never throws on a factor it cannot use", () => {
    expect(effectiveBinFactor(0, 10)).toBe(1);
    expect(effectiveBinFactor(-3, 10)).toBe(1);
    expect(effectiveBinFactor(NaN, 10)).toBe(1);
    expect(effectiveBinFactor(Infinity, 10)).toBe(1);
    expect(effectiveBinFactor(2.7, 10)).toBe(2);
    expect(effectiveBinFactor(50, 10)).toBe(10);
    expect(effectiveBinFactor(4, 0)).toBe(1);

    const whole = binSpectrum([1, 2, 3], null, null, 50);
    expect(whole.flux).toEqual([2]);
    expect(whole.first).toEqual([0]);
    expect(whole.last).toEqual([2]);
    expect(whole.factor).toBe(3);

    const empty = binSpectrum([], null, null, 4);
    expect(empty.flux).toEqual([]);
    expect(empty.first).toEqual([]);
    expect(empty.last).toEqual([]);
    expect(empty.nUsed).toEqual([]);
    expect(empty.err).toBeNull();
    expect(empty.channels).toBe(0);
    expect(empty.factor).toBe(1);

    expect(binCount(10, 4)).toBe(3);
    expect(binCount(0, 4)).toBe(0);
    expect(binCount(8, 4)).toBe(2);
  });

  it("ignores an error or a mask of another length", () => {
    expect(binSpectrum([1, 2, 3, 4], [0.1, 0.1, 0.1], null, 2).err).toBeNull();
    const binned = binSpectrum([1, 2, 3, 4], null, [true, true, true], 2);
    expect(binned.nExcluded).toBe(0);
    expect(binned.nUsed).toEqual([2, 2]);
  });

  it("keeps the count identity", () => {
    const masked = binSpectrum(FLUX_G_GAP, ERR_G, MASK_G, 7);
    expect(masked.nExcluded + masked.nNonFinite + sum(masked.nUsed)).toBe(2000);
  });
});

describe("binAxis", () => {
  it("puts the bin centre at the mean of its axis values", () => {
    const axis = Array.from({ length: 10 }, (_, i) => 0.6555 + i * 1e-4);
    expectAllClose(binAxis(axis, 4), [0.65565, 0.65605, 0.65635]);
    expectAllClose(binAxis([1, NaN, 3, NaN, NaN], 2), [1, 3, NaN]);
    expect(binAxis([], 3)).toEqual([]);
  });
});

describe("binningNote", () => {
  it("says what was done in one line", () => {
    expect(binningNote(binSpectrum(FLUX_A, ERR_A, null, 4))).toBe(`binned by 4: 2 bins${NOTE_TAIL}`);
    expect(binningNote(binSpectrum(FLUX_B, ERR_B, null, 4))).toBe(
      `binned by 4: 3 bins, the last holds 2 channels${NOTE_TAIL}`,
    );
    expect(binningNote(binSpectrum(FLUX_K, ERR_K, null, 3))).toBe(
      `binned by 3: 14 bins, the last holds 1 channel${NOTE_TAIL}`,
    );
    expect(binningNote(binSpectrum([1, 2, 3], null, null, 50))).toBe(`binned by 3: 1 bin${NOTE_TAIL}`);
    expect(binningNote(binSpectrum(FLUX_A, ERR_A, null, 1))).toBe("");
  });

  it("states the independence assumption in the basis sentence", () => {
    expect(BINNING_BASIS).toBe(
      "flux = mean of the samples used in a bin; err = sqrt(sum err^2) / n_used, the channels treated as independent",
    );
  });
});
