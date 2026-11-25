import { describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { MastDownloadItem } from "../../../shared/types/mast";
import { discardable, downloadRoot, downloadStatusText, partialText, rateText } from "../logic/downloadView";
import { createDownloadsState, downloadsReducer, rateOf } from "../state/downloadsStore";
import { finishedText } from "../state/downloadsController";
import { innermostLayer } from "../logic/escLayers";
import { DownloadQueueView } from "../DownloadQueue";
import { DownloadReadyView } from "../DownloadReady";
import { WorkspaceDownloadsView } from "../WorkspaceDownloads";

const CAL = "jw02739001001_02105_00001_nrca1_cal.fits";

function item(over: Partial<MastDownloadItem> & { partBytes?: number } = {}): MastDownloadItem {
  return {
    id: "dl1:0",
    requestId: "dl1",
    mission: "jwst",
    datasetId: "jw02739001001_02105_00001_nrca1",
    filename: CAL,
    program: "2739",
    uri: `mast:JWST/product/${CAL}`,
    sizeBytes: 2_100_000_000,
    opensAs: "image",
    open: false,
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
  } as MastDownloadItem;
}

const handlers = () => ({ onCancel: vi.fn(), onAction: vi.fn(), onOpen: vi.fn(), onReveal: vi.fn(), onDiscard: vi.fn(), onClose: vi.fn() });

describe("speed and time left", () => {
  it("measures the speed from progress events and words the time left", () => {
    let state = downloadsReducer(createDownloadsState(), { type: "item", item: item({ state: "downloading", received: 0, total: 2_100_000_000 }), at: 0 });
    expect(rateOf(state, "dl1:0")).toBeNull();
    state = downloadsReducer(state, { type: "item", item: item({ state: "downloading", received: 18_000_000, total: 2_100_000_000 }), at: 1_000 });
    expect(rateOf(state, "dl1:0")).toBe(18_000_000);
    state = downloadsReducer(state, { type: "item", item: item({ state: "downloading", received: 18_100_000, total: 2_100_000_000 }), at: 1_100 });
    expect(rateOf(state, "dl1:0")).toBe(18_000_000);
    const running = item({ state: "downloading", received: 861_000_000, total: 2_100_000_000 });
    expect(rateText(running, 18_000_000)).toBe("18 MB/s, 1 min left");
    expect(downloadStatusText(running, 18_000_000)).toBe("41% of 2.10 GB, 18 MB/s, 1 min left");
    expect(downloadStatusText(running)).toBe("41% of 2.10 GB");
    expect(rateText(item({ state: "done" }), 18_000_000)).toBeNull();
    state = downloadsReducer(state, { type: "item", item: item({ state: "done", received: 2_100_000_000, total: 2_100_000_000 }), at: 5_000 });
    expect(rateOf(state, "dl1:0")).toBeNull();
  });
});

describe("downloads drawer", () => {
  it("labels Cancel on a running item Pause and keeps Cancel for a queued one", () => {
    const html = renderToStaticMarkup(createElement(DownloadQueueView, { items: [item({ id: "a", state: "queued" }), item({ id: "b", state: "downloading", received: 10, total: 100 })], ...handlers() }));
    expect(html.match(/>Pause</g)).toHaveLength(1);
    expect(html.match(/>Cancel</g)).toHaveLength(1);
  });

  it("names the folder the files go to and opens it", () => {
    expect(downloadRoot([item()])).toBe("C:\\data\\mast");
    expect(downloadRoot([{ path: "/home/u/AstroBurst/mast/HST/1234/x_flt.fits" }])).toBe("/home/u/AstroBurst/mast");
    expect(downloadRoot([{ path: "C:\\elsewhere\\x.fits" }])).toBeNull();
    const html = renderToStaticMarkup(createElement(DownloadQueueView, { items: [item({ state: "done" })], ...handlers() }));
    expect(html).toContain('data-testid="mast-dl-root"');
    expect(html).toContain("Saved under C:\\data\\mast");
    expect(html).toContain(">Open folder<");
  });

  it("is a keyboard region of the archive and closes with Escape", () => {
    const html = renderToStaticMarkup(createElement(DownloadQueueView, { items: [], ...handlers() }));
    expect(html).toContain('data-mast-region="downloads"');
    const none = { menuOpen: false, largeOpen: false, drawerOpen: false, tokenChecking: false, searching: false };
    expect(innermostLayer({ ...none, downloadsOpen: true, searching: true })).toBe("downloads");
    expect(innermostLayer({ ...none, drawerOpen: true, downloadsOpen: true })).toBe("drawer");
  });

  it("announces finished downloads", () => {
    expect(finishedText([item()])).toBe(`Downloaded ${CAL}`);
    expect(finishedText([item(), item({ id: "b" })])).toBe("Downloaded 2 files");
    expect(finishedText([])).toBe("");
  });
});

describe("partial files left by an interrupted download", () => {
  const interrupted = item({ state: "cancelled", action: "resume", received: 1_260_000_000, total: 2_100_000_000, partBytes: 1_260_000_000 });

  it("shows them with Resume and Discard and counts their bytes against the quota", () => {
    expect(discardable(interrupted)).toBe(true);
    expect(discardable(item({ state: "cancelled", action: "resume", received: 5 }))).toBe(false);
    expect(discardable(item({ state: "downloading", partBytes: 5 } as never))).toBe(false);
    expect(downloadStatusText(interrupted)).toBe("Interrupted at 60% of 2.10 GB, the partial file is kept");
    expect(partialText([interrupted, item({ id: "b", state: "done" })])).toBe("1 partial file, 1.26 GB on disk, counted in the MAST download quota until resumed or discarded");
    expect(partialText([item({ state: "done" })])).toBeNull();
    const html = renderToStaticMarkup(createElement(DownloadQueueView, { items: [interrupted], ...handlers() }));
    expect(html).toContain(">Resume<");
    expect(html).toContain(">Discard<");
    expect(html).toContain('data-testid="mast-dl-partial"');
  });

  it("offers no Discard while the engine gives no partial size", () => {
    const html = renderToStaticMarkup(createElement(DownloadQueueView, { items: [item({ state: "cancelled", action: "resume", received: 10, total: 100 })], ...handlers() }));
    expect(html).not.toContain(">Discard<");
    expect(html).not.toContain("mast-dl-partial");
  });
});

describe("downloads seen from the Workspace", () => {
  it("shows the pill while transfers run and the files that are ready", () => {
    const onShow = vi.fn();
    const counts = { downloading: 1, queued: 2, done: 0, failed: 0, stopped: 0, total: 3 };
    const html = renderToStaticMarkup(createElement(WorkspaceDownloadsView, { counts, ready: [], onDownloads: vi.fn(), onShow, onDismiss: vi.fn() }));
    expect(html).toContain('data-testid="mast-downloads-pill"');
    expect(html).toContain("Downloads: 1 running, 2 queued");
    const quiet = renderToStaticMarkup(createElement(WorkspaceDownloadsView, { counts: { ...counts, downloading: 0, queued: 0 }, ready: [], onDownloads: vi.fn(), onShow, onDismiss: vi.fn() }));
    expect(quiet).toBe("");
    const ready = renderToStaticMarkup(createElement(WorkspaceDownloadsView, { counts: { ...counts, downloading: 0, queued: 0 }, ready: [item({ state: "done" })], onDownloads: vi.fn(), onShow, onDismiss: vi.fn() }));
    expect(ready).toContain('data-testid="mast-dl-ready"');
    expect(ready).toContain(`${CAL} is downloaded and loaded in Files.`);
    expect(ready).toContain(">Show<");
  });

  it("offers Show in Workspace on the archive notice", () => {
    const html = renderToStaticMarkup(createElement(DownloadReadyView, { items: [item({ state: "done" })], showLabel: "Show in Workspace", placement: "float", onShow: vi.fn(), onDismiss: vi.fn() }));
    expect(html).toContain('data-placement="float"');
    expect(html).toContain(">Show in Workspace<");
  });
});
