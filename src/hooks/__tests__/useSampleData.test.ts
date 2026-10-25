import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";

const { typedInvokeMock, isTauriMock, listenMock, unlistenMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  isTauriMock: vi.fn(() => true),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}));

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, isTauri: isTauriMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

import { hooks } from "../../test/hookRuntime";
import EmptyState from "../../components/EmptyState";
import { TauriCommandError } from "../../shared/types/errors";
import { updateConfig } from "../../services/config";
import type { SampleOpen, SampleProgress, SampleSetReport } from "../../services/sampleData";
import {
  PRIMARY_SAMPLE_SET,
  createSampleDataStore,
  sampleActions,
  sampleControls,
  sampleDataStore,
  startSampleData,
  useSampleData,
  type SampleDataApi,
  type SampleDataStore,
} from "../useSampleData";

const TOTAL = 30_896_640;
const FRAME = 10_298_880;
const ID = "hst-m16-wfpc2";
const FOLDER = "C:\\Users\\me\\AstroBurst\\samples\\hst-m16-wfpc2";

function report(over: Partial<SampleSetReport> = {}): SampleSetReport {
  return {
    id: ID,
    title: "HST/WFPC2 narrowband frames of M16 (Eagle Nebula)",
    credit: "HST/WFPC2 narrowband frames of the Eagle Nebula (M16): NASA, ESA, J. Hester and P. Scowen (Arizona State University), data via STScI",
    licence: "Public-domain Hubble archive data; credit requested by STScI",
    demonstrates: "Compose (SHO and HOO narrowband palettes), photometry and masks",
    total_bytes: TOTAL,
    file_count: 3,
    state: "absent",
    received_bytes: 0,
    folder: FOLDER,
    ...over,
  };
}

const OPENED: SampleOpen = {
  id: ID,
  paths: ["502nmos.fits", "656nmos.fits", "673nmos.fits"].map((name) => `${FOLDER}\\${name}`),
  credit: report().credit,
};

const IFU_ID = "jwst-gc-nirspec-ifu";
const IFU_CUBE = 174_983_040;
const IFU_TOTAL = 175_680_000;
const IFU_FOLDER = "C:\\Users\\me\\AstroBurst\\samples\\jwst-gc-nirspec-ifu";
const NOT_PUBLISHED = "jw01266-o005_t001_nirspec_g395h-f290lp_s3d.fits: not available on GitHub (HTTP 404); the sample release may not be published yet";

function ifuReport(over: Partial<SampleSetReport> = {}): SampleSetReport {
  return report({
    id: IFU_ID,
    title: "JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum",
    credit: "JWST/NIRSpec IFU cube and 1-D spectrum of the Galactic Center (GC-NIRSPEC-1): NASA, ESA, CSA, GTO programme 1266 (PI M. Garcia Marin), data via MAST at STScI",
    licence: "Public JWST archive data; acknowledgement requested by STScI",
    demonstrates: "Cube: spectra with error bands, moment maps, line fits and line measurement, next to the pipeline's 1-D spectrum",
    total_bytes: IFU_TOTAL,
    file_count: 2,
    folder: IFU_FOLDER,
    ...over,
  });
}

const IFU_OPENED: SampleOpen = {
  id: IFU_ID,
  paths: ["s3d", "x1d"].map((kind) => `${IFU_FOLDER}\\jw01266-o005_t001_nirspec_g395h-f290lp_${kind}.fits`),
  credit: ifuReport().credit,
};

