import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { fileStore } from "../useFileStore";
import { ingestFiles, ingestUnlessLoaded, registerFileIngest, showOrOpen, showWhenLoaded, type IngestOptions } from "../useFileIngest";
import type { AstroFile } from "../../shared/types";

const SOURCE = "C:/data/673nmos.fits";
const OUTPUT = "C:/out/673nmos_pixelmath.fits";
const ROTATED = "C:/out/673nmos_rot90.fits";

describe("file ingest", () => {
  let ingested: AstroFile[][];
  let options: (IngestOptions | undefined)[];
  let unregister: () => void;

  beforeEach(() => {
    fileStore.reset();
    ingested = [];
    options = [];
    unregister = registerFileIngest((files, opts) => {
      ingested.push(files);
      options.push(opts);
      fileStore.addFiles(files);
    });
  });

  afterEach(() => unregister());

  function loadAndSelect(path: string): string {
    fileStore.addFiles([{ name: path.split("/").pop() as string, path, size: 0 }]);
    const id = fileStore.getFileIds()[fileStore.getFileIds().length - 1];
    fileStore.selectFile(id);
    return id;
  }

  it("adds a PixelMath output that is not loaded yet to the Files list and keeps the selection", () => {
    const sourceId = loadAndSelect(SOURCE);
    expect(ingestUnlessLoaded(OUTPUT)).toBe(true);
    expect(ingested).toEqual([[{ name: "673nmos_pixelmath.fits", path: OUTPUT, size: 0 }]]);
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([SOURCE, OUTPUT]);
    expect(fileStore.getSelected()).toBe(sourceId);
  });

  it("does not add a second row when a re-run rewrote an output that is already loaded", () => {
    loadAndSelect(OUTPUT);
    const sourceId = loadAndSelect(SOURCE);
    expect(ingestUnlessLoaded("c:\\out\\673NMOS_pixelmath.fits")).toBe(false);
    expect(ingested).toEqual([]);
    expect(fileStore.getFiles()).toHaveLength(2);
    expect(fileStore.getSelected()).toBe(sourceId);
  });

  it("ingestUnlessLoaded marks a PixelMath output as a quiet add", () => {
    loadAndSelect(SOURCE);
    expect(ingestUnlessLoaded(OUTPUT)).toBe(true);
    expect(options).toEqual([{ quiet: true }]);
  });

  it("ingestUnlessLoaded passes a keep-mode request through with the quiet add", () => {
    loadAndSelect(SOURCE);
    expect(ingestUnlessLoaded(OUTPUT, { keepMode: true })).toBe(true);
    expect(options).toEqual([{ quiet: true, keepMode: true }]);
  });

  it("ingestFiles passes no options, so files opened by the user stay a normal add", () => {
    expect(ingestFiles([{ name: "656nmos.fits", path: "C:/data/656nmos.fits", size: 0 }])).toBe(true);
    expect(options).toEqual([undefined]);
  });

  it("retries an output row that errored before the file existed, without adding a second row", () => {
    const sourceId = loadAndSelect(SOURCE);
    const outputId = loadAndSelect(OUTPUT);
    fileStore.fileError(outputId, "No such file");
    fileStore.selectFile(sourceId);

    expect(ingestUnlessLoaded("c:\\OUT\\673nmos_pixelmath.fits")).toBe(true);

    expect(fileStore.getFile(outputId)).toMatchObject({ status: "queued", error: null });
    expect(fileStore.getFiles().map((f) => f.id)).toEqual([sourceId, outputId]);
    expect(fileStore.getStats()).toMatchObject({ total: 2, failed: 0 });
    expect(ingested).toHaveLength(1);
    expect(options).toEqual([{ quiet: true }]);
    expect(fileStore.getSelected()).toBe(sourceId);
  });

  it("leaves a row that is queued, loading or loaded alone", () => {
    const outputId = loadAndSelect(OUTPUT);
    expect(ingestUnlessLoaded(OUTPUT)).toBe(false);
    fileStore.fileStarted(outputId);
    expect(ingestUnlessLoaded(OUTPUT)).toBe(false);
    fileStore.fileDone(outputId, null);
    expect(ingestUnlessLoaded(OUTPUT)).toBe(false);
    expect(ingested).toEqual([]);
    expect(fileStore.getFile(outputId)?.status).toBe("done");
  });

  it("does not requeue an errored row when the viewer cannot ingest files yet", () => {
    const outputId = loadAndSelect(OUTPUT);
    fileStore.fileError(outputId, "No such file");
    unregister();
    expect(ingestUnlessLoaded(OUTPUT)).toBe(false);
    expect(fileStore.getFile(outputId)?.status).toBe("error");
  });

  it("reports that nothing was added when the viewer cannot ingest files yet", () => {
    unregister();
    expect(ingestUnlessLoaded(OUTPUT)).toBe(false);
    expect(fileStore.getFiles()).toHaveLength(0);
  });
});

