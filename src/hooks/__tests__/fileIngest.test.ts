import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { fileStore } from "../useFileStore";
import { ingestFiles, ingestUnlessLoaded, registerFileIngest, type IngestOptions } from "../useFileIngest";
import type { AstroFile } from "../../shared/types";

const SOURCE = "C:/data/673nmos.fits";
const OUTPUT = "C:/out/673nmos_pixelmath.fits";

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

  it("ingestFiles passes no options, so files opened by the user stay a normal add", () => {
    expect(ingestFiles([{ name: "656nmos.fits", path: "C:/data/656nmos.fits", size: 0 }])).toBe(true);
    expect(options).toEqual([undefined]);
  });

  it("reports that nothing was added when the viewer cannot ingest files yet", () => {
    unregister();
    expect(ingestUnlessLoaded(OUTPUT)).toBe(false);
    expect(fileStore.getFiles()).toHaveLength(0);
  });
});
