import { describe, it, expect, expectTypeOf, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { exportFitsRgb, exportFitsRgbWithHeader, exportValuesCell, type ExportResult } from "../export";

describe("export_fits display-referred flag", () => {
  it("is an optional boolean on the export result", () => {
    expectTypeOf<ExportResult["display_referred"]>().toEqualTypeOf<boolean | undefined>();
  });

  it("names display-referred values with the header marker", () => {
    expect(exportValuesCell({ display_referred: true })).toEqual({ label: "Values", value: "display-referred (ABDISP=T)" });
  });

  it("names unstretched values linear", () => {
    expect(exportValuesCell({ display_referred: false })).toEqual({ label: "Values", value: "linear" });
  });

  it("adds no cell when the export command does not report the flag", () => {
    expect(exportValuesCell({})).toBeNull();
  });
});

const OK = { output_path: "/x/out.fits", elapsed_ms: 1 };

describe("exportFitsRgb", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue(OK);
  });

  it("sends the header source so wizard channels held as cache keys still get a WCS", async () => {
    await exportFitsRgb("__wizard_ch_ha_aligned", null, null, "/x/out.fits", { headerPath: "D:/raw/ha_1.fits" });
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({ headerPath: "D:/raw/ha_1.fits" });
  });

  it("sends null when no header source is given", async () => {
    await exportFitsRgb("a.fits", "b.fits", "c.fits", "/x/out.fits");
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({ headerPath: null });
  });
});

describe("exportFitsRgbWithHeader", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
  });

  it("exports without the header and says so when the header source cannot be read", async () => {
    typedInvokeMock
      .mockRejectedValueOnce(new Error("Failed to read the header source D:/raw/ha_1.fits: file not found"))
      .mockResolvedValueOnce(OK);
    const out = await exportFitsRgbWithHeader("__wizard_ch_ha_aligned", null, null, "/x/out.fits", { headerPath: "D:/raw/ha_1.fits" });
    expect(out.result).toEqual(OK);
    expect(out.headerWarning).toContain("D:/raw/ha_1.fits");
    expect(typedInvokeMock.mock.calls[1][1]).toMatchObject({ headerPath: null });
  });

  it("does not hide any other export error", async () => {
    typedInvokeMock.mockRejectedValueOnce(new Error("refusing to overwrite the source file"));
    await expect(
      exportFitsRgbWithHeader("a.fits", null, null, "/x/out.fits", { headerPath: "a.fits" }),
    ).rejects.toThrow("refusing to overwrite");
    expect(typedInvokeMock).toHaveBeenCalledTimes(1);
  });

  it("reports no warning when the header was used", async () => {
    typedInvokeMock.mockResolvedValueOnce(OK);
    const out = await exportFitsRgbWithHeader("a.fits", null, null, "/x/out.fits", { headerPath: "a.fits" });
    expect(out.headerWarning).toBeNull();
  });
});
