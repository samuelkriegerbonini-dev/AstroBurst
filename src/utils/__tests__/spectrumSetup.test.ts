import { describe, expect, it } from "vitest";
import type { Spectrum1d } from "../../shared/types/spectrum1d";
import { DEFAULT_SPECTRUM_VIEW, type SpectrumViewState } from "../../hooks/useSpectrumViewStore";
import {
  channelRangeForWavelengths,
  clearedSetup,
  setupCarriesTo,
  setupFromView,
  setupInstrument,
  setupIsDefault,
  setupLabel,
  viewFromSetup,
  type SpectrumSetup,
} from "../spectrumSetup";

const A = Array.from({ length: 40 }, (_, i) => 0.6555 + i * 1e-4);
const B = Array.from({ length: 40 }, (_, i) => 0.656 + i * 1e-4);

const VIEW: SpectrumViewState = {
  ...DEFAULT_SPECTRUM_VIEW,
  bandSpecies: [],
  mode: "velocity",
  restUm: 0.656461,
  redshift: 0.05,
  lineFamilies: ["optical"],
  range: { z0: 5, z1: 12 },
  windows: [
    [0, 3],
    [20, 24],
  ],
};

const CARRIED = {
  mode: "velocity",
  restUm: 0.656461,
  convention: "optical",
  redshift: 0.05,
  lineListVisible: true,
  lineFamilies: ["optical"],
  bandSpecies: [],
  pickedLineId: null,
  lineModel: "gaussian",
  wavelengthUnit: "um",
};

const FILE_FIELDS = ["frameChoice", "fluxUnit", "binFactor", "xDomain", "logY", "yLimits", "useDq"];

function spectrumAt(source: Spectrum1d["source"]): Spectrum1d {
  return {
    source,
    axis: {
      kind: "wave",
      ctype: "WAVE",
      unit: "um",
      header_unit: "um",
      header_scale: 1,
      values: A,
      crval: A[0],
      cdelt: 1e-4,
      crpix: 1,
      rest_wavelength_um: null,
      rest_frequency_hz: null,
      specsys: null,
      velosys: null,
      notes: [],
    },
    wavelength_frame: "vacuum",
    precision: "f64",
    flux: A.map(() => 1),
    err: null,
    dq: null,
    dq_table: "unknown",
    flux_unit: "Jy",
    n_rows: 1,
    cards: {},
    notes: [],
  };
}

