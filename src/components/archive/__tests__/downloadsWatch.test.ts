import { describe, expect, it, vi } from "vitest";
import type { MastDownloadItem } from "../../../shared/types/mast";

const { snapshotMock, listenMock, cancelMock } = vi.hoisted(() => ({ snapshotMock: vi.fn(), listenMock: vi.fn(), cancelMock: vi.fn() }));

vi.mock("../../../services/mast", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../services/mast")>();
  return { ...actual, mastDownloads: snapshotMock, onMastDownload: listenMock, mastDownloadCancel: cancelMock };
});

import { createDownloadsStore } from "../state/downloadsStore";
import { cancelDownload, watchDownloads } from "../state/downloadsController";

function item(over: Partial<MastDownloadItem>): MastDownloadItem {
  return {
    id: "dl1:0",
    requestId: "dl1",
    mission: "jwst",
    datasetId: "d",
    filename: "d_cal.fits",
    program: "2739",
    uri: "mast:JWST/product/d_cal.fits",
    sizeBytes: 10,
    opensAs: "image",
    open: true,
    replace: false,
    state: "done",
    received: 10,
    total: 10,
    path: "C:\\data\\mast\\JWST\\2739\\d_cal.fits",
    resumedFrom: 0,
    reused: false,
    verified: true,
    action: null,
    error: null,
    ...over,
  };
}

describe("watchDownloads", () => {
  it("opens files that finish while watching, not the ones already done when the window loaded", async () => {
    let push: ((item: MastDownloadItem) => void) | null = null;
    listenMock.mockImplementation(async (cb: (item: MastDownloadItem) => void) => {
      push = cb;
      return () => {};
    });
    snapshotMock.mockResolvedValue([item({ id: "old:0" }), item({ id: "run:0", state: "downloading", received: 4 })]);
    const store = createDownloadsStore();
    const ingest = vi.fn(() => true);
    const setMode = vi.fn();
    await watchDownloads(store, { ingest, setMode, reload: vi.fn() });
    expect(store.getState().items.map((i) => i.id)).toEqual(["old:0", "run:0"]);
    expect(ingest).not.toHaveBeenCalled();
    push!(item({ id: "run:0" }));
    expect(ingest).toHaveBeenCalledTimes(1);
    expect(setMode).toHaveBeenCalledWith("workspace");
    push!(item({ id: "run:0" }));
    expect(ingest).toHaveBeenCalledTimes(1);
    expect(store.getState().enqueueError).toBeNull();
  });

  it("keeps a failed Cancel out of the refusal banner of the Download bar", async () => {
    cancelMock.mockRejectedValueOnce({ code: "mast_decode", message: "the app did not answer", context: {} });
    const store = createDownloadsStore();
    await cancelDownload(item({ state: "downloading", received: 4 }), store);
    expect(cancelMock).toHaveBeenCalledWith("dl1:0");
    expect(store.getState().enqueueError).toBeNull();
    expect(store.getState().cancelError).toMatchObject({ code: "mast_decode", message: "the app did not answer" });
    cancelMock.mockResolvedValueOnce(1);
    await cancelDownload(item({ state: "downloading", received: 4 }), store);
    expect(store.getState().cancelError).toBeNull();
  });

  it("stays quiet outside the desktop app, where the event bridge is missing", async () => {
    vi.resetModules();
    listenMock.mockRejectedValue(new Error("window.__TAURI_INTERNALS__ is undefined"));
    const fresh = await import("../state/downloadsController");
    const stores = await import("../state/downloadsStore");
    const store = stores.createDownloadsStore();
    await fresh.watchDownloads(store, { ingest: vi.fn(() => true), setMode: vi.fn(), reload: vi.fn() });
    expect(store.getState().enqueueError).toBeNull();
    expect(store.getState().items).toEqual([]);
  });
});
