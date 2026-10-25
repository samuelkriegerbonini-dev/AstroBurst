import { describe, expect, it } from "vitest";
import type { CatalogDatasetReport, CatalogInstallOutcome, CatalogInstallProgress, ZenodoOffer } from "../../services/catalogStore";
import {
  IDLE_INSTALL,
  TIER2_ONLINE_TEXT,
  USED_DOWNLOAD_NOTE,
  ZENODO_READING_TEXT,
  catalogRowView,
  catalogVersion,
  consentBlocker,
  consentText,
  installReducer,
  installedNote,
  isZenodoDownload,
  offersZenodo,
  registersZenodoDownload,
  removeConfirmText,
  removedNote,
  sortedCatalogRows,
  zenodoBlocker,
  zenodoConfirmLabel,
  zenodoConsentText,
  zenodoFileText,
  type InstallPhase,
} from "../catalogRows";

const MB = 1_000_000;

const TIER0: CatalogDatasetReport = {
  id: "tycho2-l6-v1",
  title: "Tycho-2 stars (bundled offline tier 0)",
  kind: "tier0_bundled",
  status: { state: "installed", path: "C:/Program Files/AstroBurst/resources/catalogs/tycho2-l6-v1.abcat", verified: true },
  size_bytes: 15_541_742,
  installable: false,
  compressed: "none",
  licence: "BSD-2-Clause (Tycho-2 as packaged by Debian)",
  credit: "Tycho-2 Catalogue, Høg et al. 2000, A&A 355, L27",
  url_host: null,
  folder: "C:/Users/u/AppData/Local/astroburst/catalogs/tycho2-l6-v1",
  folder_bytes: 0,
  free_space_bytes: 50_000 * MB,
};

const TIER1: CatalogDatasetReport = {
  id: "2mass-l8-v1",
  title: "2MASS star cut (hosted offline tier 1)",
  kind: "hosted",
  status: { state: "missing" },
  size_bytes: 154_140_794,
  installable: true,
  compressed: "none",
  licence: "Public domain (2MASS data products)",
  credit: "Two Micron All Sky Survey: University of Massachusetts and IPAC/Caltech, funded by NASA and NSF",
  url_host: "github.com",
  folder: "C:/Users/u/AppData/Local/astroburst/catalogs/2mass-l8-v1",
  folder_bytes: 0,
  free_space_bytes: 50_000 * MB,
};

const SIRIL: CatalogDatasetReport = {
  id: "siril-gaia-astro-v1",
  title: "Gaia DR3 astrometric extract from Siril (existing file, folder or download)",
  kind: "external",
  status: { state: "missing" },
  size_bytes: 1_142_276_749,
  installable: false,
  compressed: "bzip2",
  licence: "Siril catalogue format specification: CC BY 4.0; data: Gaia DR3, ESA/Gaia/DPAC",
  credit: "ESA/Gaia/DPAC; HEALPix extract published by the Siril project (Zenodo record 14692304)",
  url_host: "zenodo.org",
  folder: "C:/Users/u/AppData/Local/astroburst/catalogs/siril-gaia-astro-v1",
  folder_bytes: 0,
  free_space_bytes: 50_000 * MB,
};

const CACHE: CatalogDatasetReport = {
  id: "gaia-dr3-vizier-cache",
  title: "Gaia DR3 cone cache (online tier 2)",
  kind: "cache",
  status: { state: "cache", bytes: 3_250_000, files: 12, cap: 67_108_864 },
  size_bytes: null,
  installable: false,
  compressed: "none",
  licence: "Gaia DR3, ESA/Gaia/DPAC",
  credit: "ESA/Gaia/DPAC through VizieR (CDS, Strasbourg)",
  url_host: "vizier.cds.unistra.fr",
  folder: "C:/Users/u/AppData/Local/astroburst/cache/catalogs/gaia-dr3-vizier-cache",
  folder_bytes: 3_250_000,
  free_space_bytes: 50_000 * MB,
};

