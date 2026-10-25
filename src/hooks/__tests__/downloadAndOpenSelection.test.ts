import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { AstroFile } from "../../shared/types";
import type { MastDownloadItem } from "../../shared/types/mast";

const { snapshotMock, listenMock, enqueueMock } = vi.hoisted(() => ({ snapshotMock: vi.fn(), listenMock: vi.fn(), enqueueMock: vi.fn() }));

vi.mock("../../services/mast", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../services/mast")>();
  return { ...actual, mastDownloads: snapshotMock, onMastDownload: listenMock, mastDownload: enqueueMock };
});

import { fileStore } from "../useFileStore";
import { followFilteredSelection, ingestUnlessLoaded, registerFileIngest, showWorkspaceUnlessKept } from "../useFileIngest";
import { appModeStore } from "../useAppMode";
import { matchesActiveFilters, processedFilterable, productFilterStore } from "../useProductFilter";
import { fallbackSelectionId } from "../../utils/fileSelection";
import { FILE_STATUS } from "../../utils/constants";
import { createDownloadsStore } from "../../components/archive/state/downloadsStore";
import { openDownloaded, startDownloads, watchDownloads } from "../../components/archive/state/downloadsController";

const CAL = "C:\\data\\mast\\JWST\\2739\\jw02739001001_02105_00001_nrca1_cal.fits";
const NEW = "C:\\data\\mast\\JWST\\2739\\jw54000-o001_t001_nircam_xc1_i2d.fits";
const X1D = "C:\\data\\mast\\JWST\\2739\\jw02739-o001_t001_nirspec_x1d.fits";
const OTHER = "C:\\data\\other_i2d.fits";
const IMAGE = { previewUrl: "asset://a.png", dimensions: [256, 256] } as never;

function item(path: string, over: Partial<MastDownloadItem> = {}): MastDownloadItem {
  const filename = path.split("\\").pop() as string;
  return {
    id: "dl1:0",
    requestId: "dl1",
    mission: "jwst",
    datasetId: "jw54000-o001_t001_nircam",
    filename,
    program: "2739",
    uri: `mast:JWST/product/${filename}`,
    sizeBytes: 267_840,
    opensAs: "image",
    open: true,
    replace: false,
    state: "done",
    received: 267_840,
    total: 267_840,
    path,
    resumedFrom: 0,
    reused: false,
    verified: true,
    action: null,
    error: null,
    ...over,
  };
}

function rowOf(path: string): string {
  const row = fileStore.getFiles().find((f) => f.path === path);
  if (!row) throw new Error(`no row for ${path}`);
  return row.id;
}

function loadedAndSelected(path: string): string {
  fileStore.addFiles([{ name: path.split("\\").pop() as string, path, size: 0 }]);
  const id = rowOf(path);
  fileStore.fileDone(id, IMAGE);
  fileStore.selectFile(id);
  return id;
}

describe("Download and open of a file the Files list does not hold yet", () => {
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

  it("shows the opened file once it has loaded, although another file was shown", () => {
    const shown = loadedAndSelected(CAL);
    expect(openDownloaded(item(NEW))).toBe(true);
    const opened = rowOf(NEW);
    expect(fileStore.getSelected()).toBe(shown);

    fileStore.fileStarted(opened);
    fileStore.fileDone(opened, IMAGE);

    expect(fileStore.getSelected()).toBe(opened);
  });

  it("shows an opened spectrum once it settles as a spectrum", () => {
    loadedAndSelected(CAL);
    expect(openDownloaded(item(X1D, { opensAs: "spectrum" }))).toBe(true);
    const opened = rowOf(X1D);

    fileStore.fileSpectrum(opened, { kind: "x1d", hdu: 1 });

    expect(fileStore.getSelected()).toBe(opened);
  });

  it("keeps a file the user picked while the opened file was still loading", () => {
    loadedAndSelected(CAL);
    const picked = loadedAndSelected("C:\\data\\other_i2d.fits");
    fileStore.selectFile(rowOf(CAL));
    expect(openDownloaded(item(NEW))).toBe(true);
    fileStore.selectFile(picked);

    fileStore.fileDone(rowOf(NEW), IMAGE);

    expect(fileStore.getSelected()).toBe(picked);
  });

  it("leaves the selection alone when the opened file fails, also when it loads later without Open", () => {
    const shown = loadedAndSelected(CAL);
    expect(openDownloaded(item(NEW))).toBe(true);
    const opened = rowOf(NEW);
    fileStore.fileError(opened, "unreadable");
    expect(fileStore.getSelected()).toBe(shown);

    expect(ingestUnlessLoaded(NEW)).toBe(true);
    fileStore.fileDone(opened, IMAGE);

    expect(fileStore.getSelected()).toBe(shown);
  });

  it("keeps the selection for a file loaded without Open, as a plain Download retry or a PixelMath output", () => {
    const shown = loadedAndSelected(CAL);
    expect(ingestUnlessLoaded(NEW)).toBe(true);

    fileStore.fileDone(rowOf(NEW), IMAGE);

    expect(fileStore.getSelected()).toBe(shown);
    expect(added.map((files) => files.map((f) => f.path))).toEqual([[NEW]]);
  });

  it("forgets a pending open when the Files list is reset for a new batch", () => {
    loadedAndSelected(CAL);
    expect(openDownloaded(item(NEW))).toBe(true);

    fileStore.reset();
    fileStore.addFiles([
      { name: "other_i2d.fits", path: OTHER, size: 0 },
      { name: "xc1_i2d.fits", path: NEW, size: 0 },
    ]);
    fileStore.fileDone(rowOf(OTHER), IMAGE);
    fileStore.fileDone(rowOf(NEW), IMAGE);

    expect(fileStore.getSelected()).toBe(rowOf(OTHER));
  });

  it("still selects only the first loaded file of a batch", () => {
    fileStore.addFiles([
      { name: "a_i2d.fits", path: "C:\\d\\a_i2d.fits", size: 0 },
      { name: "b_i2d.fits", path: "C:\\d\\b_i2d.fits", size: 0 },
    ]);
    const [a, b] = fileStore.getFileIds();
    fileStore.fileDone(a, IMAGE);
    fileStore.fileDone(b, IMAGE);

    expect(fileStore.getSelected()).toBe(a);
  });
});

