import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { Download, RotateCcw, Sparkles } from "lucide-react";
import {
  CUBE_SAMPLE_SETS,
  IFU_SAMPLE_SET,
  JWST_SAMPLE_PROGRAMS,
  MAST_DATA_USE_URL,
  MIRI_LRS_SAMPLE_SET,
  jwstSampleTerms,
  openSampleProducts,
  paletteSampleSetsSelector,
  sampleDataHint,
  sampleOfferAction,
  samplePaletteActions,
  sampleSetRun,
  shownSamplePath,
  type PaletteSampleSet,
} from "../sampleOffers";
import { createSampleDataStore, type SampleControls, type SampleEntry } from "../../hooks/useSampleData";
import { TauriCommandError } from "../../shared/types/errors";
import { fileStore } from "../../hooks/useFileStore";
import { registerFileIngest } from "../../hooks/useFileIngest";
import { DEFAULT_DOCK_LAYOUT, anchorOf, anchorOpenTool, dockReducer } from "../dockLayout";

const IFU_DIR = "C:/Users/me/AstroBurst/samples/jwst-gc-nirspec-ifu";
const LRS_DIR = "C:/Users/me/AstroBurst/samples/jwst-miri-lrs";
const S3D = `${IFU_DIR}/jw01266-o005_t001_nirspec_g395h-f290lp_s3d.fits`;
const IFU_X1D = `${IFU_DIR}/jw01266-o005_t001_nirspec_g395h-f290lp_x1d.fits`;
const LRS_X1D = `${LRS_DIR}/jw02072-o001_t010_miri_p750l_x1d.fits`;
const LRS_S2D = `${LRS_DIR}/jw02072-o001_t010_miri_p750l_s2d.fits`;
const LRS_CAL = `${LRS_DIR}/jw02072001001_06101_00001_mirimage_cal.fits`;

function entry(over: Partial<SampleEntry>): SampleEntry {
  return {
    id: "hst-m16-wfpc2",
    title: "HST/WFPC2 narrowband frames of M16 (Eagle Nebula)",
    credit: "HST credit",
    licence: "HST licence",
    demonstrates: "Compose",
    total_bytes: 30_896_640,
    file_count: 3,
    state: "absent",
    received_bytes: 0,
    folder: "C:/Users/me/AstroBurst/samples/hst-m16-wfpc2",
    file: null,
    error: null,
    ...over,
  };
}

const IFU = entry({ id: IFU_SAMPLE_SET, title: "JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum", total_bytes: 175_680_000, file_count: 2, folder: IFU_DIR });
const LRS = entry({ id: MIRI_LRS_SAMPLE_SET, title: "JWST/MIRI LRS spectrum of SN 2021aefx with its 2-D and calibrated frames", total_bytes: 35_164_800, folder: LRS_DIR });

function controls(over: Partial<SampleControls> = {}): SampleControls {
  return {
    id: IFU_SAMPLE_SET,
    title: IFU.title,
    credit: IFU.credit,
    licence: IFU.licence,
    fileCount: 2,
    state: "absent",
    receivedBytes: 0,
    totalBytes: 175_680_000,
    file: null,
    error: null,
    download: vi.fn(async () => null),
    cancel: vi.fn(async () => {}),
    open: vi.fn(async () => null),
    ...over,
  };
}

describe("cube sample sets", () => {
  it("offers the NIRSpec IFU set and the MIRI LRS set, in that order", () => {
    expect(CUBE_SAMPLE_SETS).toEqual(["jwst-gc-nirspec-ifu", "jwst-miri-lrs"]);
  });
});

describe("shownSamplePath", () => {
  it("shows the cube of the IFU set and the 1-D spectrum of the MIRI LRS set, in any file order", () => {
    expect(shownSamplePath([S3D, IFU_X1D])).toBe(S3D);
    expect(shownSamplePath([IFU_X1D, S3D])).toBe(S3D);
    expect(shownSamplePath([LRS_CAL, LRS_S2D, LRS_X1D])).toBe(LRS_X1D);
    expect(shownSamplePath(["C:/s/A_S3D.FITS"])).toBe("C:/s/A_S3D.FITS");
  });

  it("falls back to the first file, and to nothing for an empty set", () => {
    expect(shownSamplePath([LRS_CAL, LRS_S2D])).toBe(LRS_CAL);
    expect(shownSamplePath([])).toBeNull();
  });
});