const running = (current: number, total: number, stage = "download"): CatalogInstallProgress => ({
  id: TIER1.id,
  current,
  total,
  percent: Math.floor((current * 100) / total),
  stage,
});

describe("catalogVersion", () => {
  it("reads the version suffix of a dataset id", () => {
    expect(catalogVersion("tycho2-l6-v1")).toBe("v1");
    expect(catalogVersion("2mass-l8-v12")).toBe("v12");
    expect(catalogVersion("custom")).toBeNull();
  });
});

describe("catalogRowView", () => {
  it("shows tier 0 as bundled with its version and size, with nothing to install or remove", () => {
    const row = catalogRowView(TIER0);
    expect(row).toMatchObject({ tier: 0, tierLabel: "Tier 0", version: "v1", sizeText: "15.5 MB", tone: "ok" });
    expect(row.stateText).toBe("Bundled with AstroBurst, SHA-256 verified");
    expect(Object.values(row.actions).some(Boolean)).toBe(false);
    expect(catalogRowView({ ...TIER0, status: { state: "installed", path: "x", verified: false } }).stateText).toBe(
      "Bundled with AstroBurst; its SHA-256 is checked on first use",
    );
  });

  it("says when this build has no bundled tier 0", () => {
    expect(catalogRowView({ ...TIER0, status: { state: "missing" } })).toMatchObject({
      stateText: "Not in this build: solving offline needs tier 1 or a Siril extract",
      tone: "warn",
    });
  });

  it("offers tier 1 for install with its size once it is published", () => {
    const row = catalogRowView(TIER1);
    expect(row).toMatchObject({ tier: 1, tierLabel: "Tier 1", sizeText: "154.1 MB", stateText: "Not installed" });
    expect(row.actions).toMatchObject({ install: true, cancel: false, remove: false });
    expect(row.installLabel).toBe("Download");
  });

  it("says tier 1 is not published yet and keeps the download closed", () => {
    const row = catalogRowView({ ...TIER1, installable: false, size_bytes: null });
    expect(row).toMatchObject({ stateText: "Not published yet: the download opens with the catalogs-v1 release", sizeText: "--" });
    expect(row.actions.install).toBe(false);
  });

  it("shows the running install with its progress and a cancel, and nothing else", () => {
    const phase = installReducer({ [TIER1.id]: { phase: "consent" } }, { type: "start", id: TIER1.id })[TIER1.id];
    const progressed = installReducer({ [TIER1.id]: phase }, { type: "progress", progress: running(64_700_000, 154_140_794) })[TIER1.id];
    const row = catalogRowView(TIER1, progressed);
    expect(row.stateText).toBe("Downloading 41 % (64.7 of 154.1 MB)");
    expect(row.percent).toBe(41);
    expect(row.actions).toEqual({ install: false, cancel: true, remove: false, chooseFolder: false, chooseFile: false });
  });

  it("offers to resume a partial download and to remove it", () => {
    const row = catalogRowView({ ...TIER1, status: { state: "partial", received: 12_300_000, total: 154_140_794 } });
    expect(row.stateText).toBe("Partial download: 12.3 of 154.1 MB; Download resumes it");
    expect(row.actions).toMatchObject({ install: true, remove: true });
    expect(row.installLabel).toBe("Resume");
  });

  it("offers Remove for an installed tier 1", () => {
    const row = catalogRowView({ ...TIER1, status: { state: "installed", path: "x", verified: true } });
    expect(row).toMatchObject({ stateText: "Installed, SHA-256 verified", tone: "ok", removeLabel: "Remove" });
    expect(row.actions).toMatchObject({ install: false, remove: true });
  });

  it("asks for an existing Siril folder or file and forgets it on request", () => {
    expect(catalogRowView(SIRIL)).toMatchObject({
      tier: 1,
      stateText: "Not set: choose your Siril catalogue folder or its siril_cat_healpix8_astro.dat file",
      actions: { install: false, chooseFolder: true, chooseFile: true, remove: false },
    });
    const valid = catalogRowView({ ...SIRIL, status: { state: "external", path: "D:/siril/x.dat", valid: true, problem: null } });
    expect(valid).toMatchObject({ stateText: "Using D:/siril/x.dat", tone: "ok", removeLabel: "Forget" });
    expect(valid.actions.remove).toBe(true);
    const invalid = catalogRowView({ ...SIRIL, status: { state: "external", path: "D:/x.dat", valid: false, problem: "it is HEALPix level 7" } });
    expect(invalid).toMatchObject({ stateText: "Cannot use D:/x.dat: it is HEALPix level 7", tone: "error" });
  });

  it("shows the tier 2 cache size against its cap and clears it", () => {
    const row = catalogRowView(CACHE);
    expect(row).toMatchObject({ tier: 2, tierLabel: "Tier 2", stateText: "Cache 3.3 MB in 12 files (cap 67.1 MB)", removeLabel: "Clear" });
    expect(row.actions.remove).toBe(true);
    expect(catalogRowView({ ...CACHE, status: { state: "cache", bytes: 0, files: 0, cap: 67_108_864 } }).actions.remove).toBe(false);
  });
});

