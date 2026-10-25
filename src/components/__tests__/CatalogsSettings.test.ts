import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, isValidElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { CatalogDatasetReport, CatalogInstallProgress, ZenodoOffer } from "../../services/catalogStore";
import { USED_DOWNLOAD_NOTE, type InstallPhase } from "../../utils/catalogRows";

const typedInvokeMock = vi.hoisted(() => vi.fn());

vi.mock("react", async (importOriginal) => (await import("../../test/hookRuntime")).reactWithFakeHooks(await importOriginal()));
vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, isTauri: () => true }));

import { hooks } from "../../test/hookRuntime";
import { createCatalogStore, type CatalogStore, type CatalogStoreApi } from "../../hooks/useCatalogStore";
import { updateConfig } from "../../services/config";
import CatalogsSettings, { CATALOG_CREDITS, CatalogsSection, type CatalogsSectionProps } from "../CatalogsSettings";

const MB = 1_000_000;

const TIER0: CatalogDatasetReport = {
  id: "tycho2-l6-v1",
  title: "Tycho-2 stars (bundled offline tier 0)",
  kind: "tier0_bundled",
  status: { state: "installed", path: "C:/AstroBurst/resources/catalogs/tycho2-l6-v1.abcat", verified: true },
  size_bytes: 15_541_742,
  installable: false,
  compressed: "none",
  licence: "BSD-2-Clause (Tycho-2 as packaged by Debian)",
  credit: "Tycho-2 Catalogue, Høg et al. 2000, A&A 355, L27",
  url_host: null,
  folder: "C:/data/catalogs/tycho2-l6-v1",
  folder_bytes: 0,
  free_space_bytes: 50_000 * MB,
};

const TIER1: CatalogDatasetReport = {
  ...TIER0,
  id: "2mass-l8-v1",
  title: "2MASS star cut (hosted offline tier 1)",
  kind: "hosted",
  status: { state: "missing" },
  size_bytes: 154_140_794,
  installable: true,
  licence: "Public domain (2MASS data products)",
  credit: "Two Micron All Sky Survey: University of Massachusetts and IPAC/Caltech, funded by NASA and NSF",
  url_host: "github.com",
  folder: "C:/data/catalogs/2mass-l8-v1",
};

const SIRIL: CatalogDatasetReport = {
  ...TIER0,
  id: "siril-gaia-astro-v1",
  title: "Gaia DR3 astrometric extract from Siril (existing file, folder or download)",
  kind: "external",
  status: { state: "missing" },
  size_bytes: 1_142_276_749,
  compressed: "bzip2",
  licence: "Siril catalogue format specification: CC BY 4.0; data: Gaia DR3, ESA/Gaia/DPAC",
  credit: "ESA/Gaia/DPAC; HEALPix extract published by the Siril project (Zenodo record 14692304)",
  url_host: "zenodo.org",
};

const CACHE: CatalogDatasetReport = {
  ...TIER0,
  id: "gaia-dr3-vizier-cache",
  title: "Gaia DR3 cone cache (online tier 2)",
  kind: "cache",
  status: { state: "cache", bytes: 3_250_000, files: 12, cap: 67_108_864 },
  size_bytes: null,
};

function props(over: Partial<CatalogsSectionProps> = {}): CatalogsSectionProps {
  return {
    reports: [TIER0, { ...TIER1, installable: false, size_bytes: null }, SIRIL],
    installs: {},
    loading: false,
    error: null,
    onAsk: vi.fn(),
    onDecline: vi.fn(),
    onConfirm: vi.fn(),
    onCancel: vi.fn(),
    onRemove: vi.fn(),
    onChooseFolder: vi.fn(),
    onChooseFile: vi.fn(),
    onRefresh: vi.fn(),
    armedRemove: null,
    onDisarm: vi.fn(),
    ...over,
  };
}

const render = (over: Partial<CatalogsSectionProps> = {}) => renderToStaticMarkup(createElement(CatalogsSection, props(over)));