describe("Download and open of several products in one request", () => {
  const store = createDownloadsStore();
  let push: (next: MastDownloadItem) => void = () => {};
  let unregister: () => void;
  const batchFile = (request: string, n: number) => `C:\\data\\mast\\JWST\\2739\\${request}_${n}_i2d.fits`;
  const batchItem = (request: string, n: number) => item(batchFile(request, n), { id: `${request}:${n}`, requestId: request });

  beforeAll(async () => {
    snapshotMock.mockResolvedValue([]);
    listenMock.mockImplementation(async (cb: (next: MastDownloadItem) => void) => {
      push = cb;
      return () => {};
    });
    await watchDownloads(store);
  });

  beforeEach(() => {
    fileStore.reset();
    unregister = registerFileIngest((files) => fileStore.addFiles(files));
  });

  afterEach(() => {
    unregister();
    appModeStore.set("workspace");
  });

  it("shows the first file that loads and keeps it in the viewer while the rest of the request loads", () => {
    loadedAndSelected(CAL);
    push(batchItem("b1", 0));
    fileStore.fileDone(rowOf(batchFile("b1", 0)), IMAGE);
    expect(fileStore.getSelected()).toBe(rowOf(batchFile("b1", 0)));

    push(batchItem("b1", 1));
    push(batchItem("b1", 2));
    fileStore.fileDone(rowOf(batchFile("b1", 1)), IMAGE);
    fileStore.fileDone(rowOf(batchFile("b1", 2)), IMAGE);

    expect(fileStore.getSelected()).toBe(rowOf(batchFile("b1", 0)));
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([CAL, batchFile("b1", 0), batchFile("b1", 1), batchFile("b1", 2)]);
  });

  it("keeps a file the user picked after the first file of the request was shown", () => {
    const picked = loadedAndSelected(CAL);
    push(batchItem("b2", 0));
    fileStore.fileDone(rowOf(batchFile("b2", 0)), IMAGE);
    fileStore.selectFile(picked);

    push(batchItem("b2", 1));
    fileStore.fileDone(rowOf(batchFile("b2", 1)), IMAGE);

    expect(fileStore.getSelected()).toBe(picked);
  });

  it("leaves the app in the Archive when the later files of the request load after the user went back to it", () => {
    unregister();
    unregister = registerFileIngest((files, options) => {
      showWorkspaceUnlessKept(options);
      fileStore.addFiles(files);
    });
    appModeStore.set("archive");
    push(batchItem("b5", 0));
    expect(appModeStore.get()).toBe("workspace");
    fileStore.fileDone(rowOf(batchFile("b5", 0)), IMAGE);
    appModeStore.set("archive");

    push(batchItem("b5", 1));
    push(batchItem("b5", 2));

    expect(appModeStore.get()).toBe("archive");
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([batchFile("b5", 0), batchFile("b5", 1), batchFile("b5", 2)]);
    expect(fileStore.getSelected()).toBe(rowOf(batchFile("b5", 0)));
  });

  it("leaves the app in the Archive when a later file of the request replaces a row that failed to load", () => {
    unregister();
    unregister = registerFileIngest((files, options) => {
      showWorkspaceUnlessKept(options);
      fileStore.addFiles(files);
    });
    fileStore.addFiles([{ name: "b6_1_i2d.fits", path: batchFile("b6", 1), size: 0 }]);
    fileStore.fileError(rowOf(batchFile("b6", 1)), "truncated");
    appModeStore.set("archive");
    push(batchItem("b6", 0));
    fileStore.fileDone(rowOf(batchFile("b6", 0)), IMAGE);
    appModeStore.set("archive");

    push(batchItem("b6", 1));

    expect(appModeStore.get()).toBe("archive");
    expect(fileStore.getFile(rowOf(batchFile("b6", 1)))?.status).toBe(FILE_STATUS.QUEUED);
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([batchFile("b6", 1), batchFile("b6", 0)]);
    expect(fileStore.getSelected()).toBe(rowOf(batchFile("b6", 0)));
  });

  it("shows a file whose Download and open joined a download of a request that already showed another file", async () => {
    loadedAndSelected(CAL);
    push(batchItem("b7", 0));
    fileStore.fileDone(rowOf(batchFile("b7", 0)), IMAGE);
    const running = { ...batchItem("b7", 1), state: "downloading" as const, received: 10 };
    push(running);
    enqueueMock.mockResolvedValueOnce([batchItem("b7", 0), running]);
    const input = { datasetId: running.datasetId, filename: running.filename, program: running.program, sizeBytes: running.sizeBytes, uri: running.uri, opensAs: running.opensAs, replace: false };

    expect(await startDownloads("jwst", [input], true, store)).toBe(true);
    push(batchItem("b7", 1));
    fileStore.fileDone(rowOf(batchFile("b7", 1)), IMAGE);

    expect(fileStore.getSelected()).toBe(rowOf(batchFile("b7", 1)));
  });

  it("still shows a file the user opens from the queue, and the first file of the next request", () => {
    loadedAndSelected(CAL);
    push(batchItem("b3", 0));
    fileStore.fileDone(rowOf(batchFile("b3", 0)), IMAGE);
    push(batchItem("b3", 1));
    fileStore.fileDone(rowOf(batchFile("b3", 1)), IMAGE);

    expect(openDownloaded(batchItem("b3", 1))).toBe(true);
    expect(fileStore.getSelected()).toBe(rowOf(batchFile("b3", 1)));

    push(batchItem("b4", 0));
    fileStore.fileDone(rowOf(batchFile("b4", 0)), IMAGE);
    expect(fileStore.getSelected()).toBe(rowOf(batchFile("b4", 0)));
  });
});

