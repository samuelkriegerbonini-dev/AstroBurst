import { describe, expect, it } from "vitest";
import { DEFAULT_TSO_DRAFT as D, type TsoCurvesDraft } from "../../hooks/useTsoStore";
import type { TsoCurve, TsoLightCurves, TsoRejectedElement } from "../../shared/types/tso";
import {
  droppedLine,
  equalEdgesText,
  excludeRangesFromText,
  limitsLine,
  rejectedLines,
  requestFromDraft,
  selectedPointLine,
} from "../tsoCurveInputs";
import { binsFromEdgesText } from "../tsoLightCurves";

const E1_RANGE: [number, number] = [0.5530469695008425, 5.370352185327354];

function curve(flux: (number | null)[], extra: Partial<TsoCurve> = {}): TsoCurve {
  return {
    band_um: [1, 2],
    limits_origin: "open",
    flux,
    flux_err: flux.map(() => 0.5),
    n_used: flux.map(() => 5),
    npixels_mean: flux.map(() => 7),
    ...extra,
  };
}

function result(extra: Partial<TsoLightCurves> = {}): TsoLightCurves {
  const time = [60000.1, 60000.2, 60000.3, 60000.4];
  return {
    set_id: "abc",
    label: "NRS1",
    series: { detector: "NRS1", sporder: 0 },
    mode: "pipeline",
    time_scale: "bmjd_tdb",
    time,
    mjd_utc: time.map((t) => t - 0.002),
    int_num: [1, 2, 3, 4],
    row_index: [0, 1, 2, 3],
    flux_unit: "Jy",
    white: curve([10, 11, 12, 13]),
    bins: [],
    dropped: { nan_time: 0, repeated_time: 0 },
    rejected_elements: [],
    notes: [],
    warnings: [],
    ...extra,
  };
}

function errorOf(patch: Partial<TsoCurvesDraft>): string | null {
  const input = requestFromDraft({ ...D, ...patch });
  expect(input.request).toBeNull();
  return input.error;
}

