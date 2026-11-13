import { describe, expect, it } from "vitest";
import { layerAssignmentEntry, pixelLayersEntry } from "../measurementLogLayers";
import { MeasurementLogCore, measurementLogCsv, pixelEntry, type MeasurementProvenance } from "../measurementLog";
import { CSV_LINE_END } from "../catalogCsv";
import type { PixelTableResult } from "../../shared/types/analysis";
import type { PixelTableLayers } from "../../shared/types/layers";
import type { PlaneInfo } from "../../shared/types/fits.types";

const PROV: MeasurementProvenance = { file: "x.fits", image: "original" };
const ERR_SOURCE = "EXTNAME VARIANCE, HDU 3: square root";
const DQ_SOURCE = "EXTNAME MASK, HDU 2";
const WGT_SOURCE = "user assignment (EXTNAME WGT), des-dr2_A_r_extn3.fits: 1 / square root, assumed inverse variance";
const PAIRING_NOTE = "CRPIX1 differs: image 50.5 against layer 51.5 (user assignment wgt_shift.fits)";

function pixelResult(extra: Partial<PixelTableResult & PixelTableLayers> = {}): PixelTableResult & PixelTableLayers {
  return {
    x: 12,
    y: 34,
    size: 3,
    x0: 11,
    y0: 33,
    values: [
      [1, 2, 3],
      [4, 5.5, 6],
      [7, 8, 9],
    ],
    err: [
      [0.1, 0.1, 0.1],
      [0.1, 0.25, 0.1],
      [0.1, 0.1, 0.1],
    ],
    dq: null,
    dq_names: null,
    dq_table: "jwst",
    unit: "MJy/sr",
    stats: { min: 1, max: 9, mean: 5.06, median: 5.5, n_finite: 9, n_nan: 0 },
    err_stats: null,
    wavelength: null,
    wavelength_stats: null,
    wavelength_unit: null,
    elapsed_ms: 2,
    ...extra,
  };
}

const PLANE: PlaneInfo = {
  kind: "hdu",
  index: 1,
  key: null,
  extname: null,
  extver: null,
  is_dq: false,
  is_err: false,
  dq_ref: null,
  err_ref: "C:/d/des-dr2_A_r_extn3.fits",
  source_path: "C:/d/des-dr2_A_r_extn1.fits",
  dq_table: null,
  err_origin: { role: "inverse_variance", source: WGT_SOURCE, assumed: true, n_no_data: null, origin: "user" },
  pairing_notes: [PAIRING_NOTE],
};

const DES: MeasurementProvenance = { file: "des-dr2_A_r_extn1.fits", image: "original" };

describe("pixelLayersEntry", () => {
  it("a logged pixel keeps the entry of pixelEntry and adds where its layers come from", () => {
    const result = pixelResult({ err_source: ERR_SOURCE, dq_source: DQ_SOURCE });
    const base = pixelEntry(PROV, result);
    expect(pixelLayersEntry(PROV, result)).toEqual({ ...base, params: { ...base.params, err_source: ERR_SOURCE, dq_source: DQ_SOURCE }, notes: [] });

    const bare = pixelResult();
    const bareBase = pixelEntry(PROV, bare);
    expect(pixelLayersEntry(PROV, bare)).toEqual({ ...bareBase, params: { ...bareBase.params, err_source: null, dq_source: null }, notes: [] });
  });
});

describe("layerAssignmentEntry", () => {
  it("an assignment is logged with what the layers are after it", () => {
    const entry = layerAssignmentEntry(DES, { role: "err", action: "assign" }, PLANE);
    expect(entry.kind).toBe("layer_assignment");
    expect(entry.file).toBe("des-dr2_A_r_extn1.fits");
    expect(entry.image).toBe("original");
    expect(entry.source).toBe('{"kind":"layers","role":"err","action":"assign"}');
    expect(entry.params).toEqual({ role: "err", action: "assign", exclusion_mask: null });
    expect(entry.values.err_source).toBe(WGT_SOURCE);
    expect(entry.values.err_origin).toBe("user");
    expect(entry.values.err_ref).toBe("C:/d/des-dr2_A_r_extn3.fits");
    expect(entry.values.dq_ref).toBeNull();
    expect(entry.notes).toEqual([PAIRING_NOTE]);
  });

  it("a chosen exclusion and a rebuilt SDSS error are logged with their numbers", () => {
    const chosen = layerAssignmentEntry(DES, { role: "exclusion", action: "choose", mask: 72 }, PLANE);
    expect(chosen.params.exclusion_mask).toBe(72);
    expect(chosen.source).toBe('{"kind":"layers","role":"exclusion","action":"choose"}');
    expect(layerAssignmentEntry(DES, { role: "exclusion", action: "default", mask: null }, PLANE).params.exclusion_mask).toBeNull();
    const rebuilt = layerAssignmentEntry(DES, { role: "err", action: "sdss_recipe" }, { ...PLANE, err_recipe: { kind: "sdss_frame", gain: 4.76, dark_variance: 1.3225 } });
    expect(rebuilt.values.sdss_gain).toBe(4.76);
    expect(rebuilt.values.sdss_dark_variance).toBe(1.3225);
  });

  it("the drafts go through the log and its CSV", () => {
    let n = 0;
    const store = new MeasurementLogCore(
      () => new Date("2026-11-12T10:00:00.000Z"),
      () => `id${++n}`,
    );
    store.append(pixelLayersEntry(PROV, pixelResult({ err_source: ERR_SOURCE, dq_source: DQ_SOURCE })));
    store.append(layerAssignmentEntry(DES, { role: "err", action: "assign" }, PLANE));
    expect(store.getSnapshot().map((e) => e.kind)).toEqual(["pixel", "layer_assignment"]);
    const lines = measurementLogCsv(store.getSnapshot()).split(CSV_LINE_END).filter((line) => line !== "");
    expect(lines).toHaveLength(3);
    const header = lines[0].split(",");
    expect(header).toContain("p_err_source");
    expect(header).toContain("v_err_source");
  });
});
