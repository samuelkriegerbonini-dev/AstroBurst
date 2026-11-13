import { describe, expect, it } from "vitest";
import type { TsoExposureCompleteness, TsoSeriesSummary } from "../../shared/types/tso";
import {
  completenessText,
  exposureStem,
  layoutText,
  levelText,
  seriesFactsText,
  seriesOptionText,
  setPaths,
  siblingSegments,
  type LoadedSeriesFile,
} from "../tsoSet";

const EXPOSURE = "jw02327001001_03101_00001";
const LEVEL3_NAME = "jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits";

function loaded(name: string): LoadedSeriesFile {
  return { path: `C:/d/${name}`, name };
}

function e1Series(extra: Partial<TsoSeriesSummary> = {}): TsoSeriesSummary {
  return {
    key: { detector: "NRS1", sporder: 0 },
    label: "NRS1",
    n_integrations: 240,
    n_channels: 431,
    flux_unit: "Jy",
    time_scale: "bmjd_tdb",
    time_source: "TDB-MID column",
    time_first: 60061.4421733457,
    time_last: 60061.52706665472,
    wavelength_min_um: 0.5530469695008425,
    wavelength_max_um: 5.370352185327354,
    common_grid: true,
    outlier_detection: "COMPLETE",
    completeness: [{ exposure: "jw02358-o030_t003_nirspec_clear-prism-s1600a1-sub2048", nints: 240, found: 240, missing: [], complete: true }],
    notes: [],
    warnings: [],
    ...extra,
  };
}

function exposure(extra: Partial<TsoExposureCompleteness> = {}): TsoExposureCompleteness {
  return { exposure: "e", nints: 44, found: 44, missing: [], complete: true, ...extra };
}

describe("tsoSet", () => {
  it("finds the exposure a segment file belongs to", () => {
    expect(exposureStem(`${EXPOSURE}-seg003_nrs1_x1dints.fits`)).toBe(EXPOSURE);
    expect(exposureStem("jw02327-o001_t001_nirspec_f290lp-g395m-s1600a1-sub2048_x1dints.fits")).toBeNull();
    expect(exposureStem("a-SEG12_x.fits")).toBe("a");
    expect(exposureStem("a-segment.fits")).toBeNull();
    expect(exposureStem("a-segment-seg2_x.fits")).toBe("a-segment");
    expect(exposureStem("a-seg1-seg2_x.fits")).toBe("a");
    expect(exposureStem("")).toBeNull();
  });

  it("lists the other segments of the same exposure and nothing for a level 3 file", () => {
    const seg1Nrs1 = loaded(`${EXPOSURE}-seg001_nrs1_x1dints.fits`);
    const seg2Nrs1 = loaded(`${EXPOSURE}-seg002_nrs1_x1dints.fits`);
    const seg1Nrs2 = loaded(`${EXPOSURE}-seg001_nrs2_x1dints.fits`);
    const other = loaded("jw02327001001_04101_00001-seg001_nrs1_x1dints.fits");
    const level3 = loaded(LEVEL3_NAME);
    const files = [seg1Nrs1, other, seg2Nrs1, level3, seg1Nrs2];

    expect(siblingSegments(seg1Nrs1.path, seg1Nrs1.name, files)).toEqual([seg2Nrs1, seg1Nrs2]);
    expect(siblingSegments(level3.path, level3.name, files)).toEqual([]);
    expect(siblingSegments(other.path, other.name, files)).toEqual([]);
    expect(siblingSegments("C:/elsewhere/x.fits", seg2Nrs1.name, files)).toEqual([seg1Nrs1, seg2Nrs1, seg1Nrs2]);
    expect(siblingSegments(seg1Nrs1.path, seg1Nrs1.name, [])).toEqual([]);

    expect(setPaths("a", ["b", "a", "b"])).toEqual(["a", "b"]);
    expect(setPaths("a", [])).toEqual(["a"]);
    expect(setPaths("a", ["c", "b"])).toEqual(["a", "c", "b"]);
  });

  it("words the level, the layout, a series and its completeness", () => {
    expect(levelText(3)).toBe("level 3");
    expect(levelText(2)).toBe("level 2b");
    expect(layoutText("rows")).toBe("one row per integration");
    expect(layoutText("extensions")).toBe("one table per integration");

    expect(seriesOptionText(e1Series())).toBe("NRS1 (240 integrations x 431 channels)");
    expect(seriesFactsText(e1Series())).toBe("240 integrations x 431 channels, flux in Jy, time axis BMJD (TDB) (TDB-MID column)");
    expect(seriesFactsText(e1Series({ time_scale: "mjd_utc", time_source: "MJD-AVG (UTC, not barycentric)" }))).toBe(
      "240 integrations x 431 channels, flux in Jy, time axis MJD (UTC) (MJD-AVG (UTC, not barycentric))",
    );
    expect(seriesFactsText(e1Series({ time_scale: "index", time_source: "integration number (no usable times)" }))).toBe(
      "240 integrations x 431 channels, flux in Jy, time axis integration number (integration number (no usable times))",
    );

    expect(completenessText(exposure())).toBe("e: 44 of 44 integrations");
    expect(completenessText(exposure({ found: 36, missing: [[17, 24]], complete: false }))).toBe("e: 36 of 44 integrations, missing 17 to 24");
    expect(completenessText(exposure({ found: 41, missing: [[2, 2], [5, 6]], complete: false }))).toBe(
      "e: 41 of 44 integrations, missing 2, 5 to 6",
    );
    expect(completenessText(exposure({ nints: null, found: 8, complete: null }))).toBe("e: 8 integrations (NINTS unknown)");
  });
});
