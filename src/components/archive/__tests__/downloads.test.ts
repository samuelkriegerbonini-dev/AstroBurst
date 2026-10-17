import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

import type { AstroFile } from "../../../shared/types";
import type { MastDownloadItem, MastProduct } from "../../../shared/types/mast";
import { fileStore } from "../../../hooks/useFileStore";
import { registerFileIngest } from "../../../hooks/useFileIngest";
import {
  ACTION_LABELS,
  CONFIRM_ABOVE_BYTES,
  NOT_VERIFIED_TEXT,
  downloadInput,
  downloadStatusText,
  downloadsPillText,
  downloadsUnavailable,
  HLSP_NAME_TEXT,
  listDownloadRefusal,
  needsConfirmation,
  openableDownload,
  picksAfterSend,
  productDownloadRefusal,
  recoveryInput,
  selectionBytes,
} from "../logic/downloadView";
import { createDownloadsState, createDownloadsStore, downloadCounts, downloadsReducer, enqueueErrorOf, pendingOpens } from "../state/downloadsStore";
import { nextDownloadRequestId, openDownloaded } from "../state/downloadsController";
import { DownloadQueueView } from "../DownloadQueue";
import { DownloadBarView } from "../DownloadBar";
import { FooterStatusView } from "../ArchiveFooterStatus";

const CAL = "jw02739001001_02105_00001_nrca1_cal.fits";

function item(over: Partial<MastDownloadItem> = {}): MastDownloadItem {
  return {
    id: "dl1:0",
    requestId: "dl1",
    mission: "jwst",
    datasetId: "jw02739001001_02105_00001_nrca1",
    filename: CAL,
    program: "2739",
    uri: `mast:JWST/product/${CAL}`,
    sizeBytes: 117_573_120,
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
  };
}

function product(over: Partial<MastProduct> = {}): MastProduct {
  return {
    datasetId: "jw02739001001_02105_00001_nrca1",
    filename: CAL,
    sizeBytes: 117_573_120,
    category: "2b",
    type: "science",
    suffix: "cal",
    cls: "image",
    opensAs: "image",
    access: "PUBLIC",
    mastUri: `mast:JWST/product/${CAL}`,
    downloadUrl: `https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:JWST/product/${CAL}`,
    badges: [],
    ...over,
  };
}

describe("download confirmation", () => {
  it("asks only above 1 GB", () => {
    expect(CONFIRM_ABOVE_BYTES).toBe(1_000_000_000);
    expect(needsConfirmation(1_000_000_000)).toBe(false);
    expect(needsConfirmation(1_000_000_001)).toBe(true);
    expect(needsConfirmation(5_400_000_000)).toBe(true);
    expect(needsConfirmation(117_573_120)).toBe(false);
    expect(selectionBytes([product(), product({ filename: "b_cal.fits", sizeBytes: 900_000_000 })])).toBe(1_017_573_120);
  });

  it("shows the confirmation with the size before anything is sent", () => {
    const html = renderToStaticMarkup(
      createElement(DownloadBarView, { count: 1, bytes: 5_400_000_000, confirming: "open", error: null, onDownload: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }),
    );
    expect(html).toContain('data-testid="mast-dl-confirm"');
    expect(html).toContain("5.40 GB");
    expect(html).toContain("Cancel");
    const idle = renderToStaticMarkup(
      createElement(DownloadBarView, { count: 2, bytes: 235_146_240, confirming: null, error: null, onDownload: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }),
    );
    expect(idle).not.toContain("mast-dl-confirm");
    expect(idle).toContain("2 files selected, 235 MB");
    expect(idle).toContain("Download and open");
    const none = renderToStaticMarkup(
      createElement(DownloadBarView, { count: 0, bytes: 0, confirming: null, error: null, onDownload: vi.fn(), onConfirm: vi.fn(), onCancel: vi.fn() }),
    );
    expect(none.match(/disabled=""/g)).toHaveLength(2);
    const refused = renderToStaticMarkup(
      createElement(DownloadBarView, {
        count: 1,
        bytes: 1,
        confirming: null,
        error: { code: "mast_quota", message: "the MAST download quota (0.00 GB) would be exceeded", context: {} },
        onDownload: vi.fn(),
        onConfirm: vi.fn(),
        onCancel: vi.fn(),
      }),
    );
    expect(refused).toContain('role="alert"');
    expect(refused).toContain("quota");
  });

  it("offers no download for Roman products and says why", () => {
    expect(downloadsUnavailable("roman")).toBe("Roman downloads are not enabled yet.");
    expect(downloadsUnavailable("roman_cgi")).toBe("Roman downloads are not enabled yet.");
    expect(downloadsUnavailable("jwst")).toBeNull();
    expect(downloadsUnavailable("hst")).toBeNull();
    const html = renderToStaticMarkup(
      createElement(DownloadBarView, {
        count: 0,
        bytes: 0,
        confirming: null,
        error: null,
        unavailable: "Roman downloads are not enabled yet.",
        onDownload: vi.fn(),
        onConfirm: vi.fn(),
        onCancel: vi.fn(),
      }),
    );
    expect(html).toContain('data-state="unavailable"');
    expect(html).toContain("Roman downloads are not enabled yet.");
    expect(html).not.toContain("Download and open");
    expect(html).not.toContain("<button");
  });
});

