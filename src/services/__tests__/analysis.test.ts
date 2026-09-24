import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, withPreview: vi.fn() }));

import { finiteSky, measurePhotometry } from "../analysis";

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

describe("finiteSky", () => {
  it("rejects a half-valid position", () => {
    expect(finiteSky({ ra: 1, dec: null })).toBeNull();
    expect(finiteSky({ ra: Number.NaN, dec: 2 })).toBeNull();
    expect(finiteSky(undefined)).toBeNull();
  });
});