function rowMarkup(markup: string, id: string): string {
  const start = markup.indexOf(`data-catalog-row="${id}"`);
  expect(start, `row ${id}`).toBeGreaterThanOrEqual(0);
  const next = markup.indexOf('data-catalog-row="', start + 1);
  return markup.slice(start, next < 0 ? undefined : next);
}

function buttonTags(markup: string): string[] {
  return markup.match(/<button[^>]*>(?:(?!<\/button>)[\s\S])*<\/button>/g) ?? [];
}

function buttons(markup: string): string[] {
  return buttonTags(markup).map((b) => b.replace(/<[^>]+>/g, ""));
}

function autofocused(markup: string): string[] {
  return buttonTags(markup)
    .filter((b) => /^<button[^>]*\sautofocus=""/.test(b))
    .map((b) => b.replace(/<[^>]+>/g, ""));
}

const SIRIL_ZENODO: CatalogDatasetReport = { ...SIRIL, zenodo_record: "14692304" };

const OFFER: ZenodoOffer = {
  id: SIRIL.id,
  record_url: "https://zenodo.org/api/records/14692304",
  record_page: "https://zenodo.org/records/14692304",
  doi: "10.5281/zenodo.14692304",
  title: "Siril Astrometry Catalogue extracted from Gaia DR3",
  record_licence: "cc-by-4.0",
  gaia_terms: "Gaia-derived data: Gaia DR3 (ESA/Gaia/DPAC), whose terms are CC BY-NC 3.0 IGO.",
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
  folder: "C:/data/catalogs/siril-gaia-astro-v1",
};

const INSTALLED_ZENODO: CatalogDatasetReport = {
  ...SIRIL_ZENODO,
  folder: OFFER.folder,
  folder_bytes: 1_601_175_680,
  status: { state: "installed", path: `${OFFER.folder}/siril_cat_healpix8_astro.dat`, verified: true },
};

const INSTALLED_TIER1: CatalogDatasetReport = { ...TIER1, status: { state: "installed", path: "C:/data/catalogs/2mass-l8-v1/x.abcat", verified: true } };