describe("which downloads open in the workspace", () => {
  it("opens images, cubes, spectra and ramps in readable files only", () => {
    expect(openableDownload(item())).toBe(true);
    for (const opensAs of ["cube", "spectrum", "ramp"] as const) expect(openableDownload(item({ opensAs }))).toBe(true);
    expect(openableDownload(item({ opensAs: "catalog", filename: "jw02739-o001_t001_nircam_clear-f090w_segm.fits" }))).toBe(false);
    expect(openableDownload(item({ opensAs: "catalog", filename: "x_cat.ecsv" }))).toBe(false);
    expect(openableDownload(item({ opensAs: null }))).toBe(false);
    expect(openableDownload(item({ opensAs: "spectrum", filename: "hlsp_x_spec.fits.gz" }))).toBe(true);
    expect(openableDownload(item({ opensAs: "spectrum", filename: "hlsp_x_spec.tar.gz" }))).toBe(false);
    expect(openableDownload(item({ filename: "r0001_cal.asdf" }))).toBe(true);
  });

  it("loads a finished file into the workspace and switches to it", () => {
    const ingest = vi.fn(() => true);
    const setMode = vi.fn();
    expect(openDownloaded(item({ state: "done" }), { ingest, setMode, reload: vi.fn() })).toBe(true);
    expect(ingest).toHaveBeenCalledWith([{ name: CAL, path: `C:\\data\\mast\\JWST\\2739\\${CAL}`, size: 0 }]);
    expect(setMode).toHaveBeenCalledWith("workspace");
    expect(openDownloaded(item({ state: "downloading" }), { ingest, setMode, reload: vi.fn() })).toBe(false);
    expect(openDownloaded(item({ state: "done", opensAs: "catalog", filename: "x_cat.ecsv" }), { ingest, setMode, reload: vi.fn() })).toBe(false);
    expect(ingest).toHaveBeenCalledTimes(1);
    const notReady = vi.fn(() => false);
    expect(openDownloaded(item({ state: "done" }), { ingest: notReady, setMode, reload: vi.fn() })).toBe(false);
  });
});

describe("Open on a file the Files list already holds", () => {
  const PATH = item().path;
  let added: AstroFile[][];
  let unregister: () => void;

  beforeEach(() => {
    fileStore.reset();
    added = [];
    unregister = registerFileIngest((files) => {
      added.push(files);
      fileStore.addFiles(files);
    });
  });

  afterEach(() => unregister());

  function listed(path: string, outcome: "done" | "error"): string {
    fileStore.addFiles([{ name: path.split(/[\\/]/).pop() as string, path, size: 0 }]);
    const id = fileStore.getFileIds()[fileStore.getFileIds().length - 1];
    if (outcome === "done") fileStore.fileDone(id, null);
    else fileStore.fileError(id, "unreadable");
    return id;
  }

  it("selects the loaded row instead of adding the file a second time", () => {
    const loaded = listed(PATH.toLowerCase().replace(/\\/g, "/"), "done");
    const other = listed("C:\\data\\other_cal.fits", "done");
    fileStore.selectFile(other);
    expect(openDownloaded(item({ state: "done" }))).toBe(true);
    expect(added).toEqual([]);
    expect(fileStore.getFiles()).toHaveLength(2);
    expect(fileStore.getSelected()).toBe(loaded);
  });

  it("adds a file the list does not hold and loads a row that failed again in place", () => {
    expect(openDownloaded(item({ state: "done" }))).toBe(true);
    expect(added.map((files) => files.map((f) => f.path))).toEqual([[PATH]]);
    fileStore.reset();
    added = [];
    const failed = listed(PATH, "error");
    expect(openDownloaded(item({ state: "done" }))).toBe(true);
    expect(fileStore.getFiles()).toHaveLength(1);
    expect(fileStore.getFile(failed)?.status).toBe("queued");
  });
});