function progress(received: number, file = "656nmos.fits", id = ID): SampleProgress {
  return { id, received_bytes: received, total_bytes: TOTAL, file };
}

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (err: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakeApi(sets: SampleSetReport[]) {
  const api = {
    list: vi.fn(async () => sets.map((s) => ({ ...s }))),
    download: vi.fn<(id: string) => Promise<SampleOpen>>(async () => OPENED),
    cancel: vi.fn(async () => true),
    open: vi.fn<(id: string) => Promise<SampleOpen>>(async () => OPENED),
  } satisfies SampleDataApi;
  return api;
}

function controlsOf(store: SampleDataStore, onOpen: (opened: SampleOpen) => void) {
  const view = sampleControls(store.get().find((e) => e.id === ID), sampleActions(store, ID, onOpen));
  if (!view) throw new Error("expected the sample set to be listed");
  return view;
}

function emptyState(store: SampleDataStore, onOpen: (opened: SampleOpen) => void) {
  return renderToStaticMarkup(createElement(EmptyState, { onBrowseFiles: () => {}, onSelectFolder: () => {}, sample: controlsOf(store, onOpen) }));
}

function mainButton(html: string): string {
  const tag = /<button[^>]*data-testid="empty-open-sample"[^>]*>/.exec(html)?.[0];
  if (!tag) return "";
  const at = html.indexOf(tag);
  return html.slice(at, html.indexOf("</button>", at));
}

function SampleDataProbe({ onOpen }: { onOpen: (opened: SampleOpen) => void }) {
  useSampleData(onOpen);
  return null;
}

function mountSampleData(onOpen: (opened: SampleOpen) => void): () => void {
  hooks.mount();
  hooks.render(SampleDataProbe, { onOpen });
  return () => hooks.mount();
}

let consoleError: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  consoleError.mockRestore();
  vi.useRealTimers();
});

