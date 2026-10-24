import { beforeEach, describe, expect, it, vi } from "vitest";

const { typedInvokeMock, listenMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn(), listenMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));

import {
  CATALOG_COMMANDS,
  CATALOG_INSTALL_PROGRESS_EVENT,
  cancelCatalogInstall,
  catalogStatus,
  chooseCatalogFile,
  chooseSirilCatalogDir,
  installCatalog,
  onCatalogInstallProgress,
  removeCatalog,
  type CatalogInstallProgress,
} from "../catalogStore";

describe("catalogStore commands", () => {
  beforeEach(() => typedInvokeMock.mockReset().mockResolvedValue({}));

  it("wraps the five catalog_store commands the backend registers", () => {
    expect(CATALOG_COMMANDS).toEqual([
      "catalog_status_cmd",
      "catalog_install_cmd",
      "catalog_use_file_cmd",
      "catalog_use_siril_dir_cmd",
      "catalog_remove_cmd",
    ]);
  });

  it("reads the dataset rows", async () => {
    const rows = [{ id: "tycho2-l6-v1" }];
    typedInvokeMock.mockResolvedValue(rows);
    await expect(catalogStatus()).resolves.toBe(rows);
    expect(typedInvokeMock).toHaveBeenCalledWith("catalog_status_cmd", {});
  });

  it("installs, points at a file or a Siril folder, and removes by dataset id", async () => {
    await installCatalog("2mass-l8-v1");
    await chooseCatalogFile("siril-gaia-astro-v1", "D:/siril/siril_cat_healpix8_astro.dat");
    await chooseSirilCatalogDir("D:/siril");
    await removeCatalog("2mass-l8-v1");
    expect(typedInvokeMock.mock.calls).toEqual([
      ["catalog_install_cmd", { id: "2mass-l8-v1" }],
      ["catalog_use_file_cmd", { id: "siril-gaia-astro-v1", path: "D:/siril/siril_cat_healpix8_astro.dat" }],
      ["catalog_use_siril_dir_cmd", { dir: "D:/siril" }],
      ["catalog_remove_cmd", { id: "2mass-l8-v1" }],
    ]);
  });

  it("cancels one install under its own progress key", async () => {
    typedInvokeMock.mockResolvedValue(true);
    await expect(cancelCatalogInstall("2mass-l8-v1")).resolves.toBe(true);
    expect(CATALOG_INSTALL_PROGRESS_EVENT).toBe("catalog-install-progress");
    expect(typedInvokeMock).toHaveBeenCalledWith("cancel_progress_cmd", { event: "catalog-install-progress:2mass-l8-v1" });
  });
});

describe("onCatalogInstallProgress", () => {
  beforeEach(() => listenMock.mockReset());

  it("passes each progress payload through and returns the unlisten function", async () => {
    const unlisten = vi.fn();
    let handler: ((event: { payload: CatalogInstallProgress }) => void) | null = null;
    listenMock.mockImplementation(async (_event: string, cb: (event: { payload: CatalogInstallProgress }) => void) => {
      handler = cb;
      return unlisten;
    });
    const seen: CatalogInstallProgress[] = [];
    const stop = await onCatalogInstallProgress((progress) => seen.push(progress));
    const payload: CatalogInstallProgress = { id: "2mass-l8-v1", current: 50, total: 200, percent: 25, stage: "download" };
    handler!({ payload });
    expect(listenMock.mock.calls[0][0]).toBe("catalog-install-progress");
    expect(seen).toEqual([payload]);
    expect(stop).toBe(unlisten);
  });
});