describe("consentText", () => {
  it("names the host, the size, the licence, the credit and the free space before any download", () => {
    expect(consentText(TIER1)).toBe(
      "Download 2MASS star cut (hosted offline tier 1) from github.com: 154.1 MB. Licence: Public domain (2MASS data products). " +
        "Credit: Two Micron All Sky Survey: University of Massachusetts and IPAC/Caltech, funded by NASA and NSF. " +
        "Free space: 50.00 GB in C:/Users/u/AppData/Local/astroburst/catalogs/2mass-l8-v1. Only the catalogue is downloaded; no image leaves this computer.",
    );
  });

  it("says when the size or the free space is not known", () => {
    expect(consentText({ ...TIER1, size_bytes: null, free_space_bytes: null, url_host: null })).toContain(
      "from an unknown host: size unknown.",
    );
    expect(consentText({ ...TIER1, free_space_bytes: null })).toContain("Free space: unknown in ");
  });
});

describe("consentBlocker", () => {
  it("blocks a download that does not fit", () => {
    expect(consentBlocker(TIER1)).toBeNull();
    expect(consentBlocker({ ...TIER1, free_space_bytes: 100 * MB })).toBe("Needs 154.1 MB free; 100.0 MB available");
    expect(consentBlocker({ ...TIER1, free_space_bytes: null })).toBeNull();
  });
});