describe("sampleOfferAction", () => {
  it("offers the download with the set's size before anything is kept", () => {
    const sample = controls();
    const action = sampleOfferAction(sample);
    expect(action).toMatchObject({ label: "Download sample data (176 MB)", icon: Download });
    action?.run();
    expect(sample.download).toHaveBeenCalledTimes(1);
  });

  it("resumes a partial download, downloads again after a failure and opens a kept set", () => {
    expect(sampleOfferAction(controls({ state: "partial", receivedBytes: 52_000_000 }))?.label).toBe("Resume sample download (52 MB of 176 MB)");
    const failed = controls({ state: "partial", receivedBytes: 52_000_000, error: "connection reset" });
    expect(sampleOfferAction(failed)).toMatchObject({ label: "Retry sample download", icon: RotateCcw });
    sampleOfferAction(failed)?.run();
    expect(failed.download).toHaveBeenCalledTimes(1);
    const kept = controls({ state: "ready", receivedBytes: 175_680_000 });
    expect(sampleOfferAction(kept)).toMatchObject({ label: "Open sample data", icon: Sparkles });
    sampleOfferAction(kept)?.run();
    expect(kept.open).toHaveBeenCalledTimes(1);
    expect(kept.download).not.toHaveBeenCalled();
  });

  it("gives the percentage when the received amount rounds to the whole set", () => {
    expect(sampleOfferAction(controls({ state: "partial", receivedBytes: 175_600_000 }))?.label).toBe("Resume sample download (99% of 176 MB)");
  });

  it("gives no button while the download runs, where the bar and Cancel take its place", () => {
    expect(sampleOfferAction(controls({ state: "downloading", receivedBytes: 1_000_000 }))).toBeNull();
  });
});

describe("sample set copy", () => {
  it("describes a set from its title and file count", () => {
    expect(sampleDataHint({ title: "HST/WFPC2 narrowband frames of M16 (Eagle Nebula)", fileCount: 3 })).toBe(
      "HST/WFPC2 narrowband frames of M16 (Eagle Nebula): 3 files, downloaded once and kept in the data folder",
    );
    expect(sampleDataHint({ title: "One frame", fileCount: 1 })).toBe("One frame: 1 file, downloaded once and kept in the data folder");
  });

  it("names the MAST data use terms and the JWST program each JWST set is acknowledged under, and nothing for the HST set", () => {
    expect(jwstSampleTerms(IFU_SAMPLE_SET)).toBe(
      "Used under the MAST data use terms (https://archive.stsci.edu/publishing/data-use); publications acknowledge JWST program #1266 and STScI.",
    );
    expect(jwstSampleTerms(MIRI_LRS_SAMPLE_SET)).toContain("JWST program #2072 and STScI");
    expect(jwstSampleTerms("hst-m16-wfpc2")).toBeNull();
    expect(jwstSampleTerms("toString")).toBeNull();
  });

  it("says what the JWST sample notice shipped with the app says", () => {
    const notice = readFileSync(fileURLToPath(new URL("../../../src-tauri/licenses/jwst-sample-data-NOTICE.txt", import.meta.url)), "utf8");
    expect(notice).toContain(MAST_DATA_USE_URL);
    for (const program of Object.values(JWST_SAMPLE_PROGRAMS)) expect(notice).toContain(`program #${program}`);
  });
});

