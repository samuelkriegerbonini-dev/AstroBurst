import { describe, it, expect, vi, beforeEach } from "vitest";

const pathMock = vi.hoisted(() => ({
  downloadDir: vi.fn(),
  homeDir: vi.fn(),
  documentDir: vi.fn(),
  desktopDir: vi.fn(),
  appDataDir: vi.fn(),
  join: vi.fn(async (...parts: string[]) => parts.join("/")),
}));

vi.mock("@tauri-apps/api/path", () => pathMock);

async function freshOutputModule() {
  vi.resetModules();
  return import("../output");
}

describe("getExportDir", () => {
  beforeEach(() => {
    pathMock.downloadDir.mockReset();
    pathMock.homeDir.mockReset();
    pathMock.documentDir.mockReset();
    pathMock.desktopDir.mockReset();
    pathMock.appDataDir.mockReset();
    pathMock.appDataDir.mockResolvedValue("/app");
  });

  it("uses the downloads folder when it resolves", async () => {
    pathMock.downloadDir.mockResolvedValue("/home/u/Downloads");
    const { getExportDir } = await freshOutputModule();
    await expect(getExportDir()).resolves.toBe("/home/u/Downloads");
  });

  it("falls back to the home folder, never to the LRU-managed output dir, when downloads cannot be resolved", async () => {
    pathMock.downloadDir.mockRejectedValue(new Error("unknown path"));
    pathMock.homeDir.mockResolvedValue("/home/u");
    const { getExportDir, getOutputDir } = await freshOutputModule();
    const exportDir = await getExportDir();
    expect(exportDir).toBe("/home/u");
    expect(exportDir).not.toBe(await getOutputDir());
  });

  it("returns a fixed fallback instead of the output dir when no user folder resolves", async () => {
    for (const fn of [pathMock.downloadDir, pathMock.homeDir, pathMock.documentDir, pathMock.desktopDir]) {
      fn.mockRejectedValue(new Error("unknown path"));
    }
    const { getExportDir } = await freshOutputModule();
    const dir = await getExportDir();
    expect(dir).toBe(".");
    expect(dir).not.toContain("output");
  });
});

describe("firstResolvedDir", () => {
  it("skips resolvers that reject or return an empty path", async () => {
    const { firstResolvedDir } = await freshOutputModule();
    const dir = await firstResolvedDir(
      [async () => { throw new Error("x"); }, async () => "", async () => "/docs"],
      "fallback",
    );
    expect(dir).toBe("/docs");
  });
});
