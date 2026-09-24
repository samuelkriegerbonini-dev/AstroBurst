import { describe, it, expect } from "vitest";
import { frameStem, framesLabel, resultsForRecipients, showsOutput, sourceLabel, stackOutputName, toDims } from "../stackingOutputs";
import type { ProcessedResult } from "../../shared/types/preview";

function result(fitsPath: string | null, previewUrl: string | null): ProcessedResult {
  return { fitsPath, previewUrl, dimensions: null, label: "x", kind: "stacking", inputPath: "/in/a.fits" };
}

describe("frameStem", () => {
  it("strips the directory and the extension, including a compression suffix", () => {
    expect(frameStem("C:\\data\\night1\\Ha_001.fits")).toBe("Ha_001");
    expect(frameStem("/data/OIII_002.fits.fz")).toBe("OIII_002");
    expect(frameStem("/data/lum.fit.gz")).toBe("lum");
  });

  it("replaces characters that are unsafe in a file name and never returns an empty stem", () => {
    expect(frameStem("/data/M 42 light #1.fits")).toBe("M_42_light_1");
    expect(frameStem("/data/.fits")).toBe("frame");
  });
});

describe("stackOutputName", () => {
  const t1 = new Date(2026, 8, 23, 14, 25, 30, 123);
  const t2 = new Date(2026, 8, 23, 14, 25, 30, 456);

  it("never falls back to the fixed name that made every stack overwrite the previous one", () => {
    expect(stackOutputName("/subs/Ha_001.fits", 20, t1)).not.toBe("stacked");
  });

  it("names the stack after its first frame, the frame count and the run time", () => {
    expect(stackOutputName("/subs/Ha_001.fits", 20, t1)).toBe("Ha_001_stack20_20260923-142530-123");
  });

  it("gives different names to stacks of different frames and to reruns of the same frames", () => {
    const ha = stackOutputName("/subs/Ha_001.fits", 20, t1);
    const oiii = stackOutputName("/subs/OIII_001.fits", 20, t1);
    const rerun = stackOutputName("/subs/Ha_001.fits", 20, t2);
    expect(new Set([ha, oiii, rerun]).size).toBe(3);
  });
});

describe("framesLabel", () => {
  it("says how many frames the result was computed from", () => {
    expect(framesLabel("Stack", 12)).toBe("Stack · 12 frames");
    expect(framesLabel("Drizzle RGB", 1)).toBe("Drizzle RGB · 1 frame");
  });
});

describe("sourceLabel", () => {
  it("keeps the bare label when the source is the displayed file, ignoring case and slashes", () => {
    expect(sourceLabel("Calibrated", "C:\\Data\\light_001.fits", "c:/data/light_001.fits")).toBe("Calibrated");
  });

  it("names the source frame when the result was computed from another file", () => {
    expect(sourceLabel("Calibrated", "/data/light_002.fits", "/data/light_001.fits")).toBe("Calibrated · light_002.fits");
  });
});

describe("resultsForRecipients", () => {
  const light1 = { key: "1|/data/light_001.fits", path: "/data/light_001.fits" };
  const light2 = { key: "2|/data/light_002.fits", path: "/data/light_002.fits" };
  const calibrated2 = {
    fitsPath: "/out/light_002_calibrated.fits",
    previewUrl: "asset://out/light_002_calibrated.png",
    dimensions: [640, 480] as [number, number],
    kind: "stacking" as const,
    inputPath: light2.path,
  };
  const calibratedLabel = (path: string) => sourceLabel("Calibrated", light2.path, path);
  const labels = (planned: { key: string; result: ProcessedResult }[]) =>
    Object.fromEntries(planned.map((p) => [p.key, p.result.label]));

  it("labels another file that shows the same output against that file, not against the run target", () => {
    const planned = resultsForRecipients(light2, [light1], calibrated2, calibratedLabel);
    expect(labels(planned)).toEqual({
      [light2.key]: "Calibrated",
      [light1.key]: "Calibrated · light_002.fits",
    });
  });

  it("does not make the source file name itself as a foreign source when the run target is another file", () => {
    const planned = resultsForRecipients(light1, [light2], calibrated2, calibratedLabel);
    expect(labels(planned)).toEqual({
      [light1.key]: "Calibrated · light_002.fits",
      [light2.key]: "Calibrated",
    });
  });

  it("publishes to the run target first and only once, with the output fields unchanged", () => {
    const planned = resultsForRecipients(light2, [light2, light1], calibrated2, calibratedLabel);
    expect(planned.map((p) => p.key)).toEqual([light2.key, light1.key]);
    for (const p of planned) expect({ ...p.result, label: undefined }).toEqual({ ...calibrated2, label: undefined });
  });

  it("gives every recipient the same label when the label does not depend on the file", () => {
    const planned = resultsForRecipients(light1, [light2], { ...calibrated2, inputPath: light1.path }, () => framesLabel("Stack", 12));
    expect(new Set(planned.map((p) => p.result.label))).toEqual(new Set(["Stack · 12 frames"]));
  });
});

describe("toDims", () => {
  it("accepts a positive width and height pair only", () => {
    expect(toDims([640, 480])).toEqual([640, 480]);
    expect(toDims([0, 480])).toBeNull();
    expect(toDims([640])).toBeNull();
    expect(toDims(undefined)).toBeNull();
  });
});

describe("showsOutput", () => {
  it("matches a record that shows the same FITS output, ignoring case and slashes", () => {
    const shown = result("C:\\out\\A_cosmetic.fits", "asset://A_cosmetic.png?v=3");
    expect(showsOutput(shown, result("c:/out/A_cosmetic.fits", "asset://A_cosmetic.png"))).toBe(true);
  });

  it("does not match a different FITS output", () => {
    expect(showsOutput(result("/out/A_cosmetic.fits", null), result("/out/B_cosmetic.fits", null))).toBe(false);
  });

  it("does not compare a FITS result with a PNG-only result", () => {
    expect(showsOutput(result(null, "asset://out/x.png?v=1"), result("/out/x.fits", "asset://out/x.png"))).toBe(false);
  });

  it("matches PNG-only results by the image URL without its version query", () => {
    const shown = result(null, "asset://out/drizzle_rgb.png?v=7");
    expect(showsOutput(shown, result(null, "asset://out/drizzle_rgb.png"))).toBe(true);
    expect(showsOutput(shown, result(null, "asset://out/other.png"))).toBe(false);
  });

  it("is false when nothing is shown", () => {
    expect(showsOutput(null, result("/out/x.fits", null))).toBe(false);
    expect(showsOutput(undefined, result("/out/x.fits", null))).toBe(false);
  });
});