describe("samplePaletteActions", () => {
  const M16 = entry({});
  const runs = () => ({
    primary: { download: vi.fn<(id: string) => void>(), open: vi.fn<(id: string) => void>() },
    cube: { download: vi.fn<(id: string) => void>(), open: vi.fn<(id: string) => void>() },
  });
  const shown = { workspaceEmpty: false, fileShown: true };
  const empty = { workspaceEmpty: true, fileShown: false };

  it("lists the JWST sets labelled with their title and size, and runs them in the Cube tool", () => {
    const calls = runs();
    const actions = samplePaletteActions([M16, IFU, { ...LRS, state: "ready", received_bytes: LRS.total_bytes }], shown, calls);
    expect(actions.map((a) => [a.id, a.label])).toEqual([
      ["download-sample-jwst-gc-nirspec-ifu", "Download sample data (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum, 176 MB)"],
      ["open-sample-jwst-miri-lrs", "Open sample data (JWST/MIRI LRS spectrum of SN 2021aefx with its 2-D and calibrated frames)"],
    ]);
    expect(actions[0].icon).toBe(Download);
    expect(actions[1].icon).toBe(Sparkles);
    expect(actions[0].keywords).toEqual(expect.arrayContaining(["Sample", "Download", "Cube"]));
    actions[0].run();
    actions[1].run();
    expect(calls.cube.download).toHaveBeenCalledWith(IFU_SAMPLE_SET);
    expect(calls.cube.open).toHaveBeenCalledWith(MIRI_LRS_SAMPLE_SET);
    expect(calls.primary.download).not.toHaveBeenCalled();
  });

  it.each(["absent", "partial", "downloading"] as const)("keeps Open but lists no JWST download where its progress would not show (%s)", (state) => {
    const actions = samplePaletteActions([{ ...IFU, state }, { ...LRS, state: "ready" }], empty, runs());
    expect(actions.map((a) => a.id)).toEqual(["open-sample-jwst-miri-lrs"]);
  });

  it("lists nothing outside the desktop app, where no set is listed", () => {
    expect(samplePaletteActions([], shown, runs())).toEqual([]);
    expect(samplePaletteActions([], empty, runs())).toEqual([]);
  });

  it.each([
    ["a partial", { state: "partial", received_bytes: 52_000_000 }, "Resume sample download (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum, 52 MB of 176 MB)", Download],
    ["a failed", { state: "partial", received_bytes: 52_000_000, error: "connection reset" }, "Retry sample download (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum)", RotateCcw],
    ["a running", { state: "downloading", received_bytes: 52_000_000 }, "Show sample download (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum)", Download],
  ] as const)("words the entry for %s download as the Cube tool's button does", (_kind, over, label, icon) => {
    const calls = runs();
    const [action] = samplePaletteActions([{ ...IFU, ...over }], shown, calls);
    expect(action).toMatchObject({ id: "download-sample-jwst-gc-nirspec-ifu", label, icon });
    action.run();
    expect(calls.cube.download).toHaveBeenCalledWith(IFU_SAMPLE_SET);
  });

  it("lists a JWST download only while a file is shown, since the Cube tool that shows the download needs one", () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "cube" });
    expect(anchorOpenTool(layout, anchorOf(layout, "cube"), false)).toBeNull();
    expect(anchorOpenTool(layout, anchorOf(layout, "cube"), true)).toBe("cube");
    expect(samplePaletteActions([IFU], { workspaceEmpty: false, fileShown: false }, runs())).toEqual([]);
    expect(samplePaletteActions([IFU], { workspaceEmpty: true, fileShown: false }, runs())).toEqual([]);
    expect(samplePaletteActions([IFU], { workspaceEmpty: false, fileShown: true }, runs()).map((a) => a.id)).toEqual(["download-sample-jwst-gc-nirspec-ifu"]);
  });

  it.each([true, false])("offers Open for the kept M16 set whatever the workspace holds (empty workspace: %s)", (workspaceEmpty) => {
    const calls = runs();
    const [action] = samplePaletteActions([{ ...M16, state: "ready", received_bytes: M16.total_bytes }], { workspaceEmpty, fileShown: !workspaceEmpty }, calls);
    expect(action).toMatchObject({ id: "open-sample", label: `Open sample data (${M16.title})`, icon: Sparkles });
    expect(action.keywords).toEqual(["Sample", "Example", "M16", "Eagle", "Download"]);
    action.run();
    expect(calls.primary.open).toHaveBeenCalledWith(M16.id);
    expect(calls.cube.open).not.toHaveBeenCalled();
  });

  it.each([
    ["absent", {}, `Download sample data (HST/WFPC2 narrowband frames of M16 (Eagle Nebula), 31 MB)`, Download],
    ["partial", { state: "partial", received_bytes: 10_298_880 }, `Resume sample download (HST/WFPC2 narrowband frames of M16 (Eagle Nebula), 10 MB of 31 MB)`, Download],
    ["failed", { state: "partial", received_bytes: 10_298_880, error: "network unreachable" }, `Retry sample download (HST/WFPC2 narrowband frames of M16 (Eagle Nebula))`, RotateCcw],
    ["running", { state: "downloading", received_bytes: 10_298_880 }, `Show sample download (HST/WFPC2 narrowband frames of M16 (Eagle Nebula))`, Download],
  ] as const)("words the M16 download (%s) on the empty workspace as the empty state does, and runs it there", (_kind, over, label, icon) => {
    const calls = runs();
    const [action] = samplePaletteActions([{ ...M16, ...over } as PaletteSampleSet], empty, calls);
    expect(action).toMatchObject({ id: "download-sample", label, icon });
    action.run();
    expect(calls.primary.download).toHaveBeenCalledWith(M16.id);
  });

  it.each(["absent", "partial", "downloading"] as const)("lists no M16 download once a batch is loaded, where its progress and errors would not show (%s)", (state) => {
    expect(samplePaletteActions([{ ...M16, state }], shown, runs())).toEqual([]);
  });
});

