import { beforeEach, describe, expect, it, vi } from "vitest";
import type { LightCurveDoc } from "../../shared/types/lightCurve";
import type {
  SeriesKey,
  TsoCurve,
  TsoFile,
  TsoLightCurves,
  TsoLightCurvesRequest,
  TsoOpenResult,
  TsoSeriesSummary,
} from "../../shared/types/tso";
import { lightCurveStore } from "../useLightCurveStore";
import { DEFAULT_TSO_DRAFT, EMPTY_TSO_CURVES, EMPTY_TSO_SET, MAX_TSO_SETS, TsoStore } from "../useTsoStore";

type OpenReader = (paths: string[]) => Promise<TsoOpenResult>;
type CurvesReader = (paths: string[], series: SeriesKey, request: TsoLightCurvesRequest) => Promise<TsoLightCurves>;

const GROUP = "tso:abc:NRS1:0";
const NRS1: SeriesKey = { detector: "NRS1", sporder: 0 };
const REFUSAL = "a.fits: DATAMODL 'CubeModel' is not a time series of spectra";

function tsoFile(path: string): TsoFile {
  return {
    path,
    file_name: path.split("/").pop() ?? path,
    sha256: "0".repeat(64),
    level: 3,
    datamodel: "TSOMultiSpecModel",
    cal_ver: "1.18.0",
    exp_type: "NRS_BRIGHTOBJ",
    layout: "rows",
    nints: 4,
    intstart: 1,
    intend: 4,
    exsegnum: null,
    exsegtot: null,
    instrument: "NIRSPEC",
    detector: "NRS1",
    filter: "CLEAR",
    pupil: null,
    grating: "PRISM",
    subarray: "SUB2048",
    target: "T",
    expstart: 60000,
    outlier_detection: "COMPLETE",
    whitelight_log: null,
    whtlt_path: null,
    n_extract1d: 1,
    data_bytes: 1000,
  };
}

function nrs1Series(): TsoSeriesSummary {
  return {
    key: NRS1,
    label: "NRS1",
    n_integrations: 4,
    n_channels: 3,
    flux_unit: "Jy",
    time_scale: "bmjd_tdb",
    time_source: "TDB-MID column",
    time_first: 60000.1,
    time_last: 60000.4,
    wavelength_min_um: 1,
    wavelength_max_um: 2,
    common_grid: true,
    outlier_detection: "COMPLETE",
    completeness: [{ exposure: "e", nints: 4, found: 4, missing: [], complete: true }],
    notes: [],
    warnings: [],
  };
}

const OPEN: TsoOpenResult = { set_id: "abc", files: [tsoFile("C:/d/a.fits")], series: [nrs1Series()], notes: [], warnings: [] };
const SECOND_OPEN: TsoOpenResult = { ...OPEN, set_id: "def", files: [tsoFile("C:/d/a.fits"), tsoFile("C:/d/b.fits")] };

