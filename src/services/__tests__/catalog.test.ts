import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({
  typedInvoke: typedInvokeMock,
  withPreview: vi.fn(),
  getOutputDir: vi.fn(),
  getPreviewUrl: vi.fn(),
}));

import { catalogConeSearch, catalogCrossMatch, catalogExportCsv } from "../catalog";

const PATH = "C:/d/a.fits";

describe("catalog services", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({});
  });

  it("pins catalog_cone_search_cmd with nulls for absent limits", async () => {
    await catalogConeSearch(PATH);
    expect(typedInvokeMock).toHaveBeenCalledWith("catalog_cone_search_cmd", {
      path: PATH,
      radiusArcmin: null,
      magLimit: null,
      maxRows: null,
    });
  });

  it("pins catalog_crossmatch_cmd with nulls for every absent option", async () => {
    await catalogCrossMatch(PATH);
    expect(typedInvokeMock).toHaveBeenCalledWith("catalog_crossmatch_cmd", {
      path: PATH,
      sigma: null,
      maxStars: null,
      radiusArcsec: null,
      band: null,
      colourTerm: null,
      apertureRadius: null,
      coneRadiusArcmin: null,
      magLimit: null,
      maxRows: null,
    });
  });

  it("forwards the cone, the G limit and the row cap of the cross-match", async () => {
    await catalogCrossMatch(PATH, {
      sigma: 5,
      maxStars: 500,
      radiusArcsec: 2,
      band: "G",
      colourTerm: true,
      apertureRadius: 5,
      coneRadiusArcmin: 12,
      magLimit: 18,
      maxRows: 5000,
    });
    expect(typedInvokeMock).toHaveBeenCalledWith("catalog_crossmatch_cmd", {
      path: PATH,
      sigma: 5,
      maxStars: 500,
      radiusArcsec: 2,
      band: "G",
      colourTerm: true,
      apertureRadius: 5,
      coneRadiusArcmin: 12,
      magLimit: 18,
      maxRows: 5000,
    });
  });

  it("pins catalog_export_csv_cmd with and without header lines", async () => {
    await catalogExportCsv(PATH, "C:/o/m.csv", { kind: "matches", matches: [] });
    expect(typedInvokeMock).toHaveBeenLastCalledWith("catalog_export_csv_cmd", {
      path: PATH,
      outputPath: "C:/o/m.csv",
      kind: "matches",
      rows: null,
      matches: [],
      headerLines: null,
    });

    await catalogExportCsv(PATH, "C:/o/m.csv", { kind: "matches", matches: [] }, ["# a: 1"]);
    expect(typedInvokeMock).toHaveBeenLastCalledWith("catalog_export_csv_cmd", {
      path: PATH,
      outputPath: "C:/o/m.csv",
      kind: "matches",
      rows: null,
      matches: [],
      headerLines: ["# a: 1"],
    });

    await catalogExportCsv(PATH, "C:/o/s.csv", { kind: "sources", rows: [] });
    expect(typedInvokeMock).toHaveBeenLastCalledWith("catalog_export_csv_cmd", {
      path: PATH,
      outputPath: "C:/o/s.csv",
      kind: "sources",
      rows: [],
      matches: null,
      headerLines: null,
    });
  });
});