describe("downloads store", () => {
  it("keeps two in flight and the third queued as Rust reports them", () => {
    const snapshot = [item({ state: "downloading", received: 10 }), item({ id: "dl1:1", state: "downloading" }), item({ id: "dl1:2", state: "queued" })];
    let state = downloadsReducer(createDownloadsState(), { type: "snapshot", items: snapshot });
    expect(downloadCounts(state)).toMatchObject({ downloading: 2, queued: 1, done: 0, failed: 0, total: 3 });
    expect(downloadsPillText(downloadCounts(state))).toBe("Downloads: 2 running, 1 queued");
    state = downloadsReducer(state, { type: "item", item: item({ state: "done", received: 117_573_120, total: 117_573_120, verified: true }) });
    expect(downloadCounts(state)).toMatchObject({ downloading: 1, queued: 1, done: 1 });
    state = downloadsReducer(state, { type: "item", item: item({ id: "dl1:2", state: "downloading" }) });
    expect(downloadCounts(state)).toMatchObject({ downloading: 2, queued: 0, done: 1 });
    expect(state.items.map((i) => i.id)).toEqual(["dl1:0", "dl1:1", "dl1:2"]);
  });

  it("never lets an older snapshot or event move an item backwards", () => {
    let state = downloadsReducer(createDownloadsState(), { type: "item", item: item({ state: "done", received: 117_573_120 }) });
    state = downloadsReducer(state, { type: "snapshot", items: [item({ state: "queued" }), item({ id: "dl2:0", state: "queued" })] });
    expect(state.items.map((i) => [i.id, i.state])).toEqual([
      ["dl1:0", "done"],
      ["dl2:0", "queued"],
    ]);
    state = downloadsReducer(state, { type: "item", item: item({ id: "dl2:0", state: "downloading", received: 50 }) });
    state = downloadsReducer(state, { type: "item", item: item({ id: "dl2:0", state: "downloading", received: 20 }) });
    expect(state.items[1].received).toBe(50);
    state = downloadsReducer(state, { type: "snapshot", items: [item({ id: "dl2:0", state: "downloading", received: 60 })] });
    expect(state.items.map((i) => i.id)).toEqual(["dl2:0"]);
    expect(state.items[0].received).toBe(60);
  });

  it("opens each finished Download and open item once", () => {
    let state = downloadsReducer(createDownloadsState(), {
      type: "snapshot",
      items: [
        item({ open: true, state: "done" }),
        item({ id: "dl1:1", open: true, state: "downloading" }),
        item({ id: "dl1:2", open: false, state: "done" }),
        item({ id: "dl1:3", open: true, state: "done", opensAs: "catalog", filename: "x_cat.ecsv" }),
        item({ id: "dl1:4", open: true, state: "error" }),
      ],
    });
    expect(pendingOpens(state).map((i) => i.id)).toEqual(["dl1:0"]);
    state = downloadsReducer(state, { type: "settled", ids: ["dl1:0"] });
    expect(pendingOpens(state)).toEqual([]);
    state = downloadsReducer(state, { type: "item", item: item({ id: "dl1:1", open: true, state: "done" }) });
    expect(pendingOpens(state).map((i) => i.id)).toEqual(["dl1:1"]);
  });

  it("hands the refusal banner the same error object while progress events arrive", () => {
    const store = createDownloadsStore();
    store.dispatch({ type: "enqueueFailed", error: { code: "mast_quota", message: "over", context: {} } });
    const refusal = enqueueErrorOf(store.getState());
    const before = store.getState();
    store.dispatch({ type: "item", item: item({ state: "downloading", received: 10 }) });
    store.dispatch({ type: "item", item: item({ state: "downloading", received: 20 }) });
    expect(store.getState()).not.toBe(before);
    expect(refusal?.code).toBe("mast_quota");
    expect(enqueueErrorOf(store.getState())).toBe(refusal);
  });

  it("toggles the drawer and keeps the last refusal", () => {
    let state = downloadsReducer(createDownloadsState(), { type: "drawer", open: true });
    expect(state.drawerOpen).toBe(true);
    state = downloadsReducer(state, { type: "enqueueFailed", error: { code: "mast_quota", message: "over", context: {} } });
    expect(state.enqueueError?.code).toBe("mast_quota");
    state = downloadsReducer(state, { type: "enqueueFailed", error: null });
    expect(state.enqueueError).toBeNull();
  });
});