function curve(flux: (number | null)[]): TsoCurve {
  return {
    band_um: [1, 2],
    limits_origin: "open",
    flux,
    flux_err: flux.map(() => 0.5),
    n_used: flux.map(() => 5),
    npixels_mean: flux.map(() => 7),
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

const CURVES = result();

function whtltDoc(): LightCurveDoc {
  return {
    id: "whtlt:c",
    label: "whtlt",
    quantity: "flux",
    unit: "Jy",
    timeScale: "bmjd_tdb",
    time: [60000.1, 60000.2],
    value: [10, 11],
    err: [null, null],
    pointRef: [0, 1],
    flags: [[], []],
    source: { kind: "table", path: "C:/d/a_whtlt.ecsv", time_column: "BJD_TDB", value_column: "whitelight_flux" },
    notes: [],
    overlay: "whtlt",
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function ids(group: string): string[] {
  return lightCurveStore.get(group).map((doc) => doc.id);
}

beforeEach(() => {
  lightCurveStore.remove(GROUP);
  lightCurveStore.remove("other");
});

describe("TsoStore sets", () => {
  it("opens a set once per list of paths and keeps the error of a failed read", async () => {
    const open = vi.fn<OpenReader>();
    const first = deferred<TsoOpenResult>();
    open.mockReturnValueOnce(first.promise);
    const store = new TsoStore(open, vi.fn<CurvesReader>());
    const asked = ["a"];

    const opening = store.open("k", asked);
    const again = store.open("k", ["a"]);

    expect(open).toHaveBeenCalledTimes(1);
    expect(open).toHaveBeenCalledWith(["a"]);
    expect(open.mock.calls[0][0]).not.toBe(asked);
    expect(store.getSet("k")).toEqual({ paths: ["a"], loading: true, result: null, error: null });
    expect(open.mock.calls[0][0]).not.toBe(store.getSet("k").paths);

    first.resolve(OPEN);
    await Promise.all([opening, again]);
    expect(store.getSet("k")).toEqual({ paths: ["a"], loading: false, result: OPEN, error: null });

    await store.open("k", ["a"]);
    expect(open).toHaveBeenCalledTimes(1);

    open.mockRejectedValueOnce(new Error(REFUSAL));
    await store.open("e", ["a.fits"]);
    expect(store.getSet("e")).toEqual({ paths: ["a.fits"], loading: false, result: null, error: REFUSAL });

    open.mockRejectedValueOnce("plain refusal");
    await store.open("e", ["a.fits"]);
    expect(open).toHaveBeenCalledTimes(3);
    expect(store.getSet("e").error).toBe("plain refusal");

    open.mockResolvedValueOnce(SECOND_OPEN);
    await store.open("k", ["a", "b"]);
    expect(open).toHaveBeenCalledTimes(4);
    expect(open).toHaveBeenLastCalledWith(["a", "b"]);
    expect(store.getSet("k")).toEqual({ paths: ["a", "b"], loading: false, result: SECOND_OPEN, error: null });

    open.mockResolvedValueOnce(OPEN);
    await store.open("k", ["b", "a"]);
    expect(open).toHaveBeenCalledTimes(5);
  });

  it("drops a result that arrives after a newer request of the same file", async () => {
    const open = vi.fn<OpenReader>();
    const first = deferred<TsoOpenResult>();
    const second = deferred<TsoOpenResult>();
    open.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const store = new TsoStore(open, vi.fn<CurvesReader>());

    const older = store.open("k", ["a"]);
    const newer = store.open("k", ["a", "b"]);
    second.resolve(SECOND_OPEN);
    await newer;
    first.resolve(OPEN);
    await older;
    expect(store.getSet("k")).toEqual({ paths: ["a", "b"], loading: false, result: SECOND_OPEN, error: null });

    const late = deferred<TsoOpenResult>();
    open.mockReturnValueOnce(late.promise).mockResolvedValueOnce(OPEN);
    const failing = store.open("k", ["c"]);
    await store.open("k", ["a"]);
    late.reject(new Error("late"));
    await failing;
    expect(store.getSet("k")).toEqual({ paths: ["a"], loading: false, result: OPEN, error: null });
  });

  it("keeps the last eight sets", async () => {
    expect(MAX_TSO_SETS).toBe(8);
    const open = vi.fn<OpenReader>().mockResolvedValue(OPEN);
    const store = new TsoStore(open, vi.fn<CurvesReader>());

    for (let k = 1; k <= 8; k += 1) await store.open(`k${k}`, [`p${k}`]);
    store.setChosen("k1", ["x"]);
    store.setSeriesIndex("k1", 1);
    await store.open("k9", ["p9"]);

    expect(store.getSet("k1")).toBe(EMPTY_TSO_SET);
    expect(store.getChosen("k1")).toEqual([]);
    expect(store.getSeriesIndex("k1")).toBe(0);
    expect(store.getSet("k2").result).toBe(OPEN);
    expect(store.getSet("k9").result).toBe(OPEN);

    const pending = deferred<TsoOpenResult>();
    open.mockReturnValueOnce(pending.promise);
    const loading = store.open("slow", ["s"]);
    for (let k = 10; k <= 17; k += 1) await store.open(`k${k}`, [`p${k}`]);
    expect(store.getSet("slow").loading).toBe(true);
    expect(store.getSet("k9")).toBe(EMPTY_TSO_SET);
    pending.resolve(OPEN);
    await loading;
    expect(store.getSet("slow").result).toBe(OPEN);
  });

  it("remembers the ticked segments, the series index and the draft per key", () => {
    const store = new TsoStore(vi.fn<OpenReader>(), vi.fn<CurvesReader>());
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    expect(EMPTY_TSO_SET).toEqual({ paths: [], loading: false, result: null, error: null });
    expect(EMPTY_TSO_CURVES).toEqual({ loading: false, result: null, error: null });
    expect(DEFAULT_TSO_DRAFT).toEqual({
      mode: "pipeline",
      minText: "",
      maxText: "",
      binsText: "",
      extraBins: [],
      excludeText: "",
      rejectOn: true,
      rejectSigmaText: "100",
    });
    expect([EMPTY_TSO_SET, EMPTY_TSO_CURVES, DEFAULT_TSO_DRAFT].every(Object.isFrozen)).toBe(true);

    expect(store.getChosen("k")).toEqual([]);
    expect(store.getChosen("k")).toBe(store.getChosen("other"));
    expect(store.getSeriesIndex("k")).toBe(0);
    expect(store.getDraft("g")).toBe(DEFAULT_TSO_DRAFT);
    expect(store.getDraft("g")).toBe(store.getDraft("g"));
    expect(store.getSet("k")).toBe(EMPTY_TSO_SET);
    expect(store.getCurves("g")).toBe(EMPTY_TSO_CURVES);

    const ticked = ["b"];
    store.setChosen("k", ticked);
    ticked.push("c");
    expect(store.getChosen("k")).toEqual(["b"]);
    expect(listener).toHaveBeenCalledTimes(1);

    store.setSeriesIndex("k", 2);
    expect(store.getSeriesIndex("k")).toBe(2);
    expect(listener).toHaveBeenCalledTimes(2);

    store.patchDraft("g", { mode: "masked", binsText: "1 2" });
    const draft = store.getDraft("g");
    expect(listener).toHaveBeenCalledTimes(3);
    expect(draft).not.toBe(DEFAULT_TSO_DRAFT);
    expect(draft).toEqual({ ...DEFAULT_TSO_DRAFT, mode: "masked", binsText: "1 2" });
    expect(draft.rejectOn).toBe(true);
    expect(draft.rejectSigmaText).toBe("100");
    expect(DEFAULT_TSO_DRAFT.mode).toBe("pipeline");
    expect(store.getDraft("g")).toBe(draft);
    expect(store.getDraft("h")).toBe(DEFAULT_TSO_DRAFT);

    store.patchDraft("g", { extraBins: [[4.15, 4.5]] });
    expect(store.getDraft("g")).toEqual({ ...DEFAULT_TSO_DRAFT, mode: "masked", binsText: "1 2", extraBins: [[4.15, 4.5]] });

    store.setSeriesIndex("k", -1);
    expect(store.getSeriesIndex("k")).toBe(0);
    store.setSeriesIndex("k", 1.7);
    expect(store.getSeriesIndex("k")).toBe(1);
    store.setSeriesIndex("k", Number.NaN);
    expect(store.getSeriesIndex("k")).toBe(0);

    unsubscribe();
    store.setChosen("k", []);
    expect(listener).toHaveBeenCalledTimes(7);
  });
});

describe("TsoStore curves", () => {
  it("computes curves, keeps the last result on an error and writes the documents of the group", async () => {
    const curves = vi.fn<CurvesReader>();
    const pending = deferred<TsoLightCurves>();
    curves.mockReturnValueOnce(pending.promise);
    const store = new TsoStore(vi.fn<OpenReader>(), curves);

    expect(store.getRequest(GROUP)).toBeNull();
    const running = store.compute(GROUP, ["a"], NRS1, { mode: "pipeline" });
    expect(curves).toHaveBeenCalledWith(["a"], NRS1, { mode: "pipeline" });
    expect(store.getCurves(GROUP)).toEqual({ loading: true, result: null, error: null });
    expect(store.getRequest(GROUP)).toBeNull();

    pending.resolve(CURVES);
    await running;
    expect(store.getCurves(GROUP)).toEqual({ loading: false, result: CURVES, error: null });
    expect(ids(GROUP)).toEqual(["white"]);
    expect(lightCurveStore.get(GROUP)[0].source).toMatchObject({ kind: "tso", paths: ["a"], detector: "NRS1", sporder: 0 });
    expect(store.getRequest(GROUP)).toEqual({ mode: "pipeline" });
    const written = lightCurveStore.get(GROUP);

    const failing = deferred<TsoLightCurves>();
    curves.mockReturnValueOnce(failing.promise);
    const again = store.compute(GROUP, ["a"], NRS1, { mode: "masked" });
    expect(store.getCurves(GROUP)).toEqual({ loading: true, result: CURVES, error: null });
    expect(store.getRequest(GROUP)).toEqual({ mode: "pipeline" });
    failing.reject(new Error("x"));
    await again;
    expect(store.getCurves(GROUP)).toEqual({ loading: false, result: CURVES, error: "x" });
    expect(lightCurveStore.get(GROUP)).toBe(written);
    expect(store.getRequest(GROUP)).toEqual({ mode: "pipeline" });
    expect(store.getRequest("other")).toBeNull();

    const older = deferred<TsoLightCurves>();
    const newer = deferred<TsoLightCurves>();
    curves.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    const slow = store.compute(GROUP, ["a"], NRS1, { mode: "masked" });
    const fast = store.compute(GROUP, ["a", "b"], NRS1, { mode: "pipeline", bins: [[1, 1.5]] });
    const binned = result({ bins: [curve([4, 4, 4, 4])] });
    newer.resolve(binned);
    await fast;
    older.resolve(result({ mode: "masked" }));
    await slow;
    expect(store.getCurves(GROUP)).toEqual({ loading: false, result: binned, error: null });
    expect(store.getRequest(GROUP)).toEqual({ mode: "pipeline", bins: [[1, 1.5]] });
    expect(ids(GROUP)).toEqual(["white", "bin:0"]);
    expect(lightCurveStore.get(GROUP)[0].source).toMatchObject({ paths: ["a", "b"] });
  });

  it("adds extra documents after the curves and keeps them across a new computation", async () => {
    const curves = vi.fn<CurvesReader>().mockResolvedValue(CURVES);
    const store = new TsoStore(vi.fn<OpenReader>(), curves);
    const listener = vi.fn();
    store.subscribe(listener);

    store.setExtraDocs("other", [whtltDoc()]);
    expect(lightCurveStore.get("other")).toEqual([]);

    await store.compute(GROUP, ["a"], NRS1, { mode: "pipeline" });
    listener.mockClear();
    const extra = whtltDoc();
    const given = [extra];
    store.setExtraDocs(GROUP, given);
    given.pop();
    expect(ids(GROUP)).toEqual(["white", "whtlt:c"]);
    expect(lightCurveStore.get(GROUP)[1]).toBe(extra);
    expect(listener).not.toHaveBeenCalled();

    await store.compute(GROUP, ["a"], NRS1, { mode: "masked" });
    expect(ids(GROUP)).toEqual(["white", "whtlt:c"]);

    store.setExtraDocs(GROUP, []);
    expect(ids(GROUP)).toEqual(["white"]);

    await store.compute("other", ["a"], NRS1, { mode: "pipeline" });
    expect(ids("other")).toEqual(["white", "whtlt:c"]);
    lightCurveStore.remove("other");
  });
});