describe("Settings > Catalogs", () => {
  it("lists tier 0 bundled with its version and size", () => {
    const row = rowMarkup(render(), TIER0.id);
    expect(row).toContain("Tier 0");
    expect(row).toContain("v1");
    expect(row).toContain("15.5 MB");
    expect(row).toContain("Bundled with AstroBurst, SHA-256 verified");
    expect(row).toContain(TIER0.licence);
    expect(buttons(row)).toEqual([]);
  });

  it("lists tier 1 as not published yet with no download button", () => {
    const row = rowMarkup(render(), TIER1.id);
    expect(row).toContain("Tier 1");
    expect(row).toContain("Not published yet: the download opens with the catalogs-v1 release");
    expect(buttons(row)).toEqual([]);
  });

  it("offers the download once published, and shows the consent text with its own confirm before any install", () => {
    const offered = rowMarkup(render({ reports: [TIER1] }), TIER1.id);
    expect(buttons(offered)).toEqual(["Download"]);
    expect(offered).not.toContain("role=\"dialog\"");
    const consenting = rowMarkup(render({ reports: [TIER1], installs: { [TIER1.id]: { phase: "consent" } } }), TIER1.id);
    expect(consenting).toContain('role="dialog"');
    expect(consenting).toContain("from github.com: 154.1 MB");
    expect(consenting).toContain("Public domain (2MASS data products)");
    expect(buttons(consenting)).toEqual(["Download 154.1 MB", "Not now"]);
  });

  it("disables the confirm when the download does not fit", () => {
    const markup = rowMarkup(
      render({ reports: [{ ...TIER1, free_space_bytes: 100 * MB }], installs: { [TIER1.id]: { phase: "consent" } } }),
      TIER1.id,
    );
    expect(markup).toContain("Needs 154.1 MB free; 100.0 MB available");
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Download 154\.1 MB<\/button>/);
  });

  it("shows the progress of a running install with a Cancel button", () => {
    const phase: InstallPhase = { phase: "running", stage: "download", current: 64_700_000, total: 154_140_794, percent: 41, cancelling: false };
    const row = rowMarkup(render({ reports: [TIER1], installs: { [TIER1.id]: phase } }), TIER1.id);
    expect(row).toContain('role="progressbar"');
    expect(row).toContain('aria-valuenow="41"');
    expect(row).toContain("Downloading 41 % (64.7 of 154.1 MB)");
    expect(buttons(row)).toEqual(["Cancel"]);
  });

  it("offers an existing Siril folder or file", () => {
    expect(buttons(rowMarkup(render(), SIRIL.id))).toEqual(["Use Siril folder", "Use file"]);
  });

  it("offers the Zenodo download of the Siril extract with its size, beside the folder and file", () => {
    expect(buttons(rowMarkup(render({ reports: [SIRIL_ZENODO] }), SIRIL.id))).toEqual([
      "Download the Siril Gaia extract (Zenodo, about 1.14 GB)",
      "Use Siril folder",
      "Use file",
    ]);
  });

  it("keeps the Zenodo confirm closed until the record is read, then shows its files, the terms and the size", () => {
    const reading = rowMarkup(render({ reports: [SIRIL_ZENODO], installs: { [SIRIL.id]: { phase: "consent" } } }), SIRIL.id);
    expect(reading).toContain("Reading the file list from the Zenodo record...");
    expect(reading).toMatch(/<button[^>]*disabled=""[^>]*>Accept the terms and download<\/button>/);
    expect(autofocused(reading)).toEqual(["Not now"]);
    const offered = rowMarkup(render({ reports: [SIRIL_ZENODO], installs: { [SIRIL.id]: { phase: "consent", offer: OFFER } } }), SIRIL.id);
    expect(offered).toContain('aria-label="Files in the Zenodo record"');
    expect(offered.match(/<li>/g) ?? []).toHaveLength(3);
    expect(offered).toContain("siril_cat_healpix8_astro.dat.bz2 · 1.14 GB · md5:2e62f2365023e621afbd336c82980208");
    expect(offered).toContain("CC BY-NC 3.0 IGO");
    expect(offered).toContain("Zenodo publishes its SHA-256");
    expect(buttons(offered)).toEqual(["Accept the terms and download 1.14 GB", "Not now", "Use Siril folder", "Use file"]);
    expect(offered).not.toMatch(/disabled=""[^>]*>Accept/);
  });

  it("shows tier 2 online Gaia with the cache size and a Clear button", () => {
    const row = rowMarkup(render({ reports: [TIER0, CACHE] }), CACHE.id);
    expect(row).toContain("Tier 2");
    expect(row).toContain("Cache 3.3 MB in 12 files (cap 67.1 MB)");
    expect(row).toContain("only the star list");
    expect(buttons(row)).toEqual(["Clear"]);
  });

  it("still names tier 2 when this build reports no cache row", () => {
    const markup = render();
    const row = rowMarkup(markup, "tier2-online");
    expect(row).toContain("Gaia DR3 online");
    expect(row).toContain("the cache size is not reported by this build");
    expect(buttons(row)).toEqual([]);
  });

  it("credits every catalogue", () => {
    const markup = render();
    expect(CATALOG_CREDITS).toContain("Høg et al. 2000");
    expect(CATALOG_CREDITS).toContain("2MASS");
    expect(CATALOG_CREDITS).toContain("ESA/Gaia/DPAC");
    expect(CATALOG_CREDITS).toContain("CC BY 4.0");
    expect(markup).toContain(CATALOG_CREDITS.replace(/&/g, "&amp;"));
  });

  it("shows the status error and a loading line", () => {
    expect(render({ reports: null, loading: true })).toContain("Reading the catalogue status");
    expect(render({ error: "the data folder is not reachable" })).toContain("the data folder is not reachable");
  });

  it("moves focus into the consent step, to Not now when the download does not fit, and back to Download when declined", () => {
    const consenting = rowMarkup(render({ reports: [TIER1], installs: { [TIER1.id]: { phase: "consent" } } }), TIER1.id);
    expect(autofocused(consenting)).toEqual(["Download 154.1 MB"]);
    const described = consenting.match(/role="dialog"[^>]*aria-describedby="([^"]+)"/);
    expect(described, "dialog described by the consent text").not.toBeNull();
    const describedId = described![1].replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    expect(consenting).toMatch(new RegExp(`<p[^>]*id="${describedId}"[^>]*>Download 2MASS`));
    const blocked = rowMarkup(
      render({ reports: [{ ...TIER1, free_space_bytes: 100 * MB }], installs: { [TIER1.id]: { phase: "consent" } } }),
      TIER1.id,
    );
    expect(autofocused(blocked)).toEqual(["Not now"]);
    const declined = rowMarkup(render({ reports: [TIER1], installs: { [TIER1.id]: { phase: "idle", returnFocus: true } } }), TIER1.id);
    expect(autofocused(declined)).toEqual(["Download"]);
    expect(autofocused(rowMarkup(render({ reports: [TIER1] }), TIER1.id))).toEqual([]);
  });

  it("moves focus to Cancel when the confirmed download starts, but not for a download adopted after a reopen", () => {
    const started: InstallPhase = { phase: "running", stage: "download", current: 0, total: 0, percent: 0, cancelling: false };
    expect(autofocused(rowMarkup(render({ reports: [TIER1], installs: { [TIER1.id]: started } }), TIER1.id))).toEqual(["Cancel"]);
    const adopted: InstallPhase = { ...started, adopted: true };
    expect(autofocused(rowMarkup(render({ reports: [TIER1], installs: { [TIER1.id]: adopted } }), TIER1.id))).toEqual([]);
  });

  it("labels the delete of a Zenodo download that is not the Siril folder Remove, with its size on disk", () => {
    expect(buttons(rowMarkup(render({ reports: [INSTALLED_ZENODO] }), SIRIL.id))).toEqual([
      "Use the Zenodo download",
      "Use Siril folder",
      "Use file",
      "Remove",
    ]);
    expect(buttons(rowMarkup(render({ reports: [INSTALLED_ZENODO], armedRemove: SIRIL.id }), SIRIL.id))).toContain("Confirm: deletes 1.60 GB");
  });

  it("arms Remove with what it deletes before deleting", () => {
    expect(buttons(rowMarkup(render({ reports: [INSTALLED_TIER1] }), TIER1.id))).toEqual(["Remove"]);
    expect(buttons(rowMarkup(render({ reports: [INSTALLED_TIER1], armedRemove: TIER1.id }), TIER1.id))).toEqual([
      "Confirm: deletes 154.1 MB",
    ]);
    expect(buttons(rowMarkup(render({ reports: [TIER0, CACHE], armedRemove: CACHE.id }), CACHE.id))).toEqual(["Confirm: clears 3.3 MB"]);
  });
});

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const CANCELLED_NOTE = "Cancelled; what was downloaded is kept and the next download resumes from there.";

