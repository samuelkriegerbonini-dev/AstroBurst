import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, withPreview: vi.fn() }));

import { computeHistogram } from "../analysis";

describe("computeHistogram", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins the command name and sends no window by default", async () => {
    typedInvokeMock.mockResolvedValue({ bins: [] });
    await computeHistogram("/a.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("compute_histogram", { path: "/a.fits", excludeDq: false, lo: null, hi: null });
  });

  it("forwards the DQ exclusion and a sky window as lo and hi", async () => {
    typedInvokeMock.mockResolvedValue({ bins: [] });
    await computeHistogram("/a.fits", true, { lo: 120, hi: 230 });
    expect(typedInvokeMock).toHaveBeenCalledWith("compute_histogram", { path: "/a.fits", excludeDq: true, lo: 120, hi: 230 });
  });
});