describe("installReducer", () => {
  const id = TIER1.id;
  const at = (phase: InstallPhase) => ({ [id]: phase });

  it("never starts an install without the consent step", () => {
    expect(installReducer({}, { type: "start", id })).toEqual({});
    expect(installReducer(at(IDLE_INSTALL), { type: "start", id })).toEqual(at(IDLE_INSTALL));
    const asked = installReducer({}, { type: "ask", id });
    expect(asked).toEqual(at({ phase: "consent" }));
    expect(installReducer(asked, { type: "start", id })[id]).toMatchObject({ phase: "running", cancelling: false, percent: 0 });
    expect(installReducer(asked, { type: "start", id })[id]).not.toHaveProperty("adopted", true);
    expect(installReducer(asked, { type: "decline", id })).toEqual(at({ phase: "idle", returnFocus: true }));
  });

  it("applies progress to the running install of the same dataset", () => {
    const started = installReducer(at({ phase: "consent" }), { type: "start", id });
    const next = installReducer(started, { type: "progress", progress: running(50, 200, "decode") });
    expect(next[id]).toMatchObject({ phase: "running", current: 50, total: 200, percent: 25, stage: "decode" });
    expect(installReducer(started, { type: "progress", progress: { ...running(50, 200), id: "other" } })[id]).toBe(started[id]);
  });

  it("adopts a download it did not start, so a reopened Settings shows its progress and Cancel", () => {
    const progress = { type: "progress", progress: running(64_700_000, 154_140_794) } as const;
    const adopted = installReducer(at(IDLE_INSTALL), progress)[id];
    expect(adopted).toEqual({
      phase: "running",
      stage: "download",
      current: 64_700_000,
      total: 154_140_794,
      percent: 41,
      cancelling: false,
      adopted: true,
    });
    expect(installReducer({}, progress)[id]).toMatchObject({ phase: "running", adopted: true });
    expect(installReducer(at({ phase: "failed", message: "x" }), progress)[id]).toMatchObject({ phase: "running", adopted: true });
    const consenting = at({ phase: "consent" });
    expect(installReducer(consenting, progress)).toBe(consenting);
    const row = catalogRowView({ ...TIER1, status: { state: "partial", received: 12_300_000, total: 154_140_794 } }, adopted);
    expect(row.stateText).toBe("Downloading 41 % (64.7 of 154.1 MB)");
    expect(row.actions).toEqual({ install: false, cancel: true, remove: false, chooseFolder: false, chooseFile: false });
  });

  it("ends a running install on the complete event and leaves a finished one as it is", () => {
    const complete = { type: "progress", progress: running(154_140_794, 154_140_794, "complete") } as const;
    const started = installReducer(at({ phase: "consent" }), { type: "start", id });
    expect(installReducer(started, complete)).toEqual(at(IDLE_INSTALL));
    const adopted = installReducer({}, { type: "progress", progress: running(10, 200) });
    expect(installReducer(adopted, complete)).toEqual(at(IDLE_INSTALL));
    const finished = at({ phase: "idle", note: "Installed 154.1 MB, SHA-256 verified." });
    expect(installReducer(finished, complete)).toBe(finished);
    expect(installReducer({}, complete)).toEqual({});
  });

  it("turns the cancelled error of a cancelled install into a resumable note, and any other error into a failure", () => {
    const started = installReducer(at({ phase: "consent" }), { type: "start", id });
    const cancelling = installReducer(started, { type: "cancel", id });
    expect(cancelling[id]).toMatchObject({ phase: "running", cancelling: true });
    expect(installReducer(cancelling, { type: "failed", id, message: "cancelled" })).toEqual(
      at({ phase: "idle", note: "Cancelled; what was downloaded is kept and the next download resumes from there." }),
    );
    expect(installReducer(started, { type: "failed", id, message: "HTTP 404" })).toEqual(at({ phase: "failed", message: "HTTP 404" }));
    expect(installReducer(started, { type: "finished", id, note: "Installed" })).toEqual(at({ phase: "idle", note: "Installed" }));
  });

  it("lets a failed install be asked for again", () => {
    expect(installReducer(at({ phase: "failed", message: "x" }), { type: "ask", id })).toEqual(at({ phase: "consent" }));
  });
});

describe("removeConfirmText", () => {
  it("names what Remove and Clear delete before they do it", () => {
    expect(removeConfirmText({ ...TIER1, status: { state: "installed", path: "x", verified: true } })).toBe("Confirm: deletes 154.1 MB");
    expect(removeConfirmText({ ...TIER1, status: { state: "partial", received: 64_700_000, total: 154_140_794 } })).toBe(
      "Confirm: deletes 64.7 MB",
    );
    expect(removeConfirmText({ ...SIRIL, status: { state: "installed", path: "x", verified: true } })).toBe("Confirm: deletes 1.14 GB");
    expect(removeConfirmText(CACHE)).toBe("Confirm: clears 3.3 MB");
    expect(removeConfirmText({ ...TIER1, size_bytes: null, status: { state: "installed", path: "x", verified: true } })).toBe(
      "Confirm: deletes the downloaded files",
    );
  });

  it("asks nothing before Forget, which keeps the external file", () => {
    expect(removeConfirmText({ ...SIRIL, status: { state: "external", path: "D:/x.dat", valid: true, problem: null } })).toBeNull();
  });
});

