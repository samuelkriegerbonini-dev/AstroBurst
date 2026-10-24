import { describe, expect, it } from "vitest";
import type { CatalogDatasetReport, CatalogInstallProgress } from "../../services/catalogStore";
import {
  IDLE_INSTALL,
  TIER2_ONLINE_TEXT,
  catalogRowView,
  catalogVersion,
  consentBlocker,
  consentText,
  installReducer,
  installedNote,
  removeConfirmText,
  removedNote,
  sortedCatalogRows,
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
    const outcome = { id: TIER1.id, path: "x", bytes: 154_140_794, resumed_from: 0, sha256_hex: "c9ab", already_installed: false };
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
