import { describe, it, expect, beforeEach, vi } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const { typedInvokeMock, listenMock, unlistenMock } = vi.hoisted(() => ({
  typedInvokeMock: vi.fn(),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
}));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

import {
  SAMPLE_COMMANDS,
  SAMPLE_DOWNLOAD_EVENT,
  cancelSampleDownload,
  downloadSampleSet,
  isSampleCancelled,
  isSampleNotDownloaded,
  listSampleSets,
  onSampleDownloadProgress,
  openSampleSet,
  sampleErrorMessage,
  type SampleOpen,
  type SampleProgress,
  type SampleSetReport,
} from "../sampleData";
import { TauriCommandError } from "../../shared/types/errors";
import { PRIMARY_SAMPLE_SET } from "../../hooks/useSampleData";
import { CUBE_SAMPLE_SETS, JWST_SAMPLE_PROGRAMS } from "../../utils/sampleOffers";

const rustSource = (rel: string) => readFileSync(fileURLToPath(new URL(`../../../src-tauri/src/${rel}`, import.meta.url)), "utf8");

const M16: SampleSetReport = {
  id: "hst-m16-wfpc2",
  title: "HST/WFPC2 narrowband frames of M16 (Eagle Nebula)",
  credit: "HST/WFPC2 narrowband frames of the Eagle Nebula (M16): NASA, ESA, J. Hester and P. Scowen (Arizona State University), data via STScI",
  licence: "Public-domain Hubble archive data; credit requested by STScI",
  demonstrates: "Compose (SHO and HOO narrowband palettes), photometry and masks",
  total_bytes: 30_896_640,
  file_count: 3,
  state: "absent",
  received_bytes: 0,
  folder: "C:\\Users\\me\\AstroBurst\\samples\\hst-m16-wfpc2",
};

const IFU: SampleSetReport = {
  id: "jwst-gc-nirspec-ifu",
  title: "JWST/NIRSpec IFU cube of the Galactic Center (G395H/F290LP) with its 1-D spectrum",
  credit: "JWST/NIRSpec IFU cube and 1-D spectrum of the Galactic Center (GC-NIRSPEC-1): NASA, ESA, CSA, GTO programme 1266 (PI M. Garcia Marin), data via MAST at STScI",
  licence: "Public JWST archive data; acknowledgement requested by STScI",
  demonstrates: "Cube: spectra with error bands, moment maps, line fits and line measurement, next to the pipeline's 1-D spectrum",
  total_bytes: 175_680_000,
  file_count: 2,
  state: "ready",
  received_bytes: 175_680_000,
  folder: "C:\\Users\\me\\AstroBurst\\samples\\jwst-gc-nirspec-ifu",
};

const LRS: SampleSetReport = {
  id: "jwst-miri-lrs",
  title: "JWST/MIRI LRS spectrum of the Type Ia supernova SN 2021aefx with its 2-D spectrum and a calibrated exposure",
  credit: "JWST/MIRI LRS slit spectroscopy of SN 2021aefx: NASA, ESA, CSA, GO programme 2072 (PI S. W. Jha), data via MAST at STScI",
  licence: "Public JWST archive data; acknowledgement requested by STScI",
  demonstrates: "1-D spectra: the spectrum document with line measurement, the 2-D resampled slit spectrum and a level-2 calibrated exposure",
  total_bytes: 35_164_800,
  file_count: 3,
  state: "absent",
  received_bytes: 0,
  folder: "C:\\Users\\me\\AstroBurst\\samples\\jwst-miri-lrs",
};

const NOT_PUBLISHED = "jw02072-o001_t010_miri_p750l_x1d.fits: not available on GitHub (HTTP 404); the sample release may not be published yet";

const OPENED: SampleOpen = {
  id: "hst-m16-wfpc2",
  paths: ["502nmos.fits", "656nmos.fits", "673nmos.fits"].map((name) => `C:\\Users\\me\\AstroBurst\\samples\\hst-m16-wfpc2\\${name}`),
  credit: M16.credit,
};

const PROGRESS: SampleProgress = { id: "hst-m16-wfpc2", received_bytes: 12_000_000, total_bytes: 30_896_640, file: "656nmos.fits" };

