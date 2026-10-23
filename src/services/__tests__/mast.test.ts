import { describe, it, expect, beforeEach, vi } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const { typedInvokeMock, listenMock, unlistenMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

import {
  MAST_COMMANDS,
  MAST_DOWNLOAD_EVENT,
  MAST_ERROR_CODES,
  MAST_PROGRESS_EVENT,
  mastCacheClear,
  mastCacheInfo,
  mastCancel,
  mastDownload,
  mastDownloadCancel,
  mastDownloads,
  mastErrorOf,
  mastFootprints,
  mastFootprintsReg,
  mastImageList,
  mastPeek,
  mastMissions,
  mastPresets,
  mastPreview,
  mastProbeMission,
  mastProducts,
  mastResolve,
  mastSearch,
  mastThumbnail,
  mastTokenClear,
  mastTokenSave,
  mastTokenStatus,
  onMastDownload,
  onMastProgress,
} from "../mast";
import { TauriCommandError } from "../../shared/types/errors";
import type {
  MastDownloadInput,
  MastDownloadItem,
  MastFilterGroup,
  MastFilterOption,
  MastFootprint,
  MastFootprintInput,
  MastFootprintRegions,
  MastFootprintSet,
  MastPeek,
  MastPreset,
  MastProgressEvent,
  MastQuery,
  MastSearchPage,
  ObjectCategoryId,
  ObjectTypeId,
} from "../../shared/types/mast";

const rustSource = (rel: string) => readFileSync(fileURLToPath(new URL(`../../../src-tauri/src/${rel}`, import.meta.url)), "utf8");

const CAL_INPUT: MastDownloadInput = {
  datasetId: "jw02739001001_02105_00001_nrca1",
  filename: "jw02739001001_02105_00001_nrca1_cal.fits",
  program: "2739",
  sizeBytes: 117_573_120,
  uri: null,
  opensAs: "image",
  replace: false,
};

const FOOTPRINT_INPUT: MastFootprintInput = {
  datasetId: "jw02739-o001_t001_nircam",
  sRegion: "POLYGON ICRS  274.69 -13.89 274.70 -13.89 274.70 -13.88",
  polygons: null,
  targtype: "FIXED",
};

const PEEK: MastPeek = {
  datasetId: "j9irw3fwq",
  filename: "j9irw3fwq_flc.fits",
  uri: "mast:HST/product/j9irw3fwq_flc.fits",
  dims: [4096, 2048],
  bitpix: -32,
  bunit: "ELECTRONS",
  exptime: 507,
  dateObs: "2004-09-12T06:35:21",
  filter: "F606W",
  instrume: "ACS",
  footprints: [{ hdu: 1, corners: [[150, 2], [150.1, 2], [150.1, 2.1], [150, 2.1]], center: [150.05, 2.05], fovArcmin: [3.4, 1.7] }],
  hdus: [{ index: 1, extname: "SCI", extver: 1, bitpix: -32, dims: [4096, 2048], wcsError: null }],
  bytesFetched: 524_288,
  ranges: 7,
  fileBytes: 168_000_000,
  complete: true,
  rangeIgnored: false,
  note: null,
  etag: null,
  elapsedMs: 12,
  fromCache: false,
};

const QUERY: MastQuery = {
  mission: "jwst",
  where: { kind: "name", name: "M16", radiusArcmin: 3 },
  program: null,
  datasetId: null,
  filters: { level: "combined" },
  preset: null,
  sort: [],
  offset: 0,
  limit: 200,
};

describe("MAST_COMMANDS", () => {
  it("lists the 21 command names of the contract, in contract order", () => {
    expect(MAST_COMMANDS).toEqual([
      "mast_missions_cmd",
      "mast_probe_mission_cmd",
      "mast_resolve_cmd",
      "mast_search_cmd",
      "mast_image_list_cmd",
      "mast_thumbnail_cmd",
      "mast_preview_cmd",
      "mast_products_cmd",
      "mast_cancel_cmd",
      "mast_token_status_cmd",
      "mast_token_save_cmd",
      "mast_token_clear_cmd",
      "mast_cache_info_cmd",
      "mast_cache_clear_cmd",
      "mast_download_cmd",
      "mast_download_cancel_cmd",
      "mast_downloads_cmd",
      "mast_presets_cmd",
      "mast_footprints_cmd",
      "mast_footprints_reg_cmd",
      "mast_peek_cmd",
    ]);
  });

  it("matches the commands cmd/mast.rs defines, in the same order", () => {
    const defined = [...rustSource("cmd/mast.rs").matchAll(/#\[tauri::command\]\s*pub async fn (\w+)\(/g)].map((m) => m[1]);
    expect(defined).toEqual([...MAST_COMMANDS]);
  });
});

describe("MAST error codes", () => {
  it("mirror MastErrorCode of core/mast/types.rs, in the same order", () => {
    const rust = rustSource("core/mast/types.rs");
    const block = rust.slice(rust.indexOf("pub enum MastErrorCode"), rust.indexOf("impl MastErrorCode"));
    const codes = [...block.matchAll(/#\[serde\(rename = "(mast_\w+)"\)\]/g)].map((m) => m[1]);
    expect(codes).toHaveLength(21);
    expect(codes[20]).toBe("mast_peek_incomplete");
    expect([...MAST_ERROR_CODES]).toEqual(codes);
  });

  it.each(["mast_size_mismatch", "mast_busy", "mast_stale", "mast_quota", "mast_uri_refused", "mast_peek_incomplete"])("keeps the download code %s and its context", (code) => {
    const err = new TauriCommandError("mast_download_cmd", "refused", code, { bytes: 5, detail: "local \"ab-v0\", MAST \"ab-v1\"" });
    expect(mastErrorOf(err)).toEqual({ code, message: "refused", context: { bytes: 5, detail: 'local "ab-v0", MAST "ab-v1"' } });
  });
});

describe("MAST presets contract", () => {
  const camel = (name: string) => name.replace(/_([a-z])/g, (_m, c: string) => c.toUpperCase());
  const fields = (rust: string, name: string) => {
    const start = rust.indexOf(`pub struct ${name} {`);
    return [...rust.slice(start, rust.indexOf("\n}", start)).matchAll(/pub (\w+):/g)].map((m) => camel(m[1]));
  };

  it("mirror MastPreset, MastQuery.preset and MastSearchPage.droppedConditions of core/mast/types.rs", () => {
    const rust = rustSource("core/mast/types.rs");
    const preset: MastPreset = {
      id: "tso",
      mission: "jwst",
      label: "TSO",
      description: "Time-series observations",
      conditions: [{ column: "tsovisit", value: "t" }],
      products: ["x1dints", "calints"],
    };
    expect(fields(rust, "MastPreset")).toEqual(Object.keys(preset));
    expect(fields(rust, "MastPresetCondition")).toEqual(Object.keys(preset.conditions[0]));
    expect(fields(rust, "MastQuery")).toContain("preset");
    expect(fields(rust, "MastSearchPage")).toContain("droppedConditions");
  });

  it("mirror the footprint input, result, set and region types of core/mast/types.rs", () => {
    const rust = rustSource("core/mast/types.rs");
    const footprint: MastFootprint = {
      datasetId: "a",
      polygons: [[[0, 0], [1, 0], [1, 1]]],
      onImage: true,
      vertices: 3,
      cone: { ra: 274.7, dec: -13.8, radiusDeg: 0.05 },
      note: null,
    };
    const set: MastFootprintSet = { path: "a.fits", width: 2048, height: 2048, footprints: [footprint] };
    const regions: MastFootprintRegions = { regText: "icrs\n", polygons: 1, notes: [{ datasetId: "b", note: "moving" }] };
    expect(fields(rust, "MastFootprintInput")).toEqual(Object.keys(FOOTPRINT_INPUT));
    expect(fields(rust, "MastFootprint")).toEqual(Object.keys(footprint));
    expect(fields(rust, "MastFootprintSet")).toEqual(Object.keys(set));
    expect(fields(rust, "MastFootprintRegions")).toEqual(Object.keys(regions));
    expect(fields(rust, "MastFootprintNote")).toEqual(Object.keys(regions.notes[0]));
    expect(fields(rustSource("core/astrometry/stcs.rs"), "BoundingCone")).toEqual(Object.keys(footprint.cone!));
  });

  it("mirror the download input and item of core/mast/types.rs", () => {
    const rust = rustSource("core/mast/types.rs");
    const item: MastDownloadItem = {
      id: "dl1:0",
      requestId: "dl1",
      mission: "jwst",
      datasetId: CAL_INPUT.datasetId,
      filename: CAL_INPUT.filename,
      program: "2739",
      uri: `mast:JWST/product/${CAL_INPUT.filename}`,
      sizeBytes: CAL_INPUT.sizeBytes,
      opensAs: "image",
      open: false,
      replace: false,
      state: "done",
      received: CAL_INPUT.sizeBytes,
      total: CAL_INPUT.sizeBytes,
      path: `C:\\data\\mast\\JWST\\2739\\${CAL_INPUT.filename}`,
      resumedFrom: 0,
      reused: false,
      verified: true,
      action: null,
      error: null,
    };
    expect(fields(rust, "MastDownloadInput")).toEqual(Object.keys(CAL_INPUT));
    expect(fields(rust, "MastDownloadItem")).toEqual(Object.keys(item));
  });

  it("mirror the object filter contract of core/mast/types.rs", () => {
    const rust = rustSource("core/mast/types.rs");
    const snake = (name: string) => name.replace(/(?<!^)([A-Z])/g, "_$1").toLowerCase();
    const variants = (name: string) => {
      const start = rust.indexOf(`pub enum ${name} {`);
      if (start < 0) return [];
      const body = rust.slice(rust.indexOf("{", start) + 1, rust.indexOf("}", start));
      return [...body.matchAll(/([A-Z]\w*)/g)].map((m) => snake(m[1]));
    };
    const option: MastFilterOption = { id: "galaxies", label: "Galaxies", title: "targcat = Galaxy" };
    const group: MastFilterGroup = { id: "category", label: "Category", column: "targcat", info: "IUE object class (IUE_CATEGORY).", options: [option] };
    const categories: ObjectCategoryId[] = ["galaxies", "galaxy_clusters", "ism", "star_clusters", "stars", "solar_system", "fields", "calibration"];
    const types: ObjectTypeId[] = [
      "planetary_nebulae",
      "nebulae_star_forming",
      "young_stars_disks",
      "exoplanets",
      "agn_quasars",
      "interacting_galaxies",
      "supernovae_transients",
      "compact_substellar",
    ];
    const page: Pick<MastSearchPage, "levelFilteredLocally" | "levelAppliedLocally" | "unfilteredTotal"> = { levelFilteredLocally: true, levelAppliedLocally: "productLevel = 3", unfilteredTotal: 1913 };
    expect(fields(rust, "MastFilterOption")).toEqual(Object.keys(option));
    expect(fields(rust, "MastFilterGroup")).toEqual(Object.keys(group));
    expect(fields(rust, "MastMission")).toContain("filterGroups");
    expect(fields(rust, "MastRow")).toEqual(expect.arrayContaining(["categories", "targetDescription"]));
    expect(fields(rust, "MastSearchPage")).toEqual(expect.arrayContaining(Object.keys(page)));
    expect(fields(rust, "MastFilterValues")).toEqual(expect.arrayContaining(["level", "categories", "objectTypes"]));
    expect(variants("LevelFilter")).toEqual(["combined", "exposures", "all"]);
    expect(variants("FilterGroupId")).toEqual(["category", "object_type"]);
    expect(variants("ObjectCategory")).toEqual(categories);
    expect(variants("ObjectType")).toEqual(types);
  });

  it("mirror the header peek result of core/mast/types.rs", () => {
    const rust = rustSource("core/mast/types.rs");
    expect(fields(rust, "MastPeek")).toEqual(Object.keys(PEEK));
    expect(fields(rust, "MastPeekFootprint")).toEqual(Object.keys(PEEK.footprints[0]));
    expect(fields(rust, "MastPeekHdu")).toEqual(Object.keys(PEEK.hdus[0]));
  });
});

describe("MAST command wrappers", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue(null);
  });

  it.each<[string, () => Promise<unknown>, string, Record<string, unknown>]>([
    ["missions", () => mastMissions(), "mast_missions_cmd", {}],
    ["probe", () => mastProbeMission("tess"), "mast_probe_mission_cmd", { mission: "tess" }],
    ["resolve", () => mastResolve("g1:resolve", "M16"), "mast_resolve_cmd", { requestId: "g1:resolve", name: "M16" }],
    ["search", () => mastSearch("g1:search", QUERY), "mast_search_cmd", { requestId: "g1:search", query: QUERY, refresh: false }],
    ["search with refresh", () => mastSearch("g2:search", QUERY, true), "mast_search_cmd", { requestId: "g2:search", query: QUERY, refresh: true }],
    [
      "image list",
      () => mastImageList("g1:list:jw02739-o001_t001_nircam", "jwst", "jw02739-o001_t001_nircam"),
      "mast_image_list_cmd",
      { requestId: "g1:list:jw02739-o001_t001_nircam", mission: "jwst", datasetId: "jw02739-o001_t001_nircam" },
    ],
    [
      "representative thumbnail",
      () => mastThumbnail("g1:thumb:ICK909030", "hst", "ICK909030", "unknown", null),
      "mast_thumbnail_cmd",
      { requestId: "g1:thumb:ICK909030", mission: "hst", datasetId: "ICK909030", levelClass: "unknown", filename: null },
    ],
    [
      "named thumbnail",
      () => mastThumbnail("g1:thumb:a:b", "jwst", "a", "exposure", "b_thumb.jpg"),
      "mast_thumbnail_cmd",
      { requestId: "g1:thumb:a:b", mission: "jwst", datasetId: "a", levelClass: "exposure", filename: "b_thumb.jpg" },
    ],
    [
      "preview",
      () => mastPreview("g1:preview:a:b.jpg:pane", "jwst", "a", "b.jpg", "pane"),
      "mast_preview_cmd",
      { requestId: "g1:preview:a:b.jpg:pane", mission: "jwst", datasetId: "a", filename: "b.jpg", size: "pane" },
    ],
    [
      "products",
      () => mastProducts("g1:products:a", "hst", ["a"]),
      "mast_products_cmd",
      { requestId: "g1:products:a", mission: "hst", datasetIds: ["a"], refresh: false },
    ],
    [
      "products with refresh",
      () => mastProducts("g1:products:a", "hst", ["a"], true),
      "mast_products_cmd",
      { requestId: "g1:products:a", mission: "hst", datasetIds: ["a"], refresh: true },
    ],
    ["cancel by prefix", () => mastCancel({ prefix: "g1:" }), "mast_cancel_cmd", { requestIds: null, prefix: "g1:" }],
    ["cancel by id", () => mastCancel({ requestIds: ["g2:preview:x"] }), "mast_cancel_cmd", { requestIds: ["g2:preview:x"], prefix: null }],
    ["token status", () => mastTokenStatus("token:check", true), "mast_token_status_cmd", { requestId: "token:check", check: true }],
    ["token status without check", () => mastTokenStatus("token:check", false), "mast_token_status_cmd", { requestId: "token:check", check: false }],
    ["token save", () => mastTokenSave("token:save", "AB-TEST-TOKEN-0123456789abcdef"), "mast_token_save_cmd", { requestId: "token:save", token: "AB-TEST-TOKEN-0123456789abcdef" }],
    ["token clear", () => mastTokenClear(), "mast_token_clear_cmd", {}],
    ["cache info", () => mastCacheInfo(), "mast_cache_info_cmd", {}],
    ["cache clear", () => mastCacheClear("images"), "mast_cache_clear_cmd", { kind: "images" }],
    [
      "download",
      () => mastDownload("dl1", "jwst", [CAL_INPUT], true),
      "mast_download_cmd",
      { requestId: "dl1", mission: "jwst", items: [CAL_INPUT], open: true },
    ],
    ["download cancel", () => mastDownloadCancel("dl1:0"), "mast_download_cancel_cmd", { requestId: "dl1:0" }],
    ["downloads", () => mastDownloads(), "mast_downloads_cmd", {}],
    ["presets", () => mastPresets(), "mast_presets_cmd", {}],
    ["search with a preset", () => mastSearch("g3:search", { ...QUERY, preset: "tso" }), "mast_search_cmd", { requestId: "g3:search", query: { ...QUERY, preset: "tso" }, refresh: false }],
    ["footprints", () => mastFootprints("C:\\data\\a_cal.fits", [FOOTPRINT_INPUT]), "mast_footprints_cmd", { path: "C:\\data\\a_cal.fits", items: [FOOTPRINT_INPUT] }],
    ["footprint regions", () => mastFootprintsReg([FOOTPRINT_INPUT]), "mast_footprints_reg_cmd", { items: [FOOTPRINT_INPUT] }],
    [
      "header peek",
      () => mastPeek("peek:j9irw3fwq:j9irw3fwq_flc.fits", "hst", "j9irw3fwq", "j9irw3fwq_flc.fits", null),
      "mast_peek_cmd",
      { requestId: "peek:j9irw3fwq:j9irw3fwq_flc.fits", mission: "hst", datasetId: "j9irw3fwq", filename: "j9irw3fwq_flc.fits", uri: null },
    ],
    [
      "header peek of an HLSP file",
      () => mastPeek("peek:c:x_i2d.fits", "jwst", "c", "x_i2d.fits", "mast:HLSP/ceers/nircam1/x_i2d.fits"),
      "mast_peek_cmd",
      { requestId: "peek:c:x_i2d.fits", mission: "jwst", datasetId: "c", filename: "x_i2d.fits", uri: "mast:HLSP/ceers/nircam1/x_i2d.fits" },
    ],
  ])("%s calls the exact command with every key of the contract", async (_label, call, command, args) => {
    await call();
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
    expect(typedInvokeMock).toHaveBeenCalledWith(command, args);
    expect(MAST_COMMANDS).toContain(command);
  });

  it("covers every command of MAST_COMMANDS with a wrapper", async () => {
    await Promise.all([
      mastMissions(),
      mastProbeMission("jwst"),
      mastResolve("r", "M16"),
      mastSearch("s", QUERY),
      mastImageList("l", "jwst", "a"),
      mastThumbnail("t", "jwst", "a", "combined", null),
      mastPreview("p", "jwst", "a", "b.jpg", "large"),
      mastProducts("q", "jwst", ["a"]),
      mastCancel({}),
      mastTokenStatus("token:check", false),
      mastTokenSave("token:save", "x"),
      mastTokenClear(),
      mastCacheInfo(),
      mastCacheClear("all"),
      mastDownload("d", "hst", [CAL_INPUT], false),
      mastDownloadCancel("d"),
      mastDownloads(),
      mastPresets(),
      mastFootprints("a.fits", [FOOTPRINT_INPUT]),
      mastFootprintsReg([FOOTPRINT_INPUT]),
      mastPeek("k", "hst", "a", "a_flc.fits", null),
    ]);
    const called = typedInvokeMock.mock.calls.map((c) => c[0]);
    expect([...called].sort()).toEqual([...MAST_COMMANDS].sort());
  });
});

describe("mastErrorOf", () => {
  it("maps a TauriCommandError with a mast code and context", () => {
    const err = new TauriCommandError("mast_search_cmd", "connection refused", "mast_network", { operation: "search/jwst/api/v0.1/search" });
    expect(mastErrorOf(err)).toEqual({ code: "mast_network", message: "connection refused", context: { operation: "search/jwst/api/v0.1/search" } });
  });

  it("keeps only the typed context keys", () => {
    const err = new TauriCommandError("mast_thumbnail_cmd", "too large", "mast_too_large", {
      bytes: 300000,
      operation: "search/jwst/api/v0.1/get_image_thumbnail",
      headers: "x",
      status: "413",
    });
    expect(mastErrorOf(err).context).toEqual({ bytes: 300000, operation: "search/jwst/api/v0.1/get_image_thumbnail" });
  });

  it("keeps the attempt count, the time MAST took and the host it tried", () => {
    const err = new TauriCommandError("mast_thumbnail_cmd", "no answer within 30 s", "mast_timeout", {
      timeoutSec: 30,
      attempts: 2,
      elapsedMs: 61_000,
      host: "127.0.0.1:9",
      operation: "search/hst/api/v0.1/get_image_thumbnail",
    });
    expect(mastErrorOf(err).context).toEqual({ timeoutSec: 30, attempts: 2, elapsedMs: 61_000, host: "127.0.0.1:9", operation: "search/hst/api/v0.1/get_image_thumbnail" });
  });

  it("maps a raw structured rejection object", () => {
    expect(mastErrorOf({ code: "mast_rate_limited", message: "429", context: { retryAfterSec: 5, consecutive: 2 } })).toEqual({
      code: "mast_rate_limited",
      message: "429",
      context: { retryAfterSec: 5, consecutive: 2 },
    });
  });

  it("turns an unknown code into mast_decode with the original message", () => {
    const err = new TauriCommandError("mast_search_cmd", "something new", "mast_brand_new", { status: 418 });
    expect(mastErrorOf(err)).toEqual({ code: "mast_decode", message: "something new", context: {} });
  });

  it("turns a plain string or Error into mast_decode", () => {
    expect(mastErrorOf("command mast_search_cmd not found")).toEqual({ code: "mast_decode", message: "command mast_search_cmd not found", context: {} });
    expect(mastErrorOf(new Error("boom"))).toEqual({ code: "mast_decode", message: "boom", context: {} });
  });

  it("gives an empty context when the error carries none", () => {
    expect(mastErrorOf(new TauriCommandError("mast_cancel_cmd", "cancelled", "mast_cancelled"))).toEqual({
      code: "mast_cancelled",
      message: "cancelled",
      context: {},
    });
  });
});

describe("onMastProgress", () => {
  beforeEach(() => {
    listenMock.mockReset();
    unlistenMock.mockReset();
  });

  it("subscribes to mast:progress, forwards the payload and returns the unlisten", async () => {
    let handler: ((e: { payload: MastProgressEvent }) => void) | null = null;
    listenMock.mockImplementation(async (_name: string, cb: (e: { payload: MastProgressEvent }) => void) => {
      handler = cb;
      return unlistenMock;
    });
    const seen: MastProgressEvent[] = [];
    const unlisten = await onMastProgress((e) => seen.push(e));
    expect(MAST_PROGRESS_EVENT).toBe("mast:progress");
    expect(listenMock).toHaveBeenCalledWith("mast:progress", expect.any(Function));
    const payload: MastProgressEvent = { requestId: "g1:preview:a:b.jpg:pane", received: 10, total: 100, stage: "download" };
    handler!({ payload });
    expect(seen).toEqual([payload]);
    unlisten();
    expect(unlistenMock).toHaveBeenCalledTimes(1);
  });
});

describe("onMastDownload", () => {
  beforeEach(() => {
    listenMock.mockReset();
    unlistenMock.mockReset();
  });

  it("subscribes to mast:download and forwards each item", async () => {
    let handler: ((e: { payload: MastDownloadItem }) => void) | null = null;
    listenMock.mockImplementation(async (_name: string, cb: (e: { payload: MastDownloadItem }) => void) => {
      handler = cb;
      return unlistenMock;
    });
    const seen: MastDownloadItem[] = [];
    const unlisten = await onMastDownload((item) => seen.push(item));
    expect(MAST_DOWNLOAD_EVENT).toBe("mast:download");
    expect(listenMock).toHaveBeenCalledWith("mast:download", expect.any(Function));
    const payload = { id: "dl1:0", state: "downloading", received: 10 } as MastDownloadItem;
    handler!({ payload });
    expect(seen).toEqual([payload]);
    unlisten();
    expect(unlistenMock).toHaveBeenCalledTimes(1);
  });
});