describe("sampleSetRun", () => {
  function storeWith(failure: { download?: string; open?: string }) {
    const report = { ...IFU, state: "ready" as const };
    return createSampleDataStore({
      list: async () => [report],
      download: async () => {
        if (failure.download) throw new TauriCommandError("sample_download_cmd", failure.download);
        return { id: IFU.id, paths: [S3D, IFU_X1D], credit: IFU.credit };
      },
      cancel: async () => true,
      open: async () => {
        if (failure.open) throw new TauriCommandError("sample_open_cmd", failure.open);
        return { id: IFU.id, paths: [S3D, IFU_X1D], credit: IFU.credit };
      },
    });
  }

  it("shows the Cube tool first and reports a failed palette download through the notice, with the set's title", async () => {
    vi.spyOn(console, "error").mockImplementationOnce(() => {});
    const store = storeWith({ download: "the sample host answered 404" });
    await store.refresh();
    const calls: string[] = [];
    const notify = vi.fn((message: string) => void calls.push(`notify ${message}`));
    sampleSetRun(store, () => calls.push("show"), notify).download(IFU.id);
    await vi.waitFor(() => expect(notify).toHaveBeenCalledTimes(1));
    expect(calls).toEqual(["show", `notify ${IFU.title}: the sample host answered 404`]);
  });

  it("reports an open that finds the files gone, and nothing for an open that works", async () => {
    const gone = storeWith({ open: "not downloaded" });
    await gone.refresh();
    const notify = vi.fn();
    sampleSetRun(gone, () => {}, notify).open(IFU.id);
    await vi.waitFor(() => expect(notify).toHaveBeenCalledWith(`${IFU.title}: The sample frames are no longer in the data folder`));
    const kept = storeWith({});
    await kept.refresh();
    const quiet = vi.fn();
    fileStore.reset();
    const unregister = registerFileIngest((files) => fileStore.addFiles(files));
    sampleSetRun(kept, () => {}, quiet).open(IFU.id);
    await vi.waitFor(() => expect(fileStore.getFiles()).toHaveLength(2));
    unregister();
    expect(quiet).not.toHaveBeenCalled();
  });
});