describe("sample data commands", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("lists the sets through sample_sets_cmd", async () => {
    typedInvokeMock.mockResolvedValueOnce([M16]);
    await expect(listSampleSets()).resolves.toEqual([M16]);
    expect(typedInvokeMock).toHaveBeenCalledWith("sample_sets_cmd", {});
  });

  it("lists the JWST sets next to the HST set, each with what it demonstrates", async () => {
    typedInvokeMock.mockResolvedValueOnce([M16, IFU, LRS]);
    const sets = await listSampleSets();
    expect(sets.map((set) => [set.id, set.state, set.total_bytes, set.demonstrates])).toEqual([
      ["hst-m16-wfpc2", "absent", 30_896_640, M16.demonstrates],
      ["jwst-gc-nirspec-ifu", "ready", 175_680_000, IFU.demonstrates],
      ["jwst-miri-lrs", "absent", 35_164_800, LRS.demonstrates],
    ]);
  });

  it("downloads a set by id through sample_download_cmd and returns the frames to open", async () => {
    typedInvokeMock.mockResolvedValueOnce(OPENED);
    await expect(downloadSampleSet("hst-m16-wfpc2")).resolves.toEqual(OPENED);
    expect(typedInvokeMock).toHaveBeenCalledWith("sample_download_cmd", { id: "hst-m16-wfpc2" });
  });

  it("cancels a running download by id through sample_cancel_cmd", async () => {
    typedInvokeMock.mockResolvedValueOnce(true);
    await expect(cancelSampleDownload("hst-m16-wfpc2")).resolves.toBe(true);
    expect(typedInvokeMock).toHaveBeenCalledWith("sample_cancel_cmd", { id: "hst-m16-wfpc2" });
  });

  it("opens a kept set by id through sample_open_cmd", async () => {
    typedInvokeMock.mockResolvedValueOnce(OPENED);
    await expect(openSampleSet("hst-m16-wfpc2")).resolves.toEqual(OPENED);
    expect(typedInvokeMock).toHaveBeenCalledWith("sample_open_cmd", { id: "hst-m16-wfpc2" });
  });

  it("names every command it invokes in SAMPLE_COMMANDS", async () => {
    typedInvokeMock.mockResolvedValue(null);
    await Promise.all([listSampleSets(), downloadSampleSet("a"), cancelSampleDownload("a"), openSampleSet("a")]);
    expect(typedInvokeMock.mock.calls.map((c) => c[0])).toEqual([...SAMPLE_COMMANDS]);
  });
});

describe("onSampleDownloadProgress", () => {
  beforeEach(() => {
    listenMock.mockReset();
    unlistenMock.mockReset();
  });

  it("subscribes to sample:download, forwards each progress payload and returns the unlisten", async () => {
    let handler: ((e: { payload: SampleProgress }) => void) | null = null;
    listenMock.mockImplementation(async (_name: string, cb: (e: { payload: SampleProgress }) => void) => {
      handler = cb;
      return unlistenMock;
    });
    const seen: SampleProgress[] = [];
    const unlisten = await onSampleDownloadProgress((p) => seen.push(p));
    expect(SAMPLE_DOWNLOAD_EVENT).toBe("sample:download");
    expect(listenMock).toHaveBeenCalledWith("sample:download", expect.any(Function));
    handler!({ payload: PROGRESS });
    expect(seen).toEqual([PROGRESS]);
    unlisten();
    expect(unlistenMock).toHaveBeenCalledTimes(1);
  });
});

describe("isSampleCancelled", () => {
  it("recognises the exact cancelled rejection of sample_download_cmd", () => {
    expect(isSampleCancelled(new TauriCommandError("sample_download_cmd", "cancelled"))).toBe(true);
    expect(isSampleCancelled("cancelled")).toBe(true);
  });

  it("treats every other failure as an error to show", () => {
    expect(isSampleCancelled(new TauriCommandError("sample_download_cmd", "656nmos.fits: the download did not match the published checksum"))).toBe(false);
    expect(isSampleCancelled(new TauriCommandError("sample_download_cmd", "502nmos.fits: the server answered HTTP 404"))).toBe(false);
    expect(isSampleCancelled(new Error("cancelled by the server"))).toBe(false);
    expect(isSampleCancelled(null)).toBe(false);
  });
});

describe("isSampleNotDownloaded", () => {
  it("recognises the not downloaded rejection of sample_open_cmd and nothing else", () => {
    expect(isSampleNotDownloaded(new TauriCommandError("sample_open_cmd", "not downloaded"))).toBe(true);
    expect(isSampleNotDownloaded(new TauriCommandError("sample_open_cmd", "the Data folder setting could not be read: x"))).toBe(false);
    expect(isSampleNotDownloaded(new TauriCommandError("sample_download_cmd", "cancelled"))).toBe(false);
    expect(isSampleNotDownloaded(undefined)).toBe(false);
  });

  it("matches the rejection text the Rust boundary sends", () => {
    expect(rustSource("cmd/sample_data.rs")).toMatch(/const NOT_DOWNLOADED: &str = "not downloaded";/);
  });
});