describe("showOrOpen", () => {
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

  const rowOf = (path: string) => fileStore.getFiles().find((f) => f.path === path)?.id as string;

  it("shows a row the Files list holds at once and adds nothing", () => {
    fileStore.addFiles([
      { name: "673nmos_pixelmath.fits", path: OUTPUT, size: 0 },
      { name: "673nmos.fits", path: SOURCE, size: 0 },
    ]);
    fileStore.fileDone(rowOf(OUTPUT), null);
    fileStore.fileDone(rowOf(SOURCE), null);
    fileStore.selectFile(rowOf(SOURCE));

    expect(showOrOpen("c:\\out\\673NMOS_pixelmath.fits")).toBe(true);

    expect(fileStore.getSelected()).toBe(rowOf(OUTPUT));
    expect(added).toEqual([]);
  });

  it("retries a row that failed to load and shows it once it loads, although another file was shown", () => {
    fileStore.addFiles([
      { name: "673nmos.fits", path: SOURCE, size: 0 },
      { name: "673nmos_pixelmath.fits", path: OUTPUT, size: 0 },
    ]);
    fileStore.fileDone(rowOf(SOURCE), null);
    fileStore.selectFile(rowOf(SOURCE));
    fileStore.fileError(rowOf(OUTPUT), "No such file");

    expect(showOrOpen(OUTPUT)).toBe(true);

    expect(fileStore.getSelected()).toBe(rowOf(SOURCE));
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([SOURCE, OUTPUT]);
    expect(added).toHaveLength(1);
    fileStore.fileDone(rowOf(OUTPUT), null);
    expect(fileStore.getSelected()).toBe(rowOf(OUTPUT));
  });
});

describe("showWhenLoaded", () => {
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

  const rowOf = (path: string) => fileStore.getFiles().find((f) => f.path === path)?.id as string;

  function showSource(): string {
    fileStore.addFiles([{ name: "673nmos.fits", path: SOURCE, size: 0 }]);
    fileStore.fileDone(rowOf(SOURCE), null);
    fileStore.selectFile(rowOf(SOURCE));
    return rowOf(SOURCE);
  }

  it("selects a row that has loaded at once and adds nothing", () => {
    showSource();
    fileStore.addFiles([{ name: "673nmos_rot90.fits", path: ROTATED, size: 0 }]);
    fileStore.fileDone(rowOf(ROTATED), null);

    expect(showWhenLoaded("c:\\OUT\\673NMOS_rot90.fits")).toBe(true);

    expect(fileStore.getSelected()).toBe(rowOf(ROTATED));
    expect(fileStore.getSelectOnLoadPath()).toBeNull();
    expect(added).toEqual([]);
  });

  it("does not select a queued or loading row at once and selects it once it has loaded", () => {
    const sourceId = showSource();
    expect(ingestUnlessLoaded(ROTATED)).toBe(true);

    expect(showWhenLoaded(ROTATED)).toBe(true);

    expect(fileStore.getSelected()).toBe(sourceId);
    expect(fileStore.getSelectOnLoadPath()).toBe(ROTATED);
    fileStore.fileStarted(rowOf(ROTATED));
    expect(showWhenLoaded(ROTATED)).toBe(true);
    expect(fileStore.getSelected()).toBe(sourceId);
    fileStore.fileDone(rowOf(ROTATED), null);
    expect(fileStore.getSelected()).toBe(rowOf(ROTATED));
    expect(fileStore.getSelectOnLoadPath()).toBeNull();
    expect(added).toHaveLength(1);
  });

  it("adds a path the Files list does not hold and selects it once it has loaded", () => {
    const sourceId = showSource();

    expect(showWhenLoaded(ROTATED)).toBe(true);

    expect(added).toEqual([[{ name: "673nmos_rot90.fits", path: ROTATED, size: 0 }]]);
    expect(fileStore.getSelected()).toBe(sourceId);
    fileStore.fileDone(rowOf(ROTATED), null);
    expect(fileStore.getSelected()).toBe(rowOf(ROTATED));
  });

  it("retries a row that failed to load, without a second row, and selects it once it has loaded", () => {
    const sourceId = showSource();
    fileStore.addFiles([{ name: "673nmos_rot90.fits", path: ROTATED, size: 0 }]);
    fileStore.fileError(rowOf(ROTATED), "No such file");

    expect(showWhenLoaded(ROTATED)).toBe(true);

    expect(fileStore.getFile(rowOf(ROTATED))?.status).toBe("queued");
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([SOURCE, ROTATED]);
    expect(fileStore.getSelected()).toBe(sourceId);
    fileStore.fileDone(rowOf(ROTATED), null);
    expect(fileStore.getSelected()).toBe(rowOf(ROTATED));
  });

  it("keeps the file the user picks while the row is loading", () => {
    const sourceId = showSource();
    ingestUnlessLoaded(ROTATED);
    showWhenLoaded(ROTATED);

    fileStore.selectFile(sourceId);

    expect(fileStore.getSelectOnLoadPath()).toBeNull();
    fileStore.fileDone(rowOf(ROTATED), null);
    expect(fileStore.getSelected()).toBe(sourceId);
  });

  it("tells the store's listeners when a row starts and stops waiting to be shown", async () => {
    const sourceId = showSource();
    ingestUnlessLoaded(ROTATED);
    await Promise.resolve();
    const heard: (string | null)[] = [];
    const stop = fileStore.subscribe(() => heard.push(fileStore.getSelectOnLoadPath()));

    showWhenLoaded(ROTATED);
    await Promise.resolve();
    fileStore.selectFile(sourceId);
    await Promise.resolve();
    stop();

    expect(heard).toEqual([ROTATED, null]);
  });
});
