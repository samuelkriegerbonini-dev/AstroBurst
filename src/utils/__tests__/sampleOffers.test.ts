import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Download, RotateCcw, Sparkles } from "lucide-react";
import {
  CUBE_SAMPLE_SETS,
  IFU_SAMPLE_SET,
  MIRI_LRS_SAMPLE_SET,
  openSampleProducts,
  paletteSampleSets,
  paletteSampleSetsKey,
  sampleOfferAction,
  sampleSetPaletteActions,
  shownSamplePath,
} from "../sampleOffers";
import type { SampleControls, SampleEntry } from "../../hooks/useSampleData";
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
    newer_at_mast: false,
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
    state: "absent",
    receivedBytes: 0,
    totalBytes: 175_680_000,
    file: null,
    error: null,
    download: vi.fn(async () => {}),
    cancel: vi.fn(async () => {}),
    open: vi.fn(async () => {}),
    retry: vi.fn(async () => {}),
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
    expect(action).toMatchObject({ label: "Download sample (176 MB)", icon: Download });
    action?.run();
    expect(sample.download).toHaveBeenCalledTimes(1);
  });

  it("resumes a partial download, retries a failed one and opens a kept set", () => {
    expect(sampleOfferAction(controls({ state: "partial", receivedBytes: 52_000_000 }))?.label).toBe("Resume sample download (52 MB of 176 MB)");
    const failed = controls({ state: "partial", receivedBytes: 52_000_000, error: "connection reset" });
    expect(sampleOfferAction(failed)).toMatchObject({ label: "Retry sample download", icon: RotateCcw });
    sampleOfferAction(failed)?.run();
    expect(failed.retry).toHaveBeenCalledTimes(1);
    const kept = controls({ state: "ready", receivedBytes: 175_680_000 });
    expect(sampleOfferAction(kept)).toMatchObject({ label: "Open sample", icon: Sparkles });
    sampleOfferAction(kept)?.run();
    expect(kept.open).toHaveBeenCalledTimes(1);
  });

  it("gives no button while the download runs, where the bar and Cancel take its place", () => {
    expect(sampleOfferAction(controls({ state: "downloading", receivedBytes: 1_000_000 }))).toBeNull();
  });
});

describe("sampleSetPaletteActions", () => {
  const run = () => ({ download: vi.fn<(id: string) => void>(), open: vi.fn<(id: string) => void>() });

  it("lists one entry per set other than the HST set, labelled with its title and size", () => {
    const calls = run();
    const actions = sampleSetPaletteActions([entry({}), IFU, { ...LRS, state: "ready", received_bytes: LRS.total_bytes }], true, calls);
    expect(actions.map((a) => [a.id, a.label])).toEqual([
      ["download-sample-jwst-gc-nirspec-ifu", "Download sample data (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum, 176 MB)"],
      ["open-sample-jwst-miri-lrs", "Open sample data (JWST/MIRI LRS spectrum of SN 2021aefx with its 2-D and calibrated frames)"],
    ]);
    expect(actions[0].icon).toBe(Download);
    expect(actions[1].icon).toBe(Sparkles);
    expect(actions[0].keywords).toEqual(expect.arrayContaining(["Sample", "Download", "Cube"]));
    actions[0].run();
    actions[1].run();
    expect(calls.download).toHaveBeenCalledWith(IFU_SAMPLE_SET);
    expect(calls.open).toHaveBeenCalledWith(MIRI_LRS_SAMPLE_SET);
  });

  it.each(["absent", "partial", "downloading"] as const)("keeps Open but lists no download where its progress would not show (%s)", (state) => {
    const actions = sampleSetPaletteActions([{ ...IFU, state }, { ...LRS, state: "ready" }], false, run());
    expect(actions.map((a) => a.id)).toEqual(["open-sample-jwst-miri-lrs"]);
  });

  it("lists nothing outside the desktop app, where no set is listed", () => {
    expect(sampleSetPaletteActions([], true, run())).toEqual([]);
  });

  it.each([
    ["a partial", { state: "partial", received_bytes: 52_000_000 }, "Resume sample download (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum, 52 MB of 176 MB)", Download],
    ["a failed", { state: "partial", received_bytes: 52_000_000, error: "connection reset" }, "Retry sample download (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum)", RotateCcw],
    ["a running", { state: "downloading", received_bytes: 52_000_000 }, "Show sample download (JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum)", Download],
  ] as const)("words the entry for %s download as the Cube tool's button does", (_kind, over, label, icon) => {
    const calls = run();
    const [action] = sampleSetPaletteActions([{ ...IFU, ...over }], true, calls);
    expect(action).toMatchObject({ id: "download-sample-jwst-gc-nirspec-ifu", label, icon });
    action.run();
    expect(calls.download).toHaveBeenCalledWith(IFU_SAMPLE_SET);
  });

  it("is listed by App only while a file is shown, since the Cube tool that shows the download needs one", () => {
    const layout = dockReducer(DEFAULT_DOCK_LAYOUT, { type: "open", tool: "cube" });
    expect(anchorOpenTool(layout, anchorOf(layout, "cube"), false)).toBeNull();
    expect(anchorOpenTool(layout, anchorOf(layout, "cube"), true)).toBe("cube");
    const app = readFileSync(fileURLToPath(new URL("../../App.tsx", import.meta.url)), "utf8");
    expect(/sampleSetPaletteActions\(\s*[^,]+,\s*([^,]+?)\s*,/.exec(app)?.[1]).toBe("hasVisibleFile");
  });
});

describe("paletteSampleSetsKey", () => {
  it("keeps its key while only the bytes of a running download change, so the palette is not rebuilt for each progress event", () => {
    const running = (received_bytes: number) => paletteSampleSetsKey([{ ...IFU, state: "downloading", received_bytes }]);
    expect(running(1_000_000)).toBe(running(90_000_000));
  });

  it("changes its key with what an entry shows, and gives the sets back", () => {
    const key = paletteSampleSetsKey([IFU]);
    expect(paletteSampleSetsKey([{ ...IFU, state: "partial", received_bytes: 52_000_000 }])).not.toBe(key);
    expect(paletteSampleSetsKey([{ ...IFU, error: "connection reset" }])).not.toBe(key);
    expect(paletteSampleSetsKey([{ ...IFU, title: "IFU" }])).not.toBe(key);
    expect(paletteSampleSetsKey([{ ...IFU, total_bytes: 1 }])).not.toBe(key);
    expect(paletteSampleSetsKey([{ ...IFU, state: "ready" }])).not.toBe(key);
    expect(paletteSampleSets(key)).toEqual([{ id: IFU.id, title: IFU.title, state: "absent", total_bytes: IFU.total_bytes, received_bytes: 0, error: null }]);
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
