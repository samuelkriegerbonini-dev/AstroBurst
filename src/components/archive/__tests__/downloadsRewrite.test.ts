import { beforeEach, describe, expect, it, vi } from "vitest";
import type { MastDownloadItem } from "../../../shared/types/mast";

const { processMock, listenMock, snapshotMock } = vi.hoisted(() => ({ processMock: vi.fn(), listenMock: vi.fn(), snapshotMock: vi.fn() }));

vi.mock("../../../services/fits", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../services/fits")>();
  return { ...actual, processFitsFull: processMock };
});

vi.mock("../../../services/mast", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../services/mast")>();
  return { ...actual, mastDownloads: snapshotMock, onMastDownload: listenMock };
});

import { fileStore } from "../../../hooks/useFileStore";
import { spectrum1dStore } from "../../../hooks/useSpectrum1dStore";
import { createDownloadsStore } from "../state/downloadsStore";
import { openDownloaded, reloadRewritten, watchDownloads } from "../state/downloadsController";

const CAL = "C:\\data\\mast\\JWST\\2739\\d_cal.fits";
const X1D = "C:\\data\\mast\\JWST\\2739\\d_x1d.fits";

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
    open: false,
    replace: true,
    state: "done",
    received: 10,
    total: 10,
    path: CAL,
    resumedFrom: 0,
    reused: false,
    verified: true,
    action: null,
    error: null,
    ...over,
  };
}

function row(path: string): string {
  fileStore.addFiles([{ name: path.split("\\").pop() as string, path, size: 0 }]);
  const ids = fileStore.getFileIds();
  return ids[ids.length - 1];
}

describe("a download that rewrites a file the Files list holds", () => {
  beforeEach(() => {
    fileStore.reset();
    processMock.mockReset();
  });

  it("reloads the image row and rereads the spectrum document of that path", async () => {
    const image = row(CAL.toLowerCase().replace(/\\/g, "/"));
    fileStore.fileDone(image, { previewUrl: "old.png", dimensions: [10, 10] } as never);
    const spectrum = row(X1D);
    fileStore.fileSpectrum(spectrum, { kind: "x1d", hdu: 1 });
    const before = fileStore.getFile(image);
    processMock.mockResolvedValue({ previewUrl: "new.png", dimensions: [20, 20] });
    const reread = vi.spyOn(spectrum1dStore, "reread").mockResolvedValue();

    await reloadRewritten(CAL);
    await reloadRewritten(X1D);

    expect(processMock).toHaveBeenCalledTimes(1);
    expect(fileStore.getFile(image)).not.toBe(before);
    expect(fileStore.getFile(image)?.result).toMatchObject({ dimensions: [20, 20] });
    expect(fileStore.getFile(image)?.result?.previewUrl).toMatch(/^new\.png/);
    expect(reread).toHaveBeenCalledWith(X1D, 1);
    reread.mockRestore();
  });

  it("reloads once per finished transfer, with or without Open, and never for a reused file or one done before the window loaded", async () => {
    let push: ((next: MastDownloadItem) => void) | null = null;
    listenMock.mockImplementation(async (cb: (next: MastDownloadItem) => void) => {
      push = cb;
      return () => {};
    });
    snapshotMock.mockResolvedValue([item({ id: "old:0" })]);
    const store = createDownloadsStore();
    const reload = vi.fn();
    const show = vi.fn(() => true);
    await watchDownloads(store, { show, load: vi.fn(), setMode: vi.fn(), reload });
    expect(reload).not.toHaveBeenCalled();

    push!(item({ id: "re:0" }));
    push!(item({ id: "re:0" }));
    push!(item({ id: "same:0", reused: true, replace: false }));
    push!(item({ id: "open:0", open: true, path: X1D, filename: "d_x1d.fits", opensAs: "spectrum" }));

    expect(reload.mock.calls).toEqual([[CAL], [X1D]]);
    expect(show).toHaveBeenCalledTimes(1);
    expect(openDownloaded(item({ id: "re:0" }), { show, load: vi.fn(), setMode: vi.fn(), reload })).toBe(true);
    expect(reload).toHaveBeenCalledTimes(2);
  });
});