describe("download rows", () => {
  it("names the recovery action Rust chose", () => {
    expect(ACTION_LABELS).toEqual({ resume: "Resume", retry: "Retry", replace: "Replace", redownload: "Re-download" });
    const stale = item({
      state: "error",
      action: "redownload",
      error: { code: "mast_stale", message: "MAST has a newer version of this file", context: {} },
    });
    const html = renderToStaticMarkup(createElement(DownloadQueueView, { items: [stale], onCancel: vi.fn(), onAction: vi.fn(), onOpen: vi.fn(), onReveal: vi.fn(), onClose: vi.fn() }));
    expect(html).toContain('data-testid="mast-dl-item"');
    expect(html).toContain('data-state="error"');
    expect(html).toContain(">Re-download<");
    expect(html).toContain("MAST has a newer version of this file");
    const foreign = renderToStaticMarkup(
      createElement(DownloadQueueView, {
        items: [item({ state: "error", action: "replace", error: { code: "mast_bad_input", message: "filename: another file is already at this place", context: {} } })],
        onCancel: vi.fn(),
        onAction: vi.fn(),
        onOpen: vi.fn(),
        onReveal: vi.fn(),
        onClose: vi.fn(),
      }),
    );
    expect(foreign).toContain(">Replace<");
  });

  it("tags each failed row with its code and drops the field name from the text", () => {
    const mismatch = item({ state: "error", action: "retry", error: { code: "mast_size_mismatch", message: "MAST listed 10 bytes, the server sent 11", context: {} } });
    const foreign = item({
      id: "dl1:1",
      state: "error",
      action: "replace",
      error: { code: "mast_bad_input", message: "filename: another file, not downloaded by AstroBurst, is already at this place", context: {} },
    });
    expect(downloadStatusText(foreign)).toBe("Another file, not downloaded by AstroBurst, is already at this place");
    expect(downloadStatusText(item({ state: "error", action: "resume", error: { code: "mast_network", message: "connection refused.", context: {} } }))).toBe("Connection refused");
    const html = renderToStaticMarkup(
      createElement(DownloadQueueView, { items: [mismatch, foreign, item({ id: "dl1:2", state: "done" })], onCancel: vi.fn(), onAction: vi.fn(), onOpen: vi.fn(), onReveal: vi.fn(), onClose: vi.fn() }),
    );
    expect(html).toContain('data-code="mast_size_mismatch"');
    expect(html).toContain('data-code="mast_bad_input"');
    expect(html.match(/data-code=/g)).toHaveLength(2);
  });

  it("lists the newest request first, so a Re-download or a new failure is not below the fold", () => {
    const html = renderToStaticMarkup(
      createElement(DownloadQueueView, {
        items: [item({ id: "dl1:0", state: "done" }), item({ id: "dl1:1", state: "done" }), item({ id: "dl2:0", state: "error", action: "redownload", error: { code: "mast_stale", message: "MAST has a newer version", context: {} } })],
        onCancel: vi.fn(),
        onAction: vi.fn(),
        onOpen: vi.fn(),
        onReveal: vi.fn(),
        onClose: vi.fn(),
      }),
    );
    expect([...html.matchAll(/data-testid="mast-dl-item" data-id="([^"]+)"/g)].map((m) => m[1])).toEqual(["dl2:0", "dl1:1", "dl1:0"]);
  });

  it("shows a failed Cancel inside the drawer", () => {
    const html = renderToStaticMarkup(
      createElement(DownloadQueueView, {
        items: [item({ state: "downloading", received: 5, total: 10 })],
        cancelError: { code: "mast_decode", message: "the app did not answer", context: {} },
        onCancel: vi.fn(),
        onAction: vi.fn(),
        onOpen: vi.fn(),
        onReveal: vi.fn(),
        onClose: vi.fn(),
      }),
    );
    expect(html).toContain('data-testid="mast-dl-cancel-error"');
    expect(html).toContain("Cancel failed: the app did not answer.");
    expect(html).not.toContain("Download refused");
  });

  it("marks a copy MAST could not confirm and offers Open only for files the app reads", () => {
    const unverified = item({ state: "done", reused: true, verified: false, received: 117_573_120, total: 117_573_120 });
    expect(downloadStatusText(unverified)).toContain(NOT_VERIFIED_TEXT);
    expect(NOT_VERIFIED_TEXT).toBe("not verified against MAST");
    expect(downloadStatusText(item({ state: "done", reused: true, verified: true }))).toBe("Already downloaded, checked against MAST");
    expect(downloadStatusText(item({ state: "done", verified: true, resumedFrom: 0 }))).toBe("Downloaded, 118 MB");
    expect(downloadStatusText(item({ state: "downloading", received: 58_786_560, total: 117_573_120 }))).toBe("50% of 118 MB");
    expect(downloadStatusText(item({ state: "downloading", received: 0, total: null, sizeBytes: 0 }))).toBe("Starting");
    expect(downloadStatusText(item({ state: "queued" }))).toBe("Queued");
    expect(downloadStatusText(item({ state: "cancelled", action: "resume" }))).toBe("Cancelled");
    const html = renderToStaticMarkup(
      createElement(DownloadQueueView, {
        items: [unverified, item({ id: "dl1:1", state: "done", verified: true, opensAs: "catalog", filename: "x_cat.ecsv" }), item({ id: "dl1:2", state: "downloading", received: 5, total: 10 })],
        onCancel: vi.fn(),
        onAction: vi.fn(),
        onOpen: vi.fn(),
        onReveal: vi.fn(),
        onClose: vi.fn(),
      }),
    );
    expect(html).toContain(NOT_VERIFIED_TEXT);
    expect(html.match(/>Open</g)).toHaveLength(1);
    expect(html.match(/>Show in folder</g)).toHaveLength(2);
    expect(html.match(/>Cancel</g)).toHaveLength(1);
    expect(html).toContain('aria-valuenow="50"');
  });
});

describe("download requests", () => {
  it("builds the item from a product and the row programme, with the URI only for HLSP", () => {
    expect(downloadInput(product(), "2739")).toEqual({
      datasetId: "jw02739001001_02105_00001_nrca1",
      filename: CAL,
      program: "2739",
      sizeBytes: 117_573_120,
      uri: null,
      opensAs: "image",
      replace: false,
    });
    const ceers = product({
      datasetId: "ceers_nircam1",
      filename: "ceers_nircam1_f200w_v0.5_i2d.fits",
      mastUri: "mast:HLSP/ceers/nircam1/ceers_nircam1_f200w_v0.5_i2d.fits",
    });
    expect(downloadInput(ceers, null)).toMatchObject({ uri: "mast:HLSP/ceers/nircam1/ceers_nircam1_f200w_v0.5_i2d.fits", program: null });
    expect(downloadInput(product(), "  ").program).toBeNull();
  });

  it("re-sends a failed row with replace only for Replace and Re-download", () => {
    expect(recoveryInput(item({ state: "error", action: "redownload" }))).toMatchObject({ filename: CAL, program: "2739", replace: true, uri: `mast:JWST/product/${CAL}` });
    expect(recoveryInput(item({ state: "error", action: "replace" })).replace).toBe(true);
    expect(recoveryInput(item({ state: "cancelled", action: "resume" })).replace).toBe(false);
    expect(recoveryInput(item({ state: "error", action: "retry" })).replace).toBe(false);
  });

  it("keeps replace when a Replace or Re-download that stopped is resumed or retried", () => {
    expect(recoveryInput(item({ state: "cancelled", action: "resume", replace: true })).replace).toBe(true);
    expect(recoveryInput(item({ state: "error", action: "resume", replace: true })).replace).toBe(true);
    expect(recoveryInput(item({ state: "cancelled", action: "retry", replace: true })).replace).toBe(true);
    expect(recoveryInput(item({ state: "cancelled", action: "resume", replace: false })).replace).toBe(false);
  });

  it("clears the picks of the dataset a finished request came from, not the ones made since in another dataset", () => {
    const sent = { key: "jwst:a", ids: new Set(["a:x_cal.fits"]) };
    expect(picksAfterSend(sent, "jwst:a")).toEqual({ key: "jwst:a", ids: new Set() });
    const since = { key: "jwst:b", ids: new Set(["b:y_cal.fits"]) };
    expect(picksAfterSend(since, "jwst:a")).toBe(since);
  });

  it("refuses up front the files whose MAST URI the download engine refuses", () => {
    const jwst = { id: "jwst", label: "JWST" } as const;
    const hst = { id: "hst", label: "HST" } as const;
    const iue = { id: "iue", label: "IUE" } as const;
    const ceers = product({ mastUri: "mast:HLSP/ceers/nircam1/ceers_nircam1_f200w_v0.5_i2d.fits" });
    const classy = product({
      filename: "hlsp_classy_hst_cos_j0021+0052_g130m_v1_coadded.fits",
      mastUri: "mast:HLSP/classy/j0021+0052/hlsp_classy_hst_cos_j0021+0052_g130m_v1_coadded.fits",
    });
    const lwp = product({ filename: "lwp00501.lihi.gz", mastUri: "mast:IUE/url//pub/iue/data/lwp/00000/lwp00501.lihi.gz" });
    expect(productDownloadRefusal(product(), jwst)).toBeNull();
    expect(productDownloadRefusal(ceers, jwst)).toBeNull();
    expect(productDownloadRefusal(ceers, iue)).toBeNull();
    expect(productDownloadRefusal(classy, hst)).toBe(HLSP_NAME_TEXT);
    expect(productDownloadRefusal(lwp, iue)).toBe("IUE downloads are not enabled yet.");
    expect(productDownloadRefusal(product(), { id: "roman", label: "Roman" })).toBe("Roman downloads are not enabled yet.");
    expect(listDownloadRefusal([lwp, lwp], iue)).toBe("IUE downloads are not enabled yet.");
    expect(listDownloadRefusal([product(), classy], hst)).toBeNull();
    expect(listDownloadRefusal([], iue)).toBeNull();
    expect(listDownloadRefusal([product()], { id: "roman", label: "Roman" })).toBe("Roman downloads are not enabled yet.");
  });

  it("gives every request a fresh id the Rust validator accepts", () => {
    const ids = new Set(Array.from({ length: 50 }, () => nextDownloadRequestId(1_760_000_000_000)));
    expect(ids.size).toBe(50);
    for (const id of ids) expect(id).toMatch(/^[A-Za-z0-9:._-]{1,64}$/);
  });
});

describe("footer downloads pill", () => {
  it("counts running and queued downloads and toggles the drawer", () => {
    const onDownloads = vi.fn();
    const html = renderToStaticMarkup(
      createElement(FooterStatusView, {
        total: 0,
        thumbs: 0,
        queued: 0,
        romanUnlocked: false,
        processing: null,
        onRoman: vi.fn(),
        downloads: { downloading: 2, queued: 1, done: 0, failed: 0, total: 3 },
        drawerOpen: false,
        onDownloads,
      }),
    );
    const pill = /<button[^>]*data-testid="mast-downloads-pill"[^>]*>/.exec(html)?.[0] ?? "";
    expect(pill).toContain('data-busy="true"');
    expect(pill).toContain('data-downloading="2"');
    expect(pill).toContain('data-queued="1"');
    expect(pill).toContain('aria-expanded="false"');
    expect(html).toContain("Downloads: 2 running, 1 queued");
    const quiet = renderToStaticMarkup(createElement(FooterStatusView, { total: 0, thumbs: 0, queued: 0, romanUnlocked: false, processing: null, onRoman: vi.fn() }));
    expect(quiet).not.toContain("mast-downloads-pill");
    expect(downloadsPillText({ downloading: 0, queued: 0, done: 3, failed: 0, total: 3 })).toBe("Downloads: 3 done");
    expect(downloadsPillText({ downloading: 0, queued: 0, done: 1, failed: 2, total: 3 })).toBe("Downloads: 2 failed");
    expect(downloadsPillText({ downloading: 0, queued: 0, done: 0, failed: 0, total: 0 })).toBe("Downloads");
  });
});
