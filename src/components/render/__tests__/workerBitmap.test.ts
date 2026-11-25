import { describe, expect, it, vi } from "vitest";
import { presentWorkerBitmap } from "../../../utils/stfworker";

function bitmap() {
  return { width: 4, height: 2, close: vi.fn() } as unknown as ImageBitmap & { close: ReturnType<typeof vi.fn> };
}

function canvas(bitmaprenderer: boolean) {
  const transferFromImageBitmap = vi.fn();
  const drawImage = vi.fn();
  const el = {
    width: 0,
    height: 0,
    getContext: (kind: string) => (kind === "bitmaprenderer" ? (bitmaprenderer ? { transferFromImageBitmap } : null) : { drawImage }),
  } as unknown as HTMLCanvasElement;
  return { el, transferFromImageBitmap, drawImage };
}

describe("presentWorkerBitmap", () => {
  it("releases a bitmap that a newer render made stale", () => {
    const shot = bitmap();
    const target = canvas(true);
    presentWorkerBitmap(target.el, { bitmap: shot, width: 4, height: 2 }, false);
    expect(shot.close).toHaveBeenCalledTimes(1);
    expect(target.transferFromImageBitmap).not.toHaveBeenCalled();
  });

  it("releases a bitmap when the canvas is gone", () => {
    const shot = bitmap();
    presentWorkerBitmap(null, { bitmap: shot, width: 4, height: 2 }, true);
    expect(shot.close).toHaveBeenCalledTimes(1);
  });

  it("hands a current bitmap to the canvas at its size", () => {
    const shot = bitmap();
    const target = canvas(true);
    presentWorkerBitmap(target.el, { bitmap: shot, width: 4, height: 2 }, true);
    expect(target.transferFromImageBitmap).toHaveBeenCalledWith(shot);
    expect([target.el.width, target.el.height]).toEqual([4, 2]);
    expect(shot.close).not.toHaveBeenCalled();
  });

  it("draws and releases a current bitmap on a 2D canvas", () => {
    const shot = bitmap();
    const target = canvas(false);
    presentWorkerBitmap(target.el, { bitmap: shot, width: 4, height: 2 }, true);
    expect(target.drawImage).toHaveBeenCalledWith(shot, 0, 0);
    expect(shot.close).toHaveBeenCalledTimes(1);
  });
});
