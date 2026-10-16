import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock, listenMock, unlistenMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

import {
  MAST_COMMANDS,
  MAST_PROGRESS_EVENT,
  mastCacheClear,
  mastCacheInfo,
  mastCancel,
  mastErrorOf,
  mastImageList,
  mastMissions,
  mastPreview,
  mastProbeMission,
  mastProducts,
  mastResolve,
  mastSearch,
  mastThumbnail,
  mastTokenClear,
  mastTokenSave,
  mastTokenStatus,
  onMastProgress,
} from "../mast";
import { TauriCommandError } from "../../shared/types/errors";
import type { MastProgressEvent, MastQuery } from "../../shared/types/mast";

const QUERY: MastQuery = {
  mission: "jwst",
  where: { kind: "name", name: "M16", radiusArcmin: 3 },
  program: null,
  datasetId: null,
  filters: { level: "combined" },
  sort: [],
  offset: 0,
  limit: 200,
};

describe("MAST_COMMANDS", () => {
  it("lists the 14 command names of the frozen contract, in contract order", () => {
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
    ]);
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