describe("sortedCatalogRows", () => {
  it("orders the rows by tier, the hosted tier 1 before the Siril extract", () => {
    expect(sortedCatalogRows([CACHE, SIRIL, TIER1, TIER0]).map((r) => r.id)).toEqual([TIER0.id, TIER1.id, SIRIL.id, CACHE.id]);
  });
});

describe("TIER2_ONLINE_TEXT", () => {
  it("says only the star list goes over the network", () => {
    expect(TIER2_ONLINE_TEXT).toMatch(/only the star list/i);
    expect(TIER2_ONLINE_TEXT).toMatch(/image never leaves/i);
  });
});

describe("install and remove notes", () => {
  it("states what was installed and whether it resumed", () => {
    const outcome = { id: TIER1.id, path: "x", bytes: 154_140_794, resumed_from: 0, sha256_hex: "c9ab", sha256_verified: true, already_installed: false };
    expect(installedNote(outcome)).toBe("Installed 154.1 MB, SHA-256 verified.");
    expect(installedNote({ ...outcome, resumed_from: 77_000_000 })).toBe("Installed 154.1 MB, SHA-256 verified; resumed after 77.0 MB.");
    expect(installedNote({ ...outcome, already_installed: true })).toBe("Already installed and verified.");
  });

  it("says a forgotten external file is kept on disk", () => {
    expect(removedNote({ id: TIER1.id, freed_bytes: 154_140_794, forgot_external: false })).toBe("Removed 154.1 MB.");
    expect(removedNote({ id: SIRIL.id, freed_bytes: 0, forgot_external: true })).toBe("Forgotten; the file itself is kept.");
    expect(removedNote({ id: CACHE.id, freed_bytes: 0, forgot_external: false })).toBe("Nothing to remove.");
  });
});

const SIRIL_ZENODO: CatalogDatasetReport = { ...SIRIL, zenodo_record: "14692304" };

const OFFER: ZenodoOffer = {
  id: SIRIL.id,
  record_url: "https://zenodo.org/api/records/14692304",
  record_page: "https://zenodo.org/records/14692304",
  doi: "10.5281/zenodo.14692304",
  title: "Siril Astrometry Catalogue extracted from Gaia DR3",
  record_licence: "cc-by-4.0",
  gaia_terms:
    "Gaia-derived data: Gaia DR3 (ESA/Gaia/DPAC), whose terms are CC BY-NC 3.0 IGO; the Zenodo record labels this Siril extract CC BY 4.0.",
  host: "zenodo.org",
  files: [
    { key: "uncompressed_files.sha256sum", size: 95, checksum: "md5:02e655b1d8b8db05fa561bc356dadfc8" },
    { key: "siril_cat_healpix8_astro.dat.bz2", size: 1_142_276_749, checksum: "md5:2e62f2365023e621afbd336c82980208" },
    { key: "siril_cat_healpix8_astro.dat.bz2.sha256sum", size: 99, checksum: "md5:ba889d7b455f857660096ea445a13953" },
  ],
  archive_key: "siril_cat_healpix8_astro.dat.bz2",
  archive_url: "https://zenodo.org/api/records/14692304/files/siril_cat_healpix8_astro.dat.bz2/content",
  archive_bytes: 1_142_276_749,
  archive_sha256: "846ad4b12c50865df0cb8c5b23453f22eec78bbe9969e17d669ae19eb49d421f",
  decoded_sha256: "2fa40c93fe115235d35c5050757f2ef60a326a6f3030f87be1598c016fcb2388",
  decoded_max_bytes: 1_601_175_680,
  resume_from: 0,
  need_bytes: 2_743_452_429,
  free_space_bytes: 50_000 * MB,
  folder: SIRIL.folder,
};