function fakeApi(rows: CatalogDatasetReport[]) {
  const listener = { current: null as ((progress: CatalogInstallProgress) => void) | null };
  const api = {
    status: vi.fn(async () => rows),
    install: vi.fn<CatalogStoreApi["install"]>(),
    cancel: vi.fn(async () => true),
    remove: vi.fn<CatalogStoreApi["remove"]>(async (id: string) => ({ id, freed_bytes: 154_140_794, forgot_external: false })),
    offer: vi.fn<CatalogStoreApi["offer"]>(),
    useSirilDir: vi.fn<CatalogStoreApi["useSirilDir"]>(),
    useFile: vi.fn<CatalogStoreApi["useFile"]>(),
    onProgress: vi.fn(async (cb: (progress: CatalogInstallProgress) => void) => {
      listener.current = cb;
      return () => {};
    }),
  } satisfies CatalogStoreApi;
  return { api, listener };
}

async function startedStore(rows: CatalogDatasetReport[]) {
  const fake = fakeApi(rows);
  const store = createCatalogStore(fake.api);
  store.start();
  await flush();
  return { store, ...fake };
}

describe("Settings > Catalogs store", () => {
  it("reads the rows each time Settings opens and follows the install progress with one listener", async () => {
    const { store, api, listener } = await startedStore([TIER0, INSTALLED_TIER1]);
    store.start();
    await flush();
    expect(api.status).toHaveBeenCalledTimes(2);
    expect(api.onProgress).toHaveBeenCalledTimes(1);
    expect(listener.current).not.toBeNull();
    expect(store.get()).toMatchObject({ reports: [TIER0, INSTALLED_TIER1], loading: false, error: null });
  });

  it("keeps the failure of an install whose Settings window was closed, for the next time it opens", async () => {
    const { store, api } = await startedStore([TIER0, TIER1]);
    let fail: (err: unknown) => void = () => {};
    api.install.mockReturnValue(new Promise((_resolve, reject) => (fail = reject)));
    await store.ask(TIER1.id);
    const running = store.confirm(TIER1.id);
    expect(store.get().installs[TIER1.id]).toMatchObject({ phase: "running" });
    fail(new Error("2mass-l8-v1: the server answered HTTP 503"));
    await running;
    expect(store.get().installs[TIER1.id]).toEqual({ phase: "failed", message: "2mass-l8-v1: the server answered HTTP 503" });
    const reopened = rowMarkup(renderToStaticMarkup(createElement(CatalogsSettings, { store })), TIER1.id);
    expect(reopened).toMatch(/role="alert"[^>]*>2mass-l8-v1: the server answered HTTP 503</);
  });

  it("says so and logs it when the install progress cannot be followed", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const { api } = fakeApi([TIER0]);
    api.onProgress.mockRejectedValue(new Error("event.listen not allowed"));
    const store = createCatalogStore(api);
    store.start();
    await flush();
    expect(store.get().error).toBe("Install progress is not available: event.listen not allowed");
    expect(logged).toHaveBeenCalledWith("[AstroBurst] Catalogue install progress:", expect.any(Error));
    logged.mockRestore();
  });

  it("reads the rows again once the Data folder setting is saved", async () => {
    typedInvokeMock.mockReset().mockResolvedValue({});
    const { store, api } = await startedStore([TIER0, INSTALLED_TIER1]);
    api.status.mockResolvedValueOnce([TIER0, TIER1]);
    await updateConfig("data_dir_override", "D:/AstroBurst");
    await flush();
    expect(api.status).toHaveBeenCalledTimes(2);
    expect(store.get().reports).toEqual([TIER0, TIER1]);
    await updateConfig("plate_solve_timeout_secs", 120);
    await flush();
    expect(api.status).toHaveBeenCalledTimes(2);
  });

  it("keeps the newest status when an older read answers last", async () => {
    const { store, api } = await startedStore([TIER0]);
    let older: (rows: CatalogDatasetReport[]) => void = () => {};
    api.status.mockReturnValueOnce(new Promise((resolve) => (older = resolve))).mockResolvedValueOnce([TIER0, INSTALLED_TIER1]);
    const first = store.refresh();
    await store.refresh();
    older([TIER0]);
    await first;
    expect(store.get().reports).toEqual([TIER0, INSTALLED_TIER1]);
  });

  it("forgets an external file and deletes a download through remove, then re-reads the rows", async () => {
    const external: CatalogDatasetReport = { ...SIRIL, status: { state: "external", path: "D:/siril/x.dat", valid: true, problem: null } };
    const { store, api } = await startedStore([TIER0, external]);
    api.remove.mockResolvedValueOnce({ id: SIRIL.id, freed_bytes: 0, forgot_external: true });
    await store.remove(SIRIL.id);
    expect(api.remove).toHaveBeenCalledWith(SIRIL.id);
    expect(api.status).toHaveBeenCalledTimes(2);
  });

  it("shows a download still running after Settings was reopened, cancels it and refreshes the row", async () => {
    const { store, api, listener } = await startedStore([TIER0, { ...TIER1, status: { state: "partial", received: 64_700_000, total: 154_140_794 } }]);
    listener.current!({ id: TIER1.id, current: 70_000_000, total: 154_140_794, percent: 45, stage: "download" });
    expect(store.get().installs[TIER1.id]).toMatchObject({ phase: "running", percent: 45, adopted: true });
    const reads = api.status.mock.calls.length;
    await store.cancel(TIER1.id);
    expect(api.cancel).toHaveBeenCalledWith(TIER1.id);
    expect(store.get().installs[TIER1.id]).toEqual({ phase: "idle", note: CANCELLED_NOTE });
    expect(api.status.mock.calls.length).toBe(reads + 1);
  });

  it("does not report an adopted download that completed as Cancel was pressed as a failure", async () => {
    const { store, api, listener } = await startedStore([TIER0, TIER1]);
    listener.current!({ id: TIER1.id, current: 1, total: 154_140_794, percent: 0, stage: "download" });
    api.cancel.mockImplementationOnce(async () => {
      listener.current!({ id: TIER1.id, current: 154_140_794, total: 154_140_794, percent: 100, stage: "complete" });
      return false;
    });
    await store.cancel(TIER1.id);
    expect(store.get().installs[TIER1.id]).toEqual({ phase: "idle" });
  });

  it("reads the Zenodo record when its download is asked for and installs with the consent built from it", async () => {
    const { store, api } = await startedStore([TIER0, SIRIL_ZENODO]);
    api.offer.mockResolvedValue(OFFER);
    api.install.mockResolvedValue({
      id: SIRIL.id,
      path: "C:/data/catalogs/siril-gaia-astro-v1/siril_cat_healpix8_astro.dat",
      bytes: 1_450_000_000,
      resumed_from: 0,
      sha256_hex: "2fa4",
      sha256_verified: true,
      already_installed: false,
    });
    await store.confirm(SIRIL.id);
    expect(api.install).not.toHaveBeenCalled();
    await store.ask(SIRIL.id);
    expect(api.offer).toHaveBeenCalledTimes(1);
    expect(store.get().installs[SIRIL.id]).toEqual({ phase: "consent", offer: OFFER });
    await store.confirm(SIRIL.id);
    expect(api.install.mock.calls).toEqual([
      [SIRIL.id, { archive_bytes: OFFER.archive_bytes, archive_sha256: OFFER.archive_sha256, licence_acknowledged: true }],
    ]);
    expect(store.get().installs[SIRIL.id]).toEqual({
      phase: "idle",
      note: "Installed 1.45 GB, SHA-256 verified. It is now your Siril catalogue folder.",
    });
  });

  it("uses a verified Zenodo download already on disk without reading Zenodo, a consent or a download", async () => {
    const { store, api } = await startedStore([TIER0, INSTALLED_ZENODO]);
    api.useSirilDir.mockResolvedValue({ dir: OFFER.folder, path: INSTALLED_ZENODO.folder });
    const reads = api.status.mock.calls.length;
    await store.ask(SIRIL.id);
    expect(api.offer).not.toHaveBeenCalled();
    expect(api.install).not.toHaveBeenCalled();
    expect(api.useSirilDir.mock.calls).toEqual([[OFFER.folder]]);
    expect(store.get().installs[SIRIL.id]).toEqual({ phase: "idle", note: USED_DOWNLOAD_NOTE });
    expect(api.status.mock.calls.length).toBe(reads + 1);
  });

  it("does not start the Zenodo download when its record could not be read", async () => {
    const { store, api } = await startedStore([TIER0, SIRIL_ZENODO]);
    api.offer.mockRejectedValue(new Error("siril-gaia-astro-v1: zenodo.org answered HTTP 503; try again in a few minutes"));
    await store.ask(SIRIL.id);
    expect(store.get().installs[SIRIL.id]).toEqual({
      phase: "consent",
      offerError: "siril-gaia-astro-v1: zenodo.org answered HTTP 503; try again in a few minutes",
    });
    await store.confirm(SIRIL.id);
    expect(api.install).not.toHaveBeenCalled();
  });

  it("installs the other catalogues without reading Zenodo or sending a consent", async () => {
    const { store, api } = await startedStore([TIER0, TIER1]);
    api.install.mockResolvedValue({ id: TIER1.id, path: "x", bytes: 154_140_794, resumed_from: 0, sha256_hex: "c9ab", sha256_verified: true, already_installed: false });
    await store.ask(TIER1.id);
    await store.confirm(TIER1.id);
    expect(api.offer).not.toHaveBeenCalled();
    expect(api.install.mock.calls).toEqual([[TIER1.id]]);
  });

  it("re-reads the rows when an install completes, also one this Settings did not start", async () => {
    const { api, listener } = await startedStore([TIER0, TIER1]);
    const reads = api.status.mock.calls.length;
    listener.current!({ id: TIER1.id, current: 154_140_794, total: 154_140_794, percent: 100, stage: "complete" });
    await flush();
    expect(api.status.mock.calls.length).toBe(reads + 1);
  });

  it("chooses a Siril folder or file and shows a failed choice as the section error", async () => {
    const { store, api } = await startedStore([TIER0, SIRIL]);
    api.useFile.mockResolvedValueOnce({ path: "D:/siril/x.dat", id: SIRIL.id });
    await store.useFile(SIRIL.id, "D:/siril/x.dat");
    expect(api.useFile).toHaveBeenCalledWith(SIRIL.id, "D:/siril/x.dat");
    api.useSirilDir.mockRejectedValueOnce(new Error("D:/nowhere is not a Siril catalogue folder"));
    await store.useSirilDir("D:/nowhere");
    expect(store.get().error).toBe("D:/nowhere is not a Siril catalogue folder");
  });
});

