import { describe, it, expect } from "vitest";
import { monoPixelsAction } from "../gpuMonoPixels";

interface GpuPixels {
  loadedKey: string | null;
  drawn: string | null;
}

const NOTHING: GpuPixels = { loadedKey: null, drawn: null };

function publish(held: GpuPixels, pngOnly: boolean, key: string, clearOnSourceChange: boolean): GpuPixels {
  const action = monoPixelsAction(pngOnly, key, held.loadedKey, clearOnSourceChange);
  if (action === "clear") return NOTHING;
  if (action === "reload") return { loadedKey: key, drawn: null };
  if (action === "load") return { loadedKey: key, drawn: held.drawn };
  return held;
}

function land(held: GpuPixels, source: string): GpuPixels {
  return { ...held, drawn: source };
}

describe("monoPixelsAction", () => {
  it("never leaves pixels of another source on the GPU when a channel FITS record replaces its PNG-only record", () => {
    let held = land(publish(NOTHING, false, "k|cube.fits|0", true), "cube.fits");
    held = land(publish(held, false, "k|ch25.fits|7", true), "ch25.fits");
    held = publish(held, true, "k|cube.fits|0", true);
    expect(held.drawn).toBeNull();
    held = publish(held, false, "k|ch26.fits|9", true);
    expect(held.drawn).toBeNull();
    held = land(held, "ch26.fits");
    expect(held.drawn).toBe("ch26.fits");
  });

  it("never draws the previous channel under a revisited channel whose FITS record is published at once", () => {
    let held = land(publish(NOTHING, false, "k|cube_frame_12.fits|4", true), "cube_frame_12.fits");
    held = publish(held, false, "k|cube_frame_13.fits|5", true);
    expect(held.drawn).toBeNull();
    held = land(held, "cube_frame_13.fits");
    held = publish(held, false, "k|cube_frame_12.fits|6", true);
    expect(held.drawn).toBeNull();
    expect(held.loadedKey).toBe("k|cube_frame_12.fits|6");
  });

  it("never draws a channel as the original cube plane after a Reset", () => {
    let held = land(publish(NOTHING, false, "k|cube_frame_12.fits|4", true), "cube_frame_12.fits");
    held = publish(held, false, "k|cube.fits|0", true);
    expect(held.drawn).toBeNull();
  });

  it("clears before reloading when a cube source changes and loads without clearing otherwise", () => {
    expect(monoPixelsAction(false, "k|cube_frame_12.fits|6", "k|cube_frame_13.fits|5", true)).toBe("reload");
    expect(monoPixelsAction(false, "k|b.fits|3", "k|a.fits|2", false)).toBe("load");
    expect(monoPixelsAction(false, "k|b.fits|3", null, true)).toBe("load");
  });

  it("drops held pixels on the first commit after opening the cube", () => {
    expect(monoPixelsAction(true, "k|cube.fits|0", "k|cube.fits|0", true)).toBe("clear");
  });

  it("does nothing while a PNG-only record is shown and nothing is held", () => {
    expect(monoPixelsAction(true, "k|cube.fits|0", null, true)).toBe("keep");
  });

  it("keeps pixels already loaded for the same key or when there is no key", () => {
    expect(monoPixelsAction(false, "k|b.fits|3", "k|b.fits|3", true)).toBe("keep");
    expect(monoPixelsAction(false, null, "k|b.fits|3", true)).toBe("keep");
  });
});
