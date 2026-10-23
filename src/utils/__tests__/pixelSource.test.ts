import { describe, expect, it } from "vitest";
import { pixelSourceVersion } from "../pixelSource";

describe("pixelSourceVersion", () => {
  it("changes with each reload of the file while the viewer shows the file itself", () => {
    const first = pixelSourceVersion({}, null, 0);
    const reloaded = pixelSourceVersion({ reloads: 1 }, null, 0);
    expect(reloaded).not.toBe(first);
    expect(pixelSourceVersion({ reloads: 2 }, null, 0)).not.toBe(reloaded);
    expect(pixelSourceVersion({ reloads: 1 }, null, 0)).toBe(reloaded);
  });

  it("follows the processed record while one is shown, so a reload of its source does not fetch the same output again", () => {
    expect(pixelSourceVersion({ reloads: 3 }, "C:/out/a_deconv.fits", 5)).toBe(pixelSourceVersion({ reloads: 2 }, "C:/out/a_deconv.fits", 5));
    expect(pixelSourceVersion({ reloads: 2 }, "C:/out/a_deconv.fits", 6)).not.toBe(pixelSourceVersion({ reloads: 2 }, "C:/out/a_deconv.fits", 5));
  });

  it("is the first version without a file", () => {
    expect(pixelSourceVersion(null, null, 0)).toBe(pixelSourceVersion({}, null, 0));
  });
});