describe("Settings > Catalogs window", () => {
  let store: CatalogStore;
  let api: ReturnType<typeof fakeApi>["api"];

  beforeEach(async () => {
    ({ store, api } = await startedStore([TIER0, INSTALLED_TIER1]));
    hooks.mount();
  });

  afterEach(() => hooks.mount());

  function section(): CatalogsSectionProps {
    const tree = hooks.render(CatalogsSettings, { store });
    expect(isValidElement(tree), "CatalogsSettings renders the section").toBe(true);
    return (tree as { props: CatalogsSectionProps }).props;
  }

  it("deletes a downloaded catalogue only on the second click of Remove", async () => {
    section().onRemove(TIER1.id);
    await flush();
    expect(api.remove).not.toHaveBeenCalled();
    expect(section().armedRemove).toBe(TIER1.id);
    section().onRemove(TIER1.id);
    await flush();
    expect(api.remove).toHaveBeenCalledWith(TIER1.id);
    expect(section().armedRemove).toBeNull();
  });

  it("disarms Remove when the focus leaves it", async () => {
    section().onRemove(TIER1.id);
    section().onDisarm();
    expect(section().armedRemove).toBeNull();
    section().onRemove(TIER1.id);
    await flush();
    expect(api.remove).not.toHaveBeenCalled();
  });

  it("shows the store's rows and installs, and re-renders when the store changes", async () => {
    expect(section().reports).toEqual([TIER0, INSTALLED_TIER1]);
    const before = hooks.run.notified;
    await store.ask(TIER1.id);
    expect(hooks.run.notified).toBeGreaterThan(before);
    expect(section().installs[TIER1.id]).toEqual({ phase: "consent" });
  });

  it("forgets an external file on one click, as nothing is deleted", async () => {
    const external: CatalogDatasetReport = { ...SIRIL, status: { state: "external", path: "D:/siril/x.dat", valid: true, problem: null } };
    ({ store, api } = await startedStore([TIER0, external]));
    hooks.mount();
    api.remove.mockResolvedValueOnce({ id: SIRIL.id, freed_bytes: 0, forgot_external: true });
    section().onRemove(SIRIL.id);
    await flush();
    expect(api.remove).toHaveBeenCalledWith(SIRIL.id);
    expect(section().armedRemove).toBeNull();
  });
});