describe("paletteSampleSetsSelector", () => {
  function selectorOver(...lists: SampleEntry[][]) {
    let at = 0;
    const store = { get: () => lists[at] };
    const select = paletteSampleSetsSelector(store);
    return { select, next: () => void at++ };
  }

  it("keeps the same array while only the bytes of a running download change, so the palette is not rebuilt for each progress event", () => {
    const { select, next } = selectorOver([{ ...IFU, state: "downloading", received_bytes: 1_000_000 }], [{ ...IFU, state: "downloading", received_bytes: 90_000_000 }]);
    const first = select();
    next();
    expect(select()).toBe(first);
    expect(first).toEqual([{ id: IFU.id, title: IFU.title, state: "downloading", total_bytes: IFU.total_bytes, received_bytes: 0, error: null }]);
  });

  it.each([
    ["state and bytes", { state: "partial", received_bytes: 52_000_000 }],
    ["error", { error: "connection reset" }],
    ["title", { title: "IFU" }],
    ["size", { total_bytes: 1 }],
    ["state", { state: "ready" }],
  ] as const)("gives a new array when the %s an entry shows changes", (_what, over) => {
    const { select, next } = selectorOver([IFU], [{ ...IFU, ...over }]);
    const first = select();
    next();
    expect(select()).not.toBe(first);
  });
});

describe("openSampleProducts", () => {
  const SOURCE = "C:/data/673nmos.fits";
  let unregister: () => void;

  beforeEach(() => {
    fileStore.reset();
    unregister = registerFileIngest((files) => fileStore.addFiles(files));
  });

  afterEach(() => unregister());

  const rowOf = (path: string) => fileStore.getFiles().find((f) => f.path === path)?.id;

  it("adds every file of the set and shows its cube once it has loaded, although another file was shown", () => {
    fileStore.addFiles([{ name: "673nmos.fits", path: SOURCE, size: 0 }]);
    const shown = rowOf(SOURCE) as string;
    fileStore.fileDone(shown, null);
    fileStore.selectFile(shown);

    openSampleProducts({ paths: [IFU_X1D, S3D] });

    expect(fileStore.getFiles().map((f) => f.path)).toEqual([SOURCE, IFU_X1D, S3D]);
    fileStore.fileSpectrum(rowOf(IFU_X1D) as string, { kind: "x1d", hdu: 1 });
    expect(fileStore.getSelected()).toBe(shown);
    fileStore.fileDone(rowOf(S3D) as string, null);
    expect(fileStore.getSelected()).toBe(rowOf(S3D));
  });

  it("opens the M16 set a second time without adding its loaded frames again, and shows the first frame", () => {
    const frames = ["502nmos.fits", "656nmos.fits", "673nmos.fits"].map((name) => `C:/data/samples/hst-m16-wfpc2/${name}`);
    expect(openSampleProducts({ paths: frames })).toBeNull();
    for (const path of frames) fileStore.fileDone(rowOf(path) as string, null);
    fileStore.selectFile(rowOf(frames[2]) as string);

    expect(openSampleProducts({ paths: frames })).toBeNull();

    expect(fileStore.getFiles().map((f) => f.path)).toEqual(frames);
    expect(fileStore.getSelected()).toBe(rowOf(frames[0]));
  });

  it("reports a file of the set that cannot be loaded and opens the others", () => {
    expect(openSampleProducts({ paths: [S3D, `${IFU_DIR}/readme.txt`] })).toEqual({ skipped: 1, calib: 0, message: null });
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([S3D]);
  });

  it("shows a cube the Files list already holds at once and adds only the files it lacks", () => {
    fileStore.addFiles([{ name: "s3d.fits", path: S3D, size: 0 }, { name: "673nmos.fits", path: SOURCE, size: 0 }]);
    fileStore.fileDone(rowOf(S3D) as string, null);
    fileStore.fileDone(rowOf(SOURCE) as string, null);
    fileStore.selectFile(rowOf(SOURCE) as string);

    openSampleProducts({ paths: [S3D, IFU_X1D] });

    expect(fileStore.getSelected()).toBe(rowOf(S3D));
    expect(fileStore.getFiles().map((f) => f.path)).toEqual([S3D, SOURCE, IFU_X1D]);
  });
});