describe("sample data store", () => {
  it("lists the sets and exposes the primary M16 set as absent before any download", async () => {
    const store = createSampleDataStore(fakeApi([report()]));
    expect(store.get()).toEqual([]);
    await store.refresh();
    const view = controlsOf(store, vi.fn());
    expect(PRIMARY_SAMPLE_SET).toBe(ID);
    expect(view).toMatchObject({ id: ID, state: "absent", receivedBytes: 0, totalBytes: TOTAL, error: null, credit: report().credit });
  });

  it("notifies subscribers on every change", async () => {
    const store = createSampleDataStore(fakeApi([report()]));
    const cb = vi.fn();
    const off = store.subscribe(cb);
    await store.refresh();
    expect(cb).toHaveBeenCalledTimes(1);
    off();
    await store.refresh();
    expect(cb).toHaveBeenCalledTimes(1);
  });

  it("downloads, moves the bar with the progress events and opens the frames once the set is complete", async () => {
    const api = fakeApi([report()]);
    const pending = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(pending.promise);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();

    const run = controlsOf(store, onOpen).download();
    expect(api.download).toHaveBeenCalledWith(ID);
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", error: null });

    store.progress(progress(FRAME, "656nmos.fits"));
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", receivedBytes: FRAME, file: "656nmos.fits" });
    const html = emptyState(store, onOpen);
    expect(html).toMatch(/data-testid="empty-sample-progress"[^>]*aria-valuenow="33"|aria-valuenow="33"[^>]*data-testid="empty-sample-progress"/);
    expect(html).toContain("33%");
    expect(onOpen).not.toHaveBeenCalled();

    store.progress(progress(TOTAL, "673nmos.fits"));
    expect(emptyState(store, onOpen)).toContain("100%");

    api.list.mockResolvedValueOnce([report({ state: "ready", received_bytes: TOTAL })]);
    pending.resolve(OPENED);
    await run;
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onOpen).toHaveBeenCalledWith(OPENED);
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "ready", receivedBytes: TOTAL, error: null });
    expect(mainButton(emptyState(store, onOpen))).toContain("Open sample data");
  });

  it("ignores progress for another set and progress that arrives when no download runs", async () => {
    const store = createSampleDataStore(fakeApi([report(), report({ id: "jwst-nircam-m16", state: "ready", received_bytes: TOTAL })]));
    await store.refresh();
    const before = store.get();
    store.progress(progress(FRAME));
    store.progress(progress(FRAME, "x.fits", "jwst-nircam-m16"));
    store.progress(progress(FRAME, "x.fits", "unknown"));
    expect(store.get()).toBe(before);
  });

  it("runs the JWST IFU set and the HST set side by side, each moved and opened by its own id", async () => {
    const api = fakeApi([ifuReport(), report()]);
    const ifuRun = deferred<SampleOpen>();
    const hstRun = deferred<SampleOpen>();
    api.download.mockImplementation((id: string) => (id === IFU_ID ? ifuRun.promise : hstRun.promise));
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();
    const ifu = () => sampleControls(store.get().find((e) => e.id === IFU_ID), sampleActions(store, IFU_ID, onOpen));

    const ifuDone = ifu()!.download();
    const hstDone = controlsOf(store, onOpen).download();
    expect(api.download.mock.calls).toEqual([[IFU_ID], [ID]]);

    store.progress({ id: IFU_ID, received_bytes: IFU_CUBE, total_bytes: IFU_TOTAL, file: "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits" });
    store.progress(progress(FRAME));
    expect(ifu()).toMatchObject({ state: "downloading", receivedBytes: IFU_CUBE, totalBytes: IFU_TOTAL, file: "jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits" });
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", receivedBytes: FRAME, totalBytes: TOTAL, file: "656nmos.fits" });
    expect(mainButton(emptyState(store, onOpen))).toBe("");
    expect(emptyState(store, onOpen)).toContain("33%");

    api.list.mockResolvedValueOnce([ifuReport({ state: "ready", received_bytes: IFU_TOTAL }), report({ state: "downloading" })]);
    ifuRun.resolve(IFU_OPENED);
    await ifuDone;
    expect(onOpen.mock.calls).toEqual([[IFU_OPENED]]);
    expect(ifu()).toMatchObject({ state: "ready", receivedBytes: IFU_TOTAL });
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", receivedBytes: FRAME });

    api.list.mockResolvedValueOnce([ifuReport({ state: "ready", received_bytes: IFU_TOTAL }), report({ state: "partial", received_bytes: FRAME })]);
    await controlsOf(store, onOpen).cancel();
    expect(api.cancel).toHaveBeenCalledWith(ID);
    hstRun.reject(new TauriCommandError("sample_download_cmd", "cancelled"));
    await hstDone;
    expect(api.cancel).not.toHaveBeenCalledWith(IFU_ID);
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "partial", receivedBytes: FRAME, error: null });
    expect(ifu()).toMatchObject({ state: "ready", error: null });
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("keeps a missing release asset error on the JWST set only, with Retry still offered", async () => {
    const api = fakeApi([ifuReport(), report()]);
    api.download.mockRejectedValueOnce(new TauriCommandError("sample_download_cmd", NOT_PUBLISHED));
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();
    const ifu = () => sampleControls(store.get().find((e) => e.id === IFU_ID), sampleActions(store, IFU_ID, onOpen));
    api.list.mockResolvedValueOnce([ifuReport(), report()]);
    await ifu()!.download();
    expect(ifu()).toMatchObject({ state: "absent", receivedBytes: 0, error: NOT_PUBLISHED });
    expect(onOpen).not.toHaveBeenCalled();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "absent", error: null });
    expect(emptyState(store, onOpen)).not.toContain('data-testid="empty-sample-error"');

    api.list.mockResolvedValueOnce([ifuReport({ state: "ready", received_bytes: IFU_TOTAL }), report()]);
    api.download.mockResolvedValueOnce(IFU_OPENED);
    await ifu()!.download();
    expect(ifu()).toMatchObject({ state: "ready", receivedBytes: IFU_TOTAL, error: null });
    expect(onOpen.mock.calls).toEqual([[IFU_OPENED]]);
  });

  it("goes back to partial with no error when the download is cancelled", async () => {
    const api = fakeApi([report()]);
    const pending = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(pending.promise);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();

    const run = controlsOf(store, onOpen).download();
    store.progress(progress(FRAME));
    api.list.mockResolvedValueOnce([report({ state: "partial", received_bytes: FRAME })]);
    await controlsOf(store, onOpen).cancel();
    expect(api.cancel).toHaveBeenCalledWith(ID);
    pending.reject(new TauriCommandError("sample_download_cmd", "cancelled"));
    await run;

    expect(onOpen).not.toHaveBeenCalled();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "partial", receivedBytes: FRAME, error: null });
    const html = emptyState(store, onOpen);
    expect(mainButton(html)).toContain("Resume sample download (10 MB of 31 MB)");
    expect(html).not.toContain('data-testid="empty-sample-error"');
    expect(consoleError).not.toHaveBeenCalled();
  });

  it("does not open the frames when Cancel was pressed as the last byte landed, and a later Open still works", async () => {
    const api = fakeApi([report()]);
    const pending = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(pending.promise);
    api.cancel.mockResolvedValueOnce(false);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();

    const run = controlsOf(store, onOpen).download();
    store.progress(progress(TOTAL, "673nmos.fits"));
    await controlsOf(store, onOpen).cancel();
    expect(api.cancel).toHaveBeenCalledWith(ID);
    api.list.mockResolvedValueOnce([report({ state: "ready", received_bytes: TOTAL })]);
    pending.resolve(OPENED);
    await run;

    expect(onOpen).not.toHaveBeenCalled();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "ready", error: null });
    await controlsOf(store, onOpen).open();
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("opens the frames of a download started after a cancelled one", async () => {
    const api = fakeApi([report()]);
    const first = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(first.promise);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();

    const run = controlsOf(store, onOpen).download();
    await controlsOf(store, onOpen).cancel();
    first.reject(new TauriCommandError("sample_download_cmd", "cancelled"));
    await run;
    await controlsOf(store, onOpen).download();
    expect(onOpen).toHaveBeenCalledWith(OPENED);
  });

  it("does not ask the backend to cancel when nothing runs", async () => {
    const api = fakeApi([report()]);
    const store = createSampleDataStore(api);
    await store.refresh();
    await controlsOf(store, vi.fn()).cancel();
    expect(api.cancel).not.toHaveBeenCalled();
  });

  it("shows the failure with Retry, and Retry resumes the set and opens it", async () => {
    const api = fakeApi([report()]);
    api.download.mockRejectedValueOnce(new TauriCommandError("sample_download_cmd", "656nmos.fits: the download did not match the published checksum"));
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();

    api.list.mockResolvedValueOnce([report({ state: "partial", received_bytes: FRAME })]);
    await controlsOf(store, onOpen).download();
    expect(onOpen).not.toHaveBeenCalled();
    expect(controlsOf(store, onOpen)).toMatchObject({
      state: "partial",
      receivedBytes: FRAME,
      error: "656nmos.fits: the download did not match the published checksum",
    });
    expect(consoleError).toHaveBeenCalledWith("[AstroBurst] Sample data download:", expect.any(TauriCommandError));
    const html = emptyState(store, onOpen);
    expect(html).toMatch(/<p[^>]*data-testid="empty-sample-error"[^>]*>656nmos.fits: the download did not match the published checksum<\/p>/);
    expect(mainButton(html)).toContain("Retry sample download");

    api.list.mockResolvedValueOnce([report({ state: "ready", received_bytes: TOTAL })]);
    await controlsOf(store, onOpen).download();
    expect(api.download).toHaveBeenCalledTimes(2);
    expect(onOpen).toHaveBeenCalledWith(OPENED);
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "ready", error: null });
    expect(emptyState(store, onOpen)).not.toContain('data-testid="empty-sample-error"');
  });

  it("falls back to a local state when the status cannot be read after a failure", async () => {
    const api = fakeApi([report()]);
    const pending = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(pending.promise);
    const store = createSampleDataStore(api);
    await store.refresh();

    const run = controlsOf(store, vi.fn()).download();
    store.progress(progress(FRAME));
    api.list.mockRejectedValueOnce(new TauriCommandError("sample_sets_cmd", "data folder unavailable"));
    pending.reject(new TauriCommandError("sample_download_cmd", "502nmos.fits: the server answered HTTP 503"));
    await run;
    expect(controlsOf(store, vi.fn())).toMatchObject({ state: "partial", error: "502nmos.fits: the server answered HTTP 503" });
  });

  it("starts one download per set and keeps it running when a refresh lands during it", async () => {
    const api = fakeApi([report()]);
    const pending = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(pending.promise);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();

    const run = controlsOf(store, onOpen).download();
    store.progress(progress(FRAME));
    await controlsOf(store, onOpen).download();
    expect(api.download).toHaveBeenCalledTimes(1);

    api.list.mockResolvedValueOnce([report({ state: "partial", received_bytes: 4_000 })]);
    await store.refresh();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", receivedBytes: FRAME });

    pending.resolve(OPENED);
    await run;
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("opens a kept set without downloading anything", async () => {
    const api = fakeApi([report({ state: "ready", received_bytes: TOTAL })]);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();
    await controlsOf(store, onOpen).open();
    expect(api.open).toHaveBeenCalledWith(ID);
    expect(api.download).not.toHaveBeenCalled();
    expect(onOpen).toHaveBeenCalledWith(OPENED);
  });

  it("reports an open that finds the frames gone and refreshes the status", async () => {
    const api = fakeApi([report({ state: "ready", received_bytes: TOTAL })]);
    api.open.mockRejectedValueOnce(new TauriCommandError("sample_open_cmd", "not downloaded"));
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();
    api.list.mockResolvedValueOnce([report()]);
    await controlsOf(store, onOpen).open();
    expect(onOpen).not.toHaveBeenCalled();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "absent", error: "The sample frames are no longer in the data folder" });
    expect(mainButton(emptyState(store, onOpen))).toContain("Retry sample download");
  });

  it("keeps Open on a kept set whose open failed, next to the error", async () => {
    const api = fakeApi([report({ state: "ready", received_bytes: TOTAL })]);
    api.open.mockRejectedValueOnce(new TauriCommandError("sample_open_cmd", "the Data folder setting could not be read: x"));
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();
    await controlsOf(store, onOpen).open();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "ready", error: "the Data folder setting could not be read: x" });
    const html = emptyState(store, onOpen);
    expect(mainButton(html)).toContain("Open sample data");
    expect(html).toMatch(/data-testid="empty-sample-error"[^>]*>the Data folder setting could not be read: x</);
  });
});