describe("sampleErrorMessage", () => {
  it("keeps the readable text of a command rejection", () => {
    expect(sampleErrorMessage(new TauriCommandError("sample_download_cmd", "673nmos.fits: the size did not match"))).toBe("673nmos.fits: the size did not match");
    expect(sampleErrorMessage("not downloaded")).toBe("not downloaded");
    expect(sampleErrorMessage(new Error("boom"))).toBe("boom");
  });

  it("shows a missing release asset with its 404 as the Rust boundary words it", () => {
    const rustText = rustSource("infra/samples.rs").match(/#\[error\(\s*"\{file\}: ([^"]*)"\s*\)\]\s*NotPublished \{/)?.[1];
    expect(rustText, "SampleError::NotPublished display").toBe("not available on GitHub (HTTP 404); the sample release may not be published yet");
    expect(sampleErrorMessage(new TauriCommandError("sample_download_cmd", NOT_PUBLISHED))).toBe(NOT_PUBLISHED);
    expect(isSampleCancelled(new TauriCommandError("sample_download_cmd", NOT_PUBLISHED))).toBe(false);
  });
});

describe("sample data contract with the Rust boundary", () => {
  const fields = (rust: string, name: string) => {
    const start = rust.search(new RegExp(`pub(?:\\(crate\\))? struct ${name}\\s*\\{`));
    expect(start, `struct ${name}`).toBeGreaterThanOrEqual(0);
    const body = rust.slice(rust.indexOf("{", start) + 1, rust.indexOf("\n}", start));
    return [...body.matchAll(/^\s*(?:pub(?:\(crate\))?\s+)?(\w+)\s*:/gm)].map((m) => m[1]).sort();
  };

  it("matches the commands cmd/sample_data.rs defines", () => {
    const defined = [...rustSource("cmd/sample_data.rs").matchAll(/#\[tauri::command\]\s*pub (?:async )?fn (\w+)/g)].map((m) => m[1]);
    expect([...defined].sort()).toEqual([...SAMPLE_COMMANDS].sort());
  });

  it("registers exactly the four sample commands in lib.rs", () => {
    const registered = [...rustSource("lib.rs").matchAll(/cmd::sample_data::(\w+)/g)].map((m) => m[1]);
    expect([...registered].sort()).toEqual([...SAMPLE_COMMANDS].sort());
  });

  it("emits progress under the same event name", () => {
    expect(rustSource("cmd/sample_data.rs")).toContain(`"${SAMPLE_DOWNLOAD_EVENT}"`);
  });

  it("recognises the cancel and not downloaded rejections exactly as the Rust boundary words them", () => {
    const cancelledText = rustSource("infra/samples.rs").match(/#\[error\("([^"]*)"\)\]\s*Cancelled,/)?.[1];
    const notDownloadedText = rustSource("cmd/sample_data.rs").match(/const NOT_DOWNLOADED: &str = "([^"]*)";/)?.[1];
    expect(cancelledText, "SampleError::Cancelled display").toBeDefined();
    expect(notDownloadedText, "NOT_DOWNLOADED").toBeDefined();
    expect(rustSource("cmd/sample_data.rs")).toMatch(/download_set\([^)]*\)\s*\.await\s*\.map_err\(\|error\| error\.to_string\(\)\)\?/);
    expect(isSampleCancelled(new TauriCommandError("sample_download_cmd", cancelledText!))).toBe(true);
    expect(isSampleNotDownloaded(new TauriCommandError("sample_open_cmd", notDownloadedText!))).toBe(true);
  });

  const rustSets = () => {
    const rust = rustSource("infra/samples.rs").replace(/\r\n/g, "\n");
    const start = rust.indexOf("static SAMPLE_SETS");
    expect(start, "static SAMPLE_SETS").toBeGreaterThanOrEqual(0);
    const body = rust.slice(start, rust.indexOf("\n];", start));
    return body.split(/\n {4}SampleSet \{/).slice(1).map((set) => ({
      id: /\bid: "([^"]+)"/.exec(set)?.[1],
      files: [...set.matchAll(/\bname: "([^"]+)"/g)].map((m) => m[1]),
    }));
  };

  it("finds the empty-state set and the Cube tool sets under the ids infra/samples.rs defines", () => {
    expect(rustSets().map((set) => set.id)).toEqual([PRIMARY_SAMPLE_SET, ...CUBE_SAMPLE_SETS]);
  });

  it("names the JWST program each Cube tool set's files come from", () => {
    for (const set of rustSets().filter((s) => s.id !== PRIMARY_SAMPLE_SET)) {
      const program = JWST_SAMPLE_PROGRAMS[set.id!];
      expect(program, set.id).toMatch(/^\d{4,5}$/);
      expect(set.files.length, set.id).toBeGreaterThan(0);
      expect(set.files.every((name) => name.startsWith(`jw${program.padStart(5, "0")}`)), set.id).toBe(true);
    }
  });

  it("mirrors the snake_case payloads of SampleSetReport, SampleOpen and SampleProgress", () => {
    const rust = rustSource("cmd/sample_data.rs");
    expect(fields(rust, "SampleSetReport")).toEqual(Object.keys(M16).sort());
    expect(Object.keys(IFU).sort()).toEqual(Object.keys(M16).sort());
    expect(Object.keys(LRS).sort()).toEqual(Object.keys(M16).sort());
    expect(fields(rust, "SampleOpen")).toEqual(Object.keys(OPENED).sort());
    expect(fields(rust, "SampleProgress")).toEqual(Object.keys(PROGRESS).sort());
  });
});
