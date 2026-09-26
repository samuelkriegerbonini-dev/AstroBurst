import { describe, it, expect, beforeEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock, withPreview: vi.fn() }));

import { computeFftSpectrum } from "../analysis";

function payload(): ArrayBuffer {
  const bytes = new Uint8Array(40 + 2 * 1);
  const view = new DataView(bytes.buffer);
  view.setUint32(0, 2, true);
  view.setUint32(4, 1, true);
  view.setFloat32(8, 3, true);
  view.setFloat32(12, 9, true);
  view.setUint32(16, 5, true);
  view.setUint32(20, 8192, true);
  view.setUint32(24, 4096, true);
  view.setUint32(28, 3, true);
  view.setUint32(32, 6000, true);
  view.setUint32(36, 3000, true);
  bytes.set([10, 20], 40);
  return bytes.buffer;
}

describe("computeFftSpectrum", () => {
  beforeEach(() => typedInvokeMock.mockReset());

  it("pins the command name and the argument object", async () => {
    typedInvokeMock.mockResolvedValue(payload());
    await computeFftSpectrum("/f.fits");
    expect(typedInvokeMock).toHaveBeenCalledWith("compute_fft_spectrum", { path: "/f.fits" });
  });

  it("returns the display size, the padded grid per axis and both flags", async () => {
    typedInvokeMock.mockResolvedValue(payload());
    const fft = await computeFftSpectrum("/f.fits");
    expect(fft).toMatchObject({
      width: 2,
      height: 1,
      elapsed_ms: 5,
      grid_width: 8192,
      grid_height: 4096,
      windowed: true,
      downsampled: true,
      image_width: 6000,
      image_height: 3000,
    });
    expect(Array.from(fft.pixels)).toEqual([10, 20]);
  });
});