describe("the Siril Gaia extract from Zenodo", () => {
  it("offers the Zenodo download beside the Siril folder and file when this build can download", () => {
    const row = catalogRowView(SIRIL_ZENODO);
    expect(row.installLabel).toBe("Download the Siril Gaia extract (Zenodo, about 1.14 GB)");
    expect(row.actions).toMatchObject({ install: true, chooseFolder: true, chooseFile: true, remove: false });
    expect(row.stateText).toContain("download the Siril Gaia extract from Zenodo");
    expect(offersZenodo(SIRIL_ZENODO)).toBe(true);
    expect(offersZenodo(SIRIL)).toBe(false);
    expect(offersZenodo({ ...TIER1, zenodo_record: "14692304" })).toBe(false);
  });

  it("resumes a partial Zenodo download and offers to use an unpacked one", () => {
    const partial = catalogRowView({ ...SIRIL_ZENODO, status: { state: "partial", received: 571_138_374, total: 1_142_276_749 } });
    expect(partial).toMatchObject({ installLabel: "Resume the Zenodo download" });
    expect(partial.actions).toMatchObject({ install: true, remove: true, chooseFolder: true });
    const unpacked = catalogRowView({ ...SIRIL_ZENODO, status: { state: "installed", path: `${SIRIL.folder}/siril_cat_healpix8_astro.dat`, verified: true } });
    expect(unpacked.installLabel).toBe("Use the Zenodo download");
    expect(unpacked.actions.install).toBe(true);
  });

  it("removes the Zenodo download with a confirm, but forgets a Siril folder of the user's own at once", () => {
    const downloaded: CatalogDatasetReport = {
      ...SIRIL_ZENODO,
      folder: "C:\\Users\\u\\AppData\\Local\\astroburst\\catalogs\\siril-gaia-astro-v1",
      status: {
        state: "external",
        path: "C:\\Users\\u\\AppData\\Local\\astroburst\\catalogs\\siril-gaia-astro-v1\\siril_cat_healpix8_astro.dat",
        valid: true,
        problem: null,
      },
    };
    expect(isZenodoDownload(downloaded)).toBe(true);
    const row = catalogRowView(downloaded);
    expect(row.stateText).toContain("Using the Zenodo download in ");
    expect(row.stateText).toBe("Using the Zenodo download in C:\\Users\\u\\AppData\\Local\\astroburst\\catalogs\\siril-gaia-astro-v1");
    expect(row).toMatchObject({ removeLabel: "Remove", tone: "ok" });
    expect(row.actions).toMatchObject({ install: false, remove: true });
    expect(removeConfirmText(downloaded)).toBe("Confirm: deletes the Zenodo download");
    const own: CatalogDatasetReport = { ...SIRIL_ZENODO, status: { state: "external", path: "D:/siril/siril_cat_healpix8_astro.dat", valid: true, problem: null } };
    expect(isZenodoDownload(own)).toBe(false);
    expect(catalogRowView(own)).toMatchObject({ removeLabel: "Forget", actions: { install: false } });
    expect(removeConfirmText(own)).toBeNull();
    const sibling: CatalogDatasetReport = {
      ...SIRIL_ZENODO,
      status: { state: "external", path: `${SIRIL.folder}-old/siril_cat_healpix8_astro.dat`, valid: true, problem: null },
    };
    expect(isZenodoDownload(sibling)).toBe(false);
    const broken: CatalogDatasetReport = { ...SIRIL_ZENODO, status: { state: "external", path: "D:/x.dat", valid: false, problem: "it is HEALPix level 7" } };
    expect(catalogRowView(broken).actions).toMatchObject({ install: true, remove: true });
  });

  it("keeps the Zenodo consent open until the record is read, and records what it read", () => {
    const id = SIRIL.id;
    const asked = installReducer({}, { type: "ask", id });
    expect(zenodoBlocker(asked[id])).toBe(ZENODO_READING_TEXT);
    const offered = installReducer(asked, { type: "offer", id, offer: OFFER });
    expect(offered[id]).toEqual({ phase: "consent", offer: OFFER });
    expect(zenodoBlocker(offered[id])).toBeNull();
    const failed = installReducer(asked, { type: "offerFailed", id, message: "HTTP 503" });
    expect(zenodoBlocker(failed[id])).toBe("Cannot read the Zenodo record: HTTP 503");
    expect(installReducer({}, { type: "offer", id, offer: OFFER })).toEqual({});
    const tight = installReducer(asked, { type: "offer", id, offer: { ...OFFER, free_space_bytes: 2_000 * MB } });
    expect(zenodoBlocker(tight[id])).toBe("Needs 2.74 GB free; 2.00 GB available");
    expect(installReducer(offered, { type: "start", id })[id]).toMatchObject({ phase: "running", stage: "download" });
  });

  it("names the source, the size, the checksum, the unpacked size, the folder and the Gaia terms before the download", () => {
    const text = zenodoConsentText(OFFER);
    for (const needle of [
      "siril_cat_healpix8_astro.dat.bz2 (1.14 GB, bzip2)",
      "Zenodo record https://zenodo.org/records/14692304 (DOI 10.5281/zenodo.14692304)",
      "Zenodo publishes its SHA-256, which is checked after the download",
      "at most 1.60 GB in C:/Users/u/AppData/Local/astroburst/catalogs/siril-gaia-astro-v1",
      "becomes your Siril catalogue folder",
      "Needs 2.74 GB free; 50.00 GB available",
      "CC BY-NC 3.0 IGO",
      "no image leaves this computer",
    ]) {
      expect(text).toContain(needle);
    }
    expect(zenodoConsentText({ ...OFFER, archive_sha256: null })).toContain("Zenodo publishes no SHA-256 for it, so only its size is checked");
    expect(zenodoConsentText({ ...OFFER, resume_from: 571_138_374 })).toContain("571.1 MB are already here and the download resumes from there.");
    expect(zenodoFileText(OFFER.files[1])).toBe("siril_cat_healpix8_astro.dat.bz2 · 1.14 GB · md5:2e62f2365023e621afbd336c82980208");
    expect(zenodoConfirmLabel({ phase: "consent" })).toBe("Accept the terms and download");
    expect(zenodoConfirmLabel({ phase: "consent", offer: OFFER })).toBe("Accept the terms and download 1.14 GB");
    expect(zenodoConfirmLabel({ phase: "consent", offer: { ...OFFER, resume_from: 571_138_374 } })).toBe("Accept the terms and download 571.1 MB");
  });

  it("shows the bytes written while decoding a catalogue of unknown decoded size", () => {
    const decoding: InstallPhase = { phase: "running", stage: "decode", current: 12_300_000, total: 0, percent: 0, cancelling: false };
    const row = catalogRowView(SIRIL_ZENODO, decoding);
    expect(row.stateText).toBe("Decoding: 12.3 MB written");
    expect(row.percent).toBeNull();
    expect(row.actions.cancel).toBe(true);
  });

  it("says when only the size was checked and that the download is now the Siril folder", () => {
    const outcome = { id: SIRIL.id, path: "x", bytes: 1_450_000_000, resumed_from: 0, sha256_hex: "2fa4", sha256_verified: false, already_installed: false };
    expect(installedNote(outcome)).toBe("Installed 1.45 GB, size checked (Zenodo published no SHA-256). It is now your Siril catalogue folder.");
    expect(installedNote({ ...outcome, sha256_verified: true })).toBe("Installed 1.45 GB, SHA-256 verified. It is now your Siril catalogue folder.");
  });
});

