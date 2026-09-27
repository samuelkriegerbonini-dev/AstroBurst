import { describe, it, expect } from "vitest";
import {
  displaysFileGrid,
  frameStem,
  framesLabel,
  otherGridHint,
  parksWizardComposite,
  pipelineInitialChoice,
  pipelineOutputName,
  pipelineViewOutput,
  resultsForRecipients,
  showsOutput,
  sourceLabel,
  stackOutputName,
  toDims,
} from "../stackingOutputs";
import type { ProcessedResult } from "../../shared/types/preview";
import type { PipelineMasterOutput, PipelineResult } from "../../shared/types/stacking";

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

describe("displaysFileGrid", () => {
  const stack = { kind: "stacking" as const, inputPath: "/d/frame_01.fits" };

  it("rejects a stack referenced to another frame as the file's pixel grid", () => {
    expect(displaysFileGrid(stack, "/d/frame_08.fits")).toBe(false);
  });

  it("accepts a stack whose reference is the viewed frame, including Windows separators", () => {
    expect(displaysFileGrid(stack, "/d/frame_01.fits")).toBe(true);
    expect(displaysFileGrid({ ...stack, inputPath: "\\d\\frame_01.fits" }, "/d/frame_01.fits")).toBe(true);
  });

  it("rejects a calibrated frame of another science file", () => {
    expect(displaysFileGrid({ kind: "stacking", inputPath: "/d/frame_03.fits" }, "/d/frame_08.fits")).toBe(false);
  });

  it("accepts the unprocessed file and geometry-preserving processing steps", () => {
    expect(displaysFileGrid(null, "/d/frame_08.fits")).toBe(true);
    expect(displaysFileGrid({ kind: "processing", inputPath: "/o/frame_08_bg.fits" }, "/d/frame_08.fits")).toBe(true);
  });

  it("tells the user that a reset alone leaves Points drawn on the stack in the stack's grid", () => {
    const hint = otherGridHint("Stack · 8 frames");
    expect(hint).toContain("Stack · 8 frames");
    expect(hint).toContain("stay in that grid after a reset");
    expect(hint).toMatch(/reset the preview, then place the Points on this frame before measuring/);
    expect(hint).not.toMatch(/Reset the preview to measure/);
    expect(otherGridHint(null)).toContain("The preview shows a result");
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

describe("pipelineOutputName", () => {
  const t1 = new Date(2026, 8, 23, 14, 25, 30, 123);
  const t2 = new Date(2026, 8, 23, 14, 25, 30, 456);

  it("names the pipeline outputs after the first light, the light count and the run time", () => {
    expect(pipelineOutputName("C:\\subs\\673nmos.fits", 3, t1)).toBe("673nmos_pipeline3_20260923-142530-123");
  });

  it("never reuses the name of a stack of the same frames or of an earlier pipeline run", () => {
    const pipeline = pipelineOutputName("/subs/673nmos.fits", 3, t1);
    const stack = stackOutputName("/subs/673nmos.fits", 3, t1);
    const rerun = pipelineOutputName("/subs/673nmos.fits", 3, t2);
    expect(new Set([pipeline, stack, rerun]).size).toBe(3);
  });
});

function master(label: string, firstLight: string): PipelineMasterOutput {
  return {
    label,
    png_path: `/out/run_${label}.png`,
    fits_path: `/out/run_${label}.fits`,
    dimensions: [1600, 1500],
    input_path: firstLight,
    previewUrl: `asset://out/run_${label}.png`,
  };
}

function channelStats(label: string, lights: number) {
  return { label, lights_input: lights, mean: 1, stddev: 1 };
}

function pipelineRun(overrides: Partial<PipelineResult> = {}): PipelineResult {
  return {
    stats: {
      darks_combined: 0,
      flats_combined: 0,
      bias_combined: 0,
      channels: [channelStats("G", 2), channelStats("R", 1), channelStats("B", 3)],
    },
    channel_previews: ["G", "R", "B"].map((label) => ({ label, pixels_b64: "", width: 800, height: 750 })),
    rgb_preview: "AAAA",
    masters: [master("G", "/subs/656nmos.fits"), master("R", "/subs/673nmos.fits"), master("B", "/subs/502nmos.fits")],
    rgb_png_path: "/out/run_rgb.png",
    rgb_dimensions: [800, 750],
    rgbPreviewUrl: "asset://out/run_rgb.png",
    ...overrides,
  };
}

describe("pipelineViewOutput", () => {
  it("shows the RGB as a PNG-only stacking result without a pixel grid, on the R channel's first light, counting every light", () => {
    expect(pipelineViewOutput(pipelineRun(), "RGB")).toEqual({
      output: {
        fitsPath: null,
        previewUrl: "asset://out/run_rgb.png",
        dimensions: null,
        kind: "stacking",
        inputPath: "/subs/673nmos.fits",
      },
      label: "Pipeline RGB · 6 frames",
    });
  });

  it("references the RGB to the first master's light when there is no R channel", () => {
    const run = pipelineRun({ masters: [master("G", "/subs/656nmos.fits"), master("B", "/subs/502nmos.fits")] });
    expect(pipelineViewOutput(run, "RGB")?.output.inputPath).toBe("/subs/656nmos.fits");
  });

  it("shows a channel as its master FITS, with the master's full size and the channel's first light", () => {
    expect(pipelineViewOutput(pipelineRun(), "B")).toEqual({
      output: {
        fitsPath: "/out/run_B.fits",
        previewUrl: "asset://out/run_B.png",
        dimensions: [1600, 1500],
        kind: "stacking",
        inputPath: "/subs/502nmos.fits",
      },
      label: "Pipeline B · 3 frames",
    });
    expect(pipelineViewOutput(pipelineRun(), "R")?.label).toBe("Pipeline R · 1 frame");
  });

  it("has nothing to show for a channel the run did not build or an RGB it did not write", () => {
    expect(pipelineViewOutput(pipelineRun(), "L")).toBeNull();
    expect(pipelineViewOutput(pipelineRun({ rgb_png_path: null, rgbPreviewUrl: undefined }), "RGB")).toBeNull();
    expect(pipelineViewOutput(pipelineRun({ masters: undefined }), "G")).toBeNull();
  });
});

describe("pipelineInitialChoice", () => {
  it("shows the RGB after a run that composed one", () => {
    expect(pipelineInitialChoice(pipelineRun())).toBe("RGB");
  });

  it("shows the first channel when no RGB was composed, and nothing when no channel was built", () => {
    expect(pipelineInitialChoice(pipelineRun({ rgb_preview: undefined, rgb_png_path: null, rgbPreviewUrl: undefined }))).toBe("G");
    expect(pipelineInitialChoice(pipelineRun({ rgb_preview: undefined, channel_previews: [] }))).toBeNull();
  });
});

describe("parksWizardComposite", () => {
  const onScreen = { targetKey: "1|/d/673nmos.fits", currentKey: "1|/d/673nmos.fits", wizardCompositeOnScreen: true, wizardCompositeReady: true };

  it("parks a finished wizard composite that would hide an output published to the file on screen", () => {
    expect(parksWizardComposite(onScreen)).toBe(true);
  });

  it("leaves the composite on screen when the output goes to another file", () => {
    expect(parksWizardComposite({ ...onScreen, currentKey: "2|/d/656nmos.fits" })).toBe(false);
    expect(parksWizardComposite({ ...onScreen, currentKey: null })).toBe(false);
  });

  it("does nothing when no wizard composite is on screen", () => {
    expect(parksWizardComposite({ ...onScreen, wizardCompositeOnScreen: false })).toBe(false);
  });

  it("does not park a composite the wizard has not finished building", () => {
    expect(parksWizardComposite({ ...onScreen, wizardCompositeReady: false })).toBe(false);
  });
});
