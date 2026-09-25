import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, withPreview: vi.fn() }));

import { finiteSky, measurePhotometry, measurePhotometryBatch, timeSeriesPhotometry, pixelTable } from "../analysis";

const BASE = { photometry: {}, gaia: null, photcal: null, warnings: [], masked: false, elapsed_ms: 1 };

describe("measurePhotometry", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("drops a sky position the WCS could not unproject instead of passing nulls to the panel", async () => {
    typedInvokeMock.mockResolvedValue({ ...BASE, sky: { ra: null, dec: null } });
    const res = await measurePhotometry("/a.fits", 10, 10);
    expect(res.sky).toBeNull();
  });

  it("keeps a finite sky position", async () => {
    typedInvokeMock.mockResolvedValue({ ...BASE, sky: { ra: 10.5, dec: -3.25 } });
    const res = await measurePhotometry("/a.fits", 10, 10);
    expect(res.sky).toEqual({ ra: 10.5, dec: -3.25 });
  });
});

describe("measurePhotometry arguments", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("sends null annulus radii and gain when the options are unset", async () => {
    typedInvokeMock.mockResolvedValue({ ...BASE, sky: null });
    await measurePhotometry("/a.fits", 10, 20, { apertureRadius: 5 });
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_photometry_cmd", {
      path: "/a.fits",
      x: 10,
      y: 20,
      apertureRadius: 5,
      annulusInner: null,
      annulusOuter: null,
      gaiaMatch: true,
      excludeDq: false,
      gain: null,
    });
  });

  it("forwards explicit annulus radii and gain", async () => {
    typedInvokeMock.mockResolvedValue({ ...BASE, sky: null });
    await measurePhotometry("/a.fits", 1, 2, { annulusInner: 8, annulusOuter: 12, gain: 1.5, gaiaMatch: false });
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({ annulusInner: 8, annulusOuter: 12, gain: 1.5, gaiaMatch: false });
  });
});

describe("measurePhotometryBatch", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  const BATCH = { photcal: null, warnings: [], masked: false, n_measured: 2, n_failed: 0, elapsed_ms: 3 };

  it("pins the command name and the argument object", async () => {
    typedInvokeMock.mockResolvedValue({ ...BATCH, rows: [] });
    await measurePhotometryBatch("/b.fits", [
      [1, 2],
      [3.5, 4.5],
    ]);
    expect(typedInvokeMock).toHaveBeenCalledWith("measure_photometry_batch_cmd", {
      path: "/b.fits",
      points: [
        [1, 2],
        [3.5, 4.5],
      ],
      apertureRadius: null,
      annulusInner: null,
      annulusOuter: null,
      gain: null,
      excludeDq: false,
      withGrowthCurve: false,
    });
  });

  it("forwards every option", async () => {
    typedInvokeMock.mockResolvedValue({ ...BATCH, rows: [] });
    await measurePhotometryBatch("/b.fits", [[1, 2]], {
      apertureRadius: 5,
      annulusInner: 10,
      annulusOuter: 15,
      gain: 2,
      excludeDq: true,
      withGrowthCurve: true,
    });
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({
      apertureRadius: 5,
      annulusInner: 10,
      annulusOuter: 15,
      gain: 2,
      excludeDq: true,
      withGrowthCurve: true,
    });
  });

  it("applies finiteSky to every row", async () => {
    typedInvokeMock.mockResolvedValue({
      ...BATCH,
      rows: [
        { index: 0, photometry: {}, sky: { ra: null, dec: null }, error: null },
        { index: 1, photometry: {}, sky: { ra: 10.5, dec: -3.25 }, error: null },
        { index: 2, photometry: null, sky: null, error: "no finite pixels" },
      ],
    });
    const res = await measurePhotometryBatch("/b.fits", [
      [1, 2],
      [3, 4],
      [5, 6],
    ]);
    expect(res.rows.map((r) => r.sky)).toEqual([null, { ra: 10.5, dec: -3.25 }, null]);
    expect(res.rows[2].error).toBe("no finite pixels");
    expect(res.n_measured).toBe(2);
  });
});

describe("finiteSky", () => {
  it("rejects a half-valid position", () => {
    expect(finiteSky({ ra: 1, dec: null })).toBeNull();
    expect(finiteSky({ ra: Number.NaN, dec: 2 })).toBeNull();
    expect(finiteSky(undefined)).toBeNull();
  });
});

describe("timeSeriesPhotometry", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  const SERIES = {
    reference_path: "/a.fits",
    targets: [],
    frames: [],
    n_frames: 2,
    n_skipped: 0,
    warnings: [],
    geometry_target: null,
    geometry_notes: [],
    elapsed_ms: 4,
  };
  const TARGETS = [
    { x: 10, y: 20, label: "T", role: "target" as const },
    { x: 30, y: 40, label: "C1", role: "comp" as const },
  ];

  it("pins the command name and the argument object with the drift and DQ defaults", async () => {
    typedInvokeMock.mockResolvedValue(SERIES);
    const res = await timeSeriesPhotometry(["/a.fits", "/b.fits"], TARGETS, { apertureRadius: 5 });
    expect(typedInvokeMock).toHaveBeenCalledWith("time_series_photometry_cmd", {
      paths: ["/a.fits", "/b.fits"],
      targets: TARGETS,
      apertureRadius: 5,
      annulusInner: null,
      annulusOuter: null,
      gain: null,
      excludeDq: false,
      trackDrift: true,
      targetRa: null,
      targetDec: null,
      siteLat: null,
      siteLon: null,
      siteHeight: null,
    });
    expect(res).toBe(SERIES);
  });

  it("forwards the site and target overrides of the time series", async () => {
    typedInvokeMock.mockResolvedValue(SERIES);
    await timeSeriesPhotometry(["/a.fits"], TARGETS, {
      apertureRadius: 5,
      targetRa: 150.5,
      targetDec: -2.25,
      siteLat: 19.82,
      siteLon: -155.47,
      siteHeight: 4200,
    });
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({
      targetRa: 150.5,
      targetDec: -2.25,
      siteLat: 19.82,
      siteLon: -155.47,
      siteHeight: 4200,
    });
  });

  it("forwards the annulus, gain, DQ exclusion and drift flags", async () => {
    typedInvokeMock.mockResolvedValue(SERIES);
    await timeSeriesPhotometry(["/a.fits"], TARGETS, {
      apertureRadius: 6,
      annulusInner: 12,
      annulusOuter: 18,
      gain: 1.5,
      excludeDq: true,
      trackDrift: false,
    });
    expect(typedInvokeMock.mock.calls[0][1]).toMatchObject({
      apertureRadius: 6,
      annulusInner: 12,
      annulusOuter: 18,
      gain: 1.5,
      excludeDq: true,
      trackDrift: false,
    });
  });
});

describe("pixelTable", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins the command name and the argument object with the default size", async () => {
    typedInvokeMock.mockResolvedValue({ values: [] });
    await pixelTable("/c.fits", 12, 34);
    expect(typedInvokeMock).toHaveBeenCalledWith("pixel_table_cmd", { path: "/c.fits", x: 12, y: 34, size: 7 });
  });

  it("forwards an explicit size and returns the backend result untouched", async () => {
    const result = { x: 1, y: 2, size: 3, values: [[null]] };
    typedInvokeMock.mockResolvedValue(result);
    const res = await pixelTable("/c.fits", 1, 2, 3);
    expect(typedInvokeMock).toHaveBeenCalledWith("pixel_table_cmd", { path: "/c.fits", x: 1, y: 2, size: 3 });
    expect(res).toBe(result);
  });
});