describe("a download this window did not start", () => {
  it("cancels it in the backend, follows it until it stops and never starts a second one", async () => {
    vi.useFakeTimers();
    const api = fakeApi([report({ state: "downloading", received_bytes: FRAME })]);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", receivedBytes: FRAME });

    await controlsOf(store, onOpen).download();
    expect(api.download).not.toHaveBeenCalled();
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", error: null });

    await controlsOf(store, onOpen).cancel();
    expect(api.cancel).toHaveBeenCalledWith(ID);
    api.list.mockResolvedValue([report({ state: "partial", received_bytes: FRAME })]);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "partial", receivedBytes: FRAME, error: null });
    expect(mainButton(emptyState(store, onOpen))).toContain("Resume sample download (10 MB of 31 MB)");

    const listed = api.list.mock.calls.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(api.list).toHaveBeenCalledTimes(listed);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("moves the bar with its progress and offers Open once it finishes, without opening the frames", async () => {
    vi.useFakeTimers();
    const api = fakeApi([report({ state: "downloading", received_bytes: FRAME })]);
    const store = createSampleDataStore(api);
    await store.refresh();
    const onOpen = vi.fn();

    store.progress(progress(TOTAL, "673nmos.fits"));
    expect(controlsOf(store, onOpen)).toMatchObject({ state: "downloading", receivedBytes: TOTAL });
    api.list.mockResolvedValue([report({ state: "ready", received_bytes: TOTAL })]);
    await vi.advanceTimersByTimeAsync(1_000);

    expect(controlsOf(store, onOpen)).toMatchObject({ state: "ready", receivedBytes: TOTAL, error: null });
    expect(mainButton(emptyState(store, onOpen))).toContain("Open sample data");
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("keeps following it after one status read fails", async () => {
    vi.useFakeTimers();
    const api = fakeApi([report({ state: "downloading", received_bytes: FRAME })]);
    const store = createSampleDataStore(api);
    await store.refresh();
    api.list.mockRejectedValueOnce(new TauriCommandError("sample_sets_cmd", "busy"));
    await vi.advanceTimersByTimeAsync(1_000);
    expect(controlsOf(store, vi.fn())).toMatchObject({ state: "downloading" });

    api.list.mockResolvedValue([report({ state: "ready", received_bytes: TOTAL })]);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(controlsOf(store, vi.fn())).toMatchObject({ state: "ready", receivedBytes: TOTAL });
  });

  it("keeps the newest status when an older read answers last", async () => {
    const api = fakeApi([report()]);
    const older = deferred<SampleSetReport[]>();
    const newer = deferred<SampleSetReport[]>();
    api.list.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    const store = createSampleDataStore(api);
    const first = store.refresh();
    const second = store.refresh();
    newer.resolve([report({ state: "ready", received_bytes: TOTAL })]);
    await second;
    older.resolve([report({ state: "downloading", received_bytes: FRAME })]);
    await first;
    expect(controlsOf(store, vi.fn())).toMatchObject({ state: "ready", receivedBytes: TOTAL });
  });

  it("does not poll the status for a download this window runs", async () => {
    vi.useFakeTimers();
    const api = fakeApi([report()]);
    const pending = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(pending.promise);
    const store = createSampleDataStore(api);
    await store.refresh();

    const run = controlsOf(store, vi.fn()).download();
    api.list.mockResolvedValue([report({ state: "downloading", received_bytes: FRAME })]);
    await store.refresh();
    const listed = api.list.mock.calls.length;
    await vi.advanceTimersByTimeAsync(10_000);
    expect(api.list).toHaveBeenCalledTimes(listed);

    api.list.mockResolvedValue([report({ state: "ready", received_bytes: TOTAL })]);
    pending.resolve(OPENED);
    await run;
  });
});

describe("startSampleData", () => {
  beforeEach(() => {
    listenMock.mockReset();
    unlistenMock.mockReset();
  });

  it("loads the sets, follows sample:download into the store and unlistens on cleanup", async () => {
    let handler: ((e: { payload: SampleProgress }) => void) | null = null;
    listenMock.mockImplementation(async (_name: string, cb: (e: { payload: SampleProgress }) => void) => {
      handler = cb;
      return unlistenMock;
    });
    const api = fakeApi([report()]);
    const pending = deferred<SampleOpen>();
    api.download.mockReturnValueOnce(pending.promise);
    const store = createSampleDataStore(api);

    const stop = startSampleData(store);
    await vi.waitFor(() => expect(store.get()).toHaveLength(1));
    expect(api.list).toHaveBeenCalledTimes(1);
    expect(listenMock).toHaveBeenCalledWith("sample:download", expect.any(Function));

    const run = controlsOf(store, vi.fn()).download();
    handler!({ payload: progress(FRAME) });
    expect(controlsOf(store, vi.fn()).receivedBytes).toBe(FRAME);

    stop();
    await vi.waitFor(() => expect(unlistenMock).toHaveBeenCalledTimes(1));
    pending.resolve(OPENED);
    await run;
  });

  it("reads the sets again once the Data folder setting is saved, and only while started", async () => {
    listenMock.mockResolvedValue(unlistenMock);
    typedInvokeMock.mockReset().mockResolvedValue({});
    const api = fakeApi([report({ state: "ready", received_bytes: TOTAL })]);
    const store = createSampleDataStore(api);
    const stop = startSampleData(store);
    await vi.waitFor(() => expect(api.list).toHaveBeenCalledTimes(1));
    api.list.mockResolvedValueOnce([report()]);

    await updateConfig("data_dir_override", "D:/AstroBurst");
    await vi.waitFor(() => expect(api.list).toHaveBeenCalledTimes(2));
    expect(controlsOf(store, vi.fn())).toMatchObject({ state: "absent" });

    await updateConfig("mast_quota_gb", 10);
    stop();
    await updateConfig("data_dir_override", null);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(api.list).toHaveBeenCalledTimes(2);
  });
});

describe("useSampleData", () => {
  function Probe({ onOpen }: { onOpen: (opened: SampleOpen) => void }) {
    const sample = useSampleData(onOpen);
    return createElement(EmptyState, { onBrowseFiles: () => {}, onSelectFolder: () => {}, sample: sample ?? undefined });
  }

  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockImplementation(async (command: string) => (command === "sample_sets_cmd" ? [report()] : null));
  });

  it("offers nothing outside the desktop app", async () => {
    isTauriMock.mockReturnValue(false);
    await sampleDataStore.refresh();
    const html = renderToStaticMarkup(createElement(Probe, { onOpen: vi.fn() }));
    expect(html).not.toContain('data-testid="empty-open-sample"');
    isTauriMock.mockReturnValue(true);
  });

  it("drives the empty state from the listed primary set in the desktop app", async () => {
    isTauriMock.mockReturnValue(true);
    await sampleDataStore.refresh();
    expect(typedInvokeMock).toHaveBeenCalledWith("sample_sets_cmd", {});
    const html = renderToStaticMarkup(createElement(Probe, { onOpen: vi.fn() }));
    expect(mainButton(html)).toContain("Download sample data (31 MB)");
  });

  it("keeps the empty state on the HST set by its id when a kept JWST set is listed first", async () => {
    isTauriMock.mockReturnValue(true);
    typedInvokeMock.mockImplementation(async (command: string) =>
      command === "sample_sets_cmd" ? [ifuReport({ state: "ready", received_bytes: IFU_TOTAL }), report()] : null,
    );
    await sampleDataStore.refresh();
    expect(sampleDataStore.get().map((e) => e.id)).toEqual([IFU_ID, ID]);
    const html = renderToStaticMarkup(createElement(Probe, { onOpen: vi.fn() }));
    expect(mainButton(html)).toContain("Download sample data (31 MB)");
    expect(html).not.toContain("Open sample data");
    expect(html).toContain(report().credit);
    expect(html).not.toContain(ifuReport().credit);
  });
});

describe("useSampleData on mount", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockImplementation(async (command: string) => (command === "sample_sets_cmd" ? [report()] : null));
    listenMock.mockReset();
    unlistenMock.mockReset();
    listenMock.mockResolvedValue(unlistenMock);
  });

  it("loads the sets and follows sample:download in the desktop app, and stops following on unmount", async () => {
    isTauriMock.mockReturnValue(true);
    const unmount = mountSampleData(vi.fn());
    expect(typedInvokeMock).toHaveBeenCalledWith("sample_sets_cmd", {});
    await vi.waitFor(() => expect(listenMock).toHaveBeenCalledWith("sample:download", expect.any(Function)));
    unmount();
    await vi.waitFor(() => expect(unlistenMock).toHaveBeenCalledTimes(1));
  });

  it("asks nothing of the backend outside the desktop app", async () => {
    isTauriMock.mockReturnValue(false);
    const unmount = mountSampleData(vi.fn());
    unmount();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(typedInvokeMock).not.toHaveBeenCalled();
    expect(listenMock).not.toHaveBeenCalled();
    isTauriMock.mockReturnValue(true);
  });
});