describe("tsoCurveInputs", () => {
  it("builds the default request of each mode", () => {
    expect(requestFromDraft(D)).toEqual({
      request: {
        mode: "pipeline",
        wavelength_min_um: null,
        wavelength_max_um: null,
        bins: [],
        exclude_um: [],
        reject_elements_sigma: null,
      },
      error: null,
    });
    const masked = requestFromDraft({ ...D, mode: "masked" });
    expect(masked.error).toBeNull();
    expect(masked.request).toMatchObject({ mode: "masked", reject_elements_sigma: 100, exclude_um: [] });
    expect(requestFromDraft({ ...D, mode: "masked", rejectOn: false }).request).toMatchObject({
      mode: "masked",
      reject_elements_sigma: null,
    });
  });

  it("reads limits, bin edges, extra bins and exclusion ranges", () => {
    const draft: TsoCurvesDraft = {
      ...D,
      mode: "masked",
      minText: "1.0",
      maxText: "5",
      binsText: "0.6, 1, 2",
      extraBins: [[4.15, 4.5]],
      excludeText: "3.72-3.82; 4.2 to 4.4",
      rejectSigmaText: "50",
    };
    expect(requestFromDraft(draft)).toEqual({
      request: {
        mode: "masked",
        wavelength_min_um: 1,
        wavelength_max_um: 5,
        bins: [
          [0.6, 1],
          [1, 2],
          [4.15, 4.5],
        ],
        exclude_um: [
          [3.72, 3.82],
          [4.2, 4.4],
        ],
        reject_elements_sigma: 50,
      },
      error: null,
    });
    const pipeline = requestFromDraft({ ...draft, mode: "pipeline" });
    expect(pipeline.error).toBeNull();
    expect(pipeline.request).toMatchObject({ mode: "pipeline", exclude_um: [], reject_elements_sigma: null });
    expect(pipeline.request?.bins).toHaveLength(3);
    expect(requestFromDraft({ ...D, minText: " 2 " }).request).toMatchObject({ wavelength_min_um: 2, wavelength_max_um: null });
    expect(excludeRangesFromText("  ")).toEqual({ ok: true, ranges: [] });
    expect(excludeRangesFromText("3.72 - 3.82, 4.2to4.4;")).toEqual({
      ok: true,
      ranges: [
        [3.72, 3.82],
        [4.2, 4.4],
      ],
    });
    expect(excludeRangesFromText("3-3")).toEqual({ ok: true, ranges: [[3, 3]] });
  });

  it("names the first thing that is wrong", () => {
    expect(errorOf({ minText: "a" })).toBe("the lower limit is not a number");
    expect(errorOf({ maxText: "1,5" })).toBe("the upper limit is not a number");
    expect(errorOf({ minText: "2", maxText: "1" })).toBe("the lower limit must be below the upper one");
    expect(errorOf({ minText: "2", maxText: "2" })).toBe("the lower limit must be below the upper one");
    expect(errorOf({ binsText: "1, x" })).toBe("bin edges: edge 'x' is not a number");
    expect(errorOf({ mode: "masked", excludeText: "3" })).toBe("exclude range '3' is not two numbers written as a-b");
    expect(errorOf({ mode: "masked", excludeText: "4-3" })).toBe("exclude range '4-3': the first number must not be above the second");
    expect(errorOf({ mode: "masked", rejectSigmaText: "0" })).toBe("the rejection threshold must be a number above 0");
    expect(errorOf({ mode: "masked", rejectSigmaText: "" })).toBe("the rejection threshold must be a number above 0");
    const edges = Array.from({ length: 511 }, (_, k) => String(1 + k * 0.01)).join(", ");
    expect(errorOf({ binsText: edges, extraBins: [[1, 2], [2, 3], [3, 4]] })).toBe("at most 512 bins");
    expect(requestFromDraft({ ...D, binsText: edges, extraBins: [[1, 2], [2, 3]] }).request?.bins).toHaveLength(512);
    expect(errorOf({ minText: "a", binsText: "1, x" })).toBe("the lower limit is not a number");
    expect(errorOf({ mode: "masked", binsText: "x", excludeText: "3" })).toBe("bin edges: edge 'x' is not a number");
    expect(requestFromDraft({ ...D, excludeText: "3", rejectSigmaText: "0" }).error).toBeNull();
    expect(requestFromDraft({ ...D, mode: "masked", rejectOn: false, rejectSigmaText: "0" }).error).toBeNull();
  });

  it("fills equal bins as edges text", () => {
    expect(equalEdgesText(0.6, 5.3, 5)).toBe("0.6, 1.54, 2.48, 3.42, 4.36, 5.3");
    expect(equalEdgesText(null, 5, 3)).toBe("");
    expect(equalEdgesText(1, null, 3)).toBe("");
    expect(equalEdgesText(1, 2, 0)).toBe("");
    const filled = binsFromEdgesText(equalEdgesText(E1_RANGE[0], E1_RANGE[1], 4));
    expect(filled.ok).toBe(true);
    expect(filled.ok && filled.bins).toHaveLength(4);
  });

  it("words the limits, the dropped integrations, the rejected elements and a selected point", () => {
    expect(limitsLine({ band_um: E1_RANGE, limits_origin: "open" })).toBe("White light summed over 0.553 to 5.37 um (open limits)");
    expect(limitsLine({ band_um: [2.47, 3.94], limits_origin: "file log" })).toBe(
      "White light summed over 2.47 to 3.94 um (limits from the processing log of the file)",
    );
    expect(limitsLine({ band_um: [2.47, 3.94], limits_origin: "built-in default" })).toBe(
      "White light summed over 2.47 to 3.94 um (built-in limits of this mode)",
    );
    expect(limitsLine({ band_um: [2.47, 3.94], limits_origin: "user" })).toBe("White light summed over 2.47 to 3.94 um (your limits)");

    expect(droppedLine({ nan_time: 0, repeated_time: 0 })).toBeNull();
    expect(droppedLine({ nan_time: 3, repeated_time: 0 })).toBe(
      "The pipeline rule left out 3 integration(s) with no MJD-AVG and 0 with a repeated MJD-AVG.",
    );
    expect(droppedLine({ nan_time: 0, repeated_time: 2 })).toBe(
      "The pipeline rule left out 0 integration(s) with no MJD-AVG and 2 with a repeated MJD-AVG.",
    );

    expect(rejectedLines([{ index: 230, wavelength_um: 3.0178602767154326, reason: "r" }])).toEqual([
      "Rejected element at 3.01786 um (channel 230)",
    ]);
    const seven: TsoRejectedElement[] = Array.from({ length: 7 }, (_, k) => ({ index: k, wavelength_um: 1 + k, reason: "r" }));
    const lines = rejectedLines(seven, 5);
    expect(lines).toHaveLength(6);
    expect(lines.slice(0, 5)).toEqual([0, 1, 2, 3, 4].map((k) => `Rejected element at ${1 + k} um (channel ${k})`));
    expect(lines[5]).toBe("and 2 more");
    expect(rejectedLines(seven)).toEqual(lines);
    expect(rejectedLines(seven.slice(0, 5))).toHaveLength(5);
    expect(rejectedLines([])).toEqual([]);

    expect(selectedPointLine(result(), 2)).toBe(
      "integration 2 of the series (INT_NUM 2), BMJD (TDB) 60000.2, white light 11 Jy, 5 elements, mean NPIXELS 7",
    );
    expect(selectedPointLine(result(), 99)).toBeNull();
    expect(selectedPointLine(result({ flux_unit: "unknown" }), 2)).toBe(
      "integration 2 of the series (INT_NUM 2), BMJD (TDB) 60000.2, white light 11, 5 elements, mean NPIXELS 7",
    );
    const missing = selectedPointLine(result({ white: curve([10, null, 12, 13], { npixels_mean: [7, null, 7, 7] }) }), 2);
    expect(missing).toContain("white light --");
    expect(missing).toContain("mean NPIXELS --");
    expect(selectedPointLine(result({ int_num: [5, 2, 2, 6], row_index: [0, 2, 3, 5], time_scale: "index", time: [1, 3, 4, 6] }), 2)).toBe(
      "integration 3 of the series (INT_NUM 2), integration number 3, white light 11 Jy, 5 elements, mean NPIXELS 7",
    );
  });
});
