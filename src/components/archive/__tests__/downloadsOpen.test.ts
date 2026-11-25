import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MastDownloadInput, MastDownloadItem } from "../../../shared/types/mast";

const { snapshotMock, listenMock, downloadMock, cancelMock } = vi.hoisted(() => ({ snapshotMock: vi.fn(), listenMock: vi.fn(), downloadMock: vi.fn(), cancelMock: vi.fn() }));

vi.mock("../../../services/mast", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../services/mast")>();
  return { ...actual, mastDownloads: snapshotMock, onMastDownload: listenMock, mastDownload: downloadMock, mastDownloadCancel: cancelMock };
});

type Controller = typeof import("../state/downloadsController");
type Stores = typeof import("../state/downloadsStore");
type Context = import("../state/downloadsStore").OpenContext;

const CAL = "jw02739001001_02105_00001_nrca1_cal.fits";
const DATASET = "jw02739001001_02105_00001_nrca1";
const ASKED: Context = { mode: "archive", mission: "jwst", datasetId: DATASET };

function item(over: Partial<MastDownloadItem> = {}): MastDownloadItem {
  return {
    id: "dl1:0",
    requestId: "dl1",
    mission: "jwst",
    datasetId: DATASET,
    filename: CAL,
    program: "2739",
    uri: `mast:JWST/product/${CAL}`,
    sizeBytes: 10,
    opensAs: "image",
    open: true,
    replace: false,
    state: "queued",
    received: 0,
    total: null,
    path: `C:\\data\\mast\\JWST\\2739\\${CAL}`,
    resumedFrom: 0,
    reused: false,
    verified: null,
    action: null,
    error: null,
    ...over,
  };
}

const INPUT: MastDownloadInput = { datasetId: DATASET, filename: CAL, program: "2739", sizeBytes: 10, uri: null, opensAs: "image", replace: false };

let push: ((item: MastDownloadItem) => void) | null = null;
let ctl: Controller;
let stores: Stores;

beforeEach(async () => {
  vi.resetModules();
  push = null;
  listenMock.mockImplementation(async (cb: (item: MastDownloadItem) => void) => {
    push = cb;
    return () => {};
  });
  snapshotMock.mockResolvedValue([]);
  downloadMock.mockImplementation(async () => [item()]);
  ctl = await import("../state/downloadsController");
  stores = await import("../state/downloadsStore");
});

async function askAndFinish(here: Context | null) {
  const store = stores.createDownloadsStore();
  const deps = { show: vi.fn(() => true), load: vi.fn(), setMode: vi.fn(), reload: vi.fn(), here: () => here };
  await ctl.watchDownloads(store, deps);
  expect(await ctl.startDownloads("jwst", [INPUT], true, store, ASKED)).toBe(true);
  await vi.waitFor(() => expect(push).not.toBeNull());
  push!(item({ state: "done", received: 10, total: 10 }));
  return { store, deps };
}

describe("Download and open that finishes later", () => {
  it("switches to the Workspace when the user is still on the dataset he asked from", async () => {
    const { store, deps } = await askAndFinish(ASKED);
    expect(deps.setMode).toHaveBeenCalledWith("workspace");
    expect(deps.show).toHaveBeenCalledWith(item().path);
    expect(stores.readyDownloads(store.getState())).toEqual([]);
  });

  it("loads the file without switching and offers the switch when the user moved to another dataset", async () => {
    const { store, deps } = await askAndFinish({ ...ASKED, datasetId: "jw02739001001_02105_00002_nrca1" });
    expect(deps.setMode).not.toHaveBeenCalled();
    expect(deps.show).not.toHaveBeenCalled();
    expect(deps.load).toHaveBeenCalledWith(item().path);
    expect(stores.readyDownloads(store.getState()).map((i) => i.id)).toEqual(["dl1:0"]);
  });

  it("does not take the screen when the user already left the archive, and Show in Workspace opens it once", async () => {
    const { store, deps } = await askAndFinish({ ...ASKED, mode: "workspace" });
    expect(deps.setMode).not.toHaveBeenCalled();
    const [ready] = stores.readyDownloads(store.getState());
    expect(ctl.showReady(ready, store, deps)).toBe(true);
    expect(deps.setMode).toHaveBeenCalledWith("workspace");
    expect(deps.show).toHaveBeenCalledWith(item().path);
    expect(stores.readyDownloads(store.getState())).toEqual([]);
  });

  it("keeps the dismissed notice gone", async () => {
    const { store } = await askAndFinish({ ...ASKED, mission: "hst" });
    const [ready] = stores.readyDownloads(store.getState());
    ctl.dismissReady(ready, store);
    expect(stores.readyDownloads(store.getState())).toEqual([]);
  });
});

describe("stillWhereAsked", () => {
  it("compares mode, mission and dataset, and keeps the old behaviour without a recorded request", async () => {
    expect(ctl.stillWhereAsked(ASKED, ASKED)).toBe(true);
    expect(ctl.stillWhereAsked(ASKED, { ...ASKED, mode: "workspace" })).toBe(false);
    expect(ctl.stillWhereAsked(ASKED, { ...ASKED, mission: "hst" })).toBe(false);
    expect(ctl.stillWhereAsked(ASKED, { ...ASKED, datasetId: "other" })).toBe(false);
    expect(ctl.stillWhereAsked(null, { ...ASKED, datasetId: "other" })).toBe(true);
  });
});

describe("Discard of a partial file", () => {
  it("asks the engine to drop the item and takes the list it answers", async () => {
    const store = stores.createDownloadsStore();
    const partial = { ...item({ state: "cancelled", action: "resume", received: 6 }), partBytes: 6 } as MastDownloadItem;
    store.dispatch({ type: "snapshot", items: [partial, item({ id: "dl2:0", state: "done" })] });
    cancelMock.mockResolvedValueOnce(1);
    snapshotMock.mockResolvedValueOnce([item({ id: "dl2:0", state: "done" })]);
    await ctl.discardDownload(partial, store);
    expect(cancelMock).toHaveBeenCalledWith("dl1:0");
    expect(store.getState().items.map((i) => i.id)).toEqual(["dl2:0"]);
    expect(store.getState().cancelError).toBeNull();
  });

  it("keeps the item and says why when the engine refuses", async () => {
    const store = stores.createDownloadsStore();
    const partial = { ...item({ state: "cancelled", action: "resume", received: 6 }), partBytes: 6 } as MastDownloadItem;
    store.dispatch({ type: "snapshot", items: [partial] });
    cancelMock.mockRejectedValueOnce({ code: "mast_cache", message: "the partial file is in use", context: {} });
    await ctl.discardDownload(partial, store);
    expect(store.getState().items.map((i) => i.id)).toEqual(["dl1:0"]);
    expect(store.getState().cancelError).toMatchObject({ code: "mast_cache" });
  });
});

describe("finished downloads in the live region", () => {
  it("announces a file that finished while watching", async () => {
    const store = stores.createDownloadsStore();
    const announce = vi.fn();
    await ctl.watchDownloads(store, { show: vi.fn(() => true), load: vi.fn(), setMode: vi.fn(), reload: vi.fn(), announce });
    await vi.waitFor(() => expect(push).not.toBeNull());
    push!(item({ open: false, state: "done", received: 10, total: 10 }));
    expect(announce).toHaveBeenCalledWith(`Downloaded ${CAL}`);
  });
});