describe("Download and open while a Files filter hides the opened file", () => {
  const C1 = "C:\\data\\c1_cal.fits";
  const C2 = "C:\\data\\c2_cal.fits";
  const C3 = "C:\\data\\c3_cal.fits";
  let unregister: () => void;

  function shownInViewer(): string | null {
    const active = productFilterStore.getActiveFilters();
    const visible = fileStore.getFiles().filter((f) => matchesActiveFilters(processedFilterable(f), active, productFilterStore.getMode()));
    return fallbackSelectionId(visible, fileStore.getSelected(), active.length > 0);
  }

  beforeEach(() => {
    fileStore.reset();
    productFilterStore.setActiveFilters([]);
    unregister = registerFileIngest((files) => fileStore.addFiles(files));
  });

  afterEach(() => {
    unregister();
    productFilterStore.setActiveFilters([]);
  });

  it("widens the filter to the opened file instead of leaving the viewer on the first visible row", () => {
    loadedAndSelected(C1);
    loadedAndSelected(C2);
    const viewing = loadedAndSelected(C3);
    productFilterStore.setActiveFilters(["cal"]);
    expect(shownInViewer()).toBe(viewing);

    expect(openDownloaded(item(NEW))).toBe(true);
    fileStore.fileDone(rowOf(NEW), IMAGE);

    expect(shownInViewer()).toBe(rowOf(NEW));
    expect(productFilterStore.getActiveFilters()).toEqual(["cal", "i2d"]);
  });

  it("widens the filter to an opened file the Files list already holds", () => {
    const opened = loadedAndSelected(NEW);
    const viewing = loadedAndSelected(C3);
    productFilterStore.setActiveFilters(["cal"]);
    expect(shownInViewer()).toBe(viewing);

    expect(openDownloaded(item(NEW))).toBe(true);

    expect(shownInViewer()).toBe(opened);
  });

  it("still shows the opened file when a filter change moved the selection while it loaded", () => {
    loadedAndSelected(OTHER);
    loadedAndSelected(C1);
    expect(openDownloaded(item(NEW))).toBe(true);
    productFilterStore.setActiveFilters(["i2d"]);
    followFilteredSelection(shownInViewer());
    expect(fileStore.getSelected()).toBe(rowOf(OTHER));

    fileStore.fileDone(rowOf(NEW), IMAGE);

    expect(fileStore.getSelected()).toBe(rowOf(NEW));
  });

  it("leaves the filter alone when the opened file is already visible", () => {
    loadedAndSelected(C3);
    productFilterStore.setActiveFilters(["i2d"]);

    expect(openDownloaded(item(NEW))).toBe(true);
    fileStore.fileDone(rowOf(NEW), IMAGE);

    expect(shownInViewer()).toBe(rowOf(NEW));
    expect(productFilterStore.getActiveFilters()).toEqual(["i2d"]);
  });
});