describe("spectrum setup", () => {
  it("places a wavelength range on the channels of another axis", () => {
    expect(channelRangeForWavelengths(A, 0.6565, 0.6575)).toEqual({ z0: 10, z1: 20 });
    expect(channelRangeForWavelengths(B, 0.6565, 0.6575)).toEqual({ z0: 5, z1: 15 });
    expect(channelRangeForWavelengths([...B].reverse(), 0.6565, 0.6575)).toEqual({ z0: 24, z1: 34 });
    expect(channelRangeForWavelengths(A, 0.6575, 0.6565)).toEqual({ z0: 10, z1: 20 });
    expect(channelRangeForWavelengths(B, 0.6575, 0.6565)).toEqual({ z0: 5, z1: 15 });
    expect(channelRangeForWavelengths([...B].reverse(), 0.6575, 0.6565)).toEqual({ z0: 24, z1: 34 });
    expect(channelRangeForWavelengths(A, 0.65546, 0.6575)).toEqual({ z0: 0, z1: 20 });
  });

  it("places nothing outside the axis", () => {
    expect(channelRangeForWavelengths(A, 0.7, 0.71)).toBeNull();
    expect(channelRangeForWavelengths(A, 0.654, 0.6575)).toBeNull();
    expect(channelRangeForWavelengths(A, NaN, 0.6575)).toBeNull();
    expect(channelRangeForWavelengths([], 0.65, 0.66)).toBeNull();
    expect(channelRangeForWavelengths([NaN, NaN], 0.65, 0.66)).toBeNull();
  });

  it("stores a setup with its range in vacuum micrometres", () => {
    const setup = setupFromView("1|C:/d/a.fits", "a.fits, 1D array in HDU 1", VIEW, A);
    expect(setup).toEqual({
      fromKey: "1|C:/d/a.fits",
      fromLabel: "a.fits, 1D array in HDU 1",
      instrument: "",
      ...CARRIED,
      rangeUm: [A[5], A[12]],
      windowsUm: [
        [A[0], A[3]],
        [A[20], A[24]],
      ],
    });
    expect(setup.lineFamilies).not.toBe(VIEW.lineFamilies);
    expect(setup.bandSpecies).not.toBe(VIEW.bandSpecies);

    const noAxis = setupFromView("k", "a", VIEW, null);
    expect(noAxis.rangeUm).toBeNull();
    expect(noAxis.windowsUm).toBeNull();
    expect(noAxis.redshift).toBe(0.05);

    expect(setupFromView("k", "a", { ...VIEW, range: { z0: 5, z1: 40 } }, A).rangeUm).toBeNull();
    const oneWindowBeyond = setupFromView("k", "a", { ...VIEW, windows: [[0, 3], [20, 40]] }, A);
    expect(oneWindowBeyond.windowsUm).toBeNull();
    expect(oneWindowBeyond.rangeUm).toEqual([A[5], A[12]]);
  });

  it("rebuilds the view of another spectrum from a setup", () => {
    const setup = setupFromView("1|C:/d/a.fits", "a.fits, 1D array in HDU 1", VIEW, A);
    const onB = viewFromSetup(setup, B);
    expect(onB).toEqual({ ...CARRIED, range: { z0: 0, z1: 7 }, windows: null });
    expect(onB.lineFamilies).not.toBe(setup.lineFamilies);

    const inside: SpectrumSetup = {
      ...setup,
      windowsUm: [
        [A[6], A[8]],
        [A[20], A[24]],
      ],
    };
    expect(viewFromSetup(inside, B).windows).toEqual([
      [1, 3],
      [15, 19],
    ]);

    expect(viewFromSetup(setup, null)).toEqual({ ...CARRIED, range: null, windows: null });

    const keys = Object.keys(onB);
    expect(keys).toHaveLength(12);
    for (const field of FILE_FIELDS) expect(keys).not.toContain(field);
  });

  it("knows an untouched setup and how to clear one", () => {
    expect(setupIsDefault(DEFAULT_SPECTRUM_VIEW)).toBe(true);
    expect(setupIsDefault({ ...DEFAULT_SPECTRUM_VIEW, redshift: 0.1 })).toBe(false);
    expect(setupIsDefault({ ...DEFAULT_SPECTRUM_VIEW, lineFamilies: ["optical", "nir", "mir"] })).toBe(false);
    expect(setupIsDefault({ ...DEFAULT_SPECTRUM_VIEW, range: { z0: 1, z1: 3 } })).toBe(false);
    expect(setupIsDefault({ ...DEFAULT_SPECTRUM_VIEW, lineFamilies: ["optical", "nir", "mir", "radio"] })).toBe(true);
    expect(setupIsDefault({ ...DEFAULT_SPECTRUM_VIEW, frameChoice: "vacuum" })).toBe(true);
    expect(setupIsDefault({ ...DEFAULT_SPECTRUM_VIEW, binFactor: 4 })).toBe(true);
    expect(setupIsDefault({ ...DEFAULT_SPECTRUM_VIEW, xDomain: [0.656, 0.657] })).toBe(true);
    expect(clearedSetup()).toEqual({
      mode: null,
      restUm: null,
      convention: "optical",
      redshift: 0,
      lineListVisible: true,
      lineFamilies: ["optical", "nir", "mir", "radio"],
      bandSpecies: [],
      pickedLineId: null,
      lineModel: "gaussian",
      wavelengthUnit: "um",
      range: null,
      windows: null,
    });
    expect(clearedSetup().lineFamilies).not.toBe(DEFAULT_SPECTRUM_VIEW.lineFamilies);
  });

  it("labels a setup by file and layout", () => {
    expect(setupLabel(spectrumAt({ kind: "wcs1d", path: "C:/data/ha_bess.fits", hdu: 0 }))).toBe("ha_bess.fits, 1D array in HDU 0");
    const series = spectrumAt({
      kind: "x1dints",
      paths: ["C:/d/jw03375-o001_x1dints.fits"],
      detector: "NIS",
      sporder: 2,
      pick: { kind: "integration", index: 2 },
    });
    expect(setupLabel(series)).toBe("jw03375-o001_x1dints.fits, time-series spectra, NIS, order 2: integration 3");
  });

  it("keeps a setup to the instrument, disperser and filter it was chosen with", () => {
    const withCards = (cards: Record<string, string>) => ({ ...spectrumAt({ kind: "x1d", path: "C:/d/b_x1d.fits", hdu: 1, extver: 1 }), cards });
    const nrs1 = withCards({ INSTRUME: "NIRSPEC", GRATING: "G235H", FILTER: "F170LP", DETECTOR: "NRS1" });
    const nrs2 = withCards({ INSTRUME: "NIRSPEC", GRATING: "G235H", FILTER: "F170LP", DETECTOR: "NRS2", TARGNAME: "other" });
    const spaced = withCards({ INSTRUME: " nirspec ", GRATING: "g235h", FILTER: "F170LP" });
    const g395 = withCards({ INSTRUME: "NIRSPEC", GRATING: "G395H", FILTER: "F290LP" });
    const miri = withCards({ INSTRUME: "MIRI", FILTER: "P750L", DETECTOR: "MIRIMAGE" });
    const soss = withCards({ INSTRUME: "NIRISS", FILTER: "CLEAR", PUPIL: "GR700XD" });
    const bare = withCards({});
    expect(setupInstrument(nrs1)).toBe("INSTRUME=NIRSPEC GRATING=G235H FILTER=F170LP");
    expect(setupInstrument(soss)).toBe("INSTRUME=NIRISS FILTER=CLEAR PUPIL=GR700XD");
    expect(setupInstrument(bare)).toBe("");

    const fromNrs1 = setupFromView("k", "a", VIEW, A, setupInstrument(nrs1));
    expect(fromNrs1.instrument).toBe("INSTRUME=NIRSPEC GRATING=G235H FILTER=F170LP");
    expect(setupCarriesTo(fromNrs1, nrs1)).toBe(true);
    expect(setupCarriesTo(fromNrs1, nrs2)).toBe(true);
    expect(setupCarriesTo(fromNrs1, spaced)).toBe(true);
    expect(setupCarriesTo(fromNrs1, g395)).toBe(false);
    expect(setupCarriesTo(fromNrs1, miri)).toBe(false);
    expect(setupCarriesTo(fromNrs1, bare)).toBe(false);

    const fromBare = setupFromView("k", "a", VIEW, A);
    expect(fromBare.instrument).toBe("");
    expect(setupCarriesTo(fromBare, bare)).toBe(true);
    expect(setupCarriesTo(fromBare, nrs1)).toBe(false);
  });
});