describe("the Siril row after the review of the Zenodo download", () => {
  const ON_DISK = 1_601_175_680;
  const DOWNLOADED = `${SIRIL.folder}/siril_cat_healpix8_astro.dat`;
  const installed: CatalogDatasetReport = { ...SIRIL_ZENODO, folder_bytes: ON_DISK, status: { state: "installed", path: DOWNLOADED, verified: true } };

  it("shows a Zenodo download kept beside the user's own folder, and Forget deletes nothing", () => {
    const own: CatalogDatasetReport = {
      ...SIRIL_ZENODO,
      folder_bytes: ON_DISK,
      status: { state: "external", path: "D:/Siril/siril_cat_healpix8_astro.dat", valid: true, problem: null },
    };
    const row = catalogRowView(own);
    expect(row.stateText).toBe(`Using D:/Siril/siril_cat_healpix8_astro.dat. The download in ${SIRIL.folder} (1.60 GB) stays on disk; Forget keeps it.`);
    expect(row.removeLabel).toBe("Forget");
    expect(removeConfirmText(own)).toBeNull();
    expect(catalogRowView({ ...own, folder_bytes: 0 }).stateText).toBe("Using D:/Siril/siril_cat_healpix8_astro.dat");
    const broken: CatalogDatasetReport = { ...own, status: { state: "external", path: "D:/gone.dat", valid: false, problem: "the file does not exist" } };
    expect(catalogRowView(broken).stateText).toContain("stays on disk; Forget keeps it.");
  });

  it("labels the delete of a download that is not the Siril folder Remove and confirms its size on disk", () => {
    expect(catalogRowView(installed).removeLabel).toBe("Remove");
    expect(removeConfirmText(installed)).toBe("Confirm: deletes 1.60 GB");
    const partial: CatalogDatasetReport = { ...installed, status: { state: "partial", received: 571_138_374, total: 1_142_276_749 } };
    expect(catalogRowView(partial).removeLabel).toBe("Remove");
    const registered: CatalogDatasetReport = { ...installed, status: { state: "external", path: DOWNLOADED, valid: true, problem: null } };
    expect(removeConfirmText(registered)).toBe("Confirm: deletes the Zenodo download (1.60 GB)");
  });

  it("says what was deleted and what was forgotten, also when both happened", () => {
    expect(removedNote({ id: SIRIL.id, freed_bytes: ON_DISK, forgot_external: true })).toBe(
      "Removed 1.60 GB; your own file or folder is forgotten and kept on disk.",
    );
    expect(removedNote({ id: SIRIL.id, freed_bytes: ON_DISK, forgot_external: false })).toBe("Removed 1.60 GB.");
  });

  it("uses a verified download already on disk without a consent, and checks an unverified one against Zenodo first", () => {
    expect(registersZenodoDownload(installed)).toBe(true);
    expect(catalogRowView(installed)).toMatchObject({ installLabel: "Use the Zenodo download", actions: { install: true } });
    expect(registersZenodoDownload({ ...installed, status: { state: "installed", path: DOWNLOADED, verified: false } })).toBe(false);
    expect(registersZenodoDownload({ ...installed, status: { state: "partial", received: 1, total: 2 } })).toBe(false);
    expect(registersZenodoDownload({ ...installed, zenodo_record: null })).toBe(false);
    expect(USED_DOWNLOAD_NOTE).toBe("Using the verified Zenodo download; it is now your Siril catalogue folder.");
  });

  it("names the host every byte comes from and the licence the record lists", () => {
    const text = zenodoConsentText({ ...OFFER, host: "127.0.0.1" });
    expect(text).toContain("from 127.0.0.1, Zenodo record https://zenodo.org/records/14692304 (DOI 10.5281/zenodo.14692304)");
    expect(text).toContain("The Zenodo record lists its licence as cc-by-4.0.");
    expect(zenodoConsentText({ ...OFFER, record_licence: null })).toContain("The Zenodo record lists no licence.");
  });

  it("never reports an outcome without its checksum flag as SHA-256 verified", () => {
    const outcome = { id: SIRIL.id, path: "x", bytes: 1_450_000_000, resumed_from: 0, sha256_hex: "2fa4", already_installed: false };
    expect(installedNote(outcome as unknown as CatalogInstallOutcome)).not.toContain("SHA-256 verified");
  });
});
