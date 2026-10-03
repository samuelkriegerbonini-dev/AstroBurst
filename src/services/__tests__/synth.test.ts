import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

const { typedInvokeMock } = vi.hoisted(() => ({ typedInvokeMock: vi.fn() }));

vi.mock("../../infrastructure/tauri", () => ({ typedInvoke: typedInvokeMock }));

import { onCommandOutputs } from "../../infrastructure/tauri/outputEvents";
import { writtenFitsPaths } from "../../utils/overwrittenFiles";
import { generateSynth, generateSynthStack, type SynthConfig } from "../synth";

const config: SynthConfig = {
  field: { width: 64, height: 64, n_stars: 5, flux_min: 2000, flux_max: 500000, seed: 1 },
  field_type: "Uniform",
  psf_type: { Gaussian: { fwhm: 3 } },
  noise: { gain: 1.5, readout_noise: 8, sky_background: 200, dark_current: 0.05, exposure_time: 300, bias_level: 1000 },
  apply_vignette: false,
  vignette_strength: 0.3,
  n_frames: 1,
};

describe("generateSynth output announcements", () => {
  const announced: unknown[] = [];
  let off: () => void = () => {};

  beforeEach(() => {
    announced.length = 0;
    typedInvokeMock.mockReset();
    off = onCommandOutputs((result) => announced.push(result));
  });

  afterEach(() => off());

  it("announces the written FITS so an open tab of an overwritten file reloads", async () => {
    const res = { output_path: "C:/x/synthetic.fits", width: 1, height: 1, star_count: 0 };
    typedInvokeMock.mockResolvedValue(res);

    await expect(generateSynth(config, "C:/x/synthetic.fits")).resolves.toEqual(res);

    expect(announced.flatMap(writtenFitsPaths)).toEqual(["C:/x/synthetic.fits"]);
  });

  it("also announces the ground truth when it is saved", async () => {
    typedInvokeMock.mockResolvedValue({ output_path: "C:/x/s.fits", width: 1, height: 1, star_count: 0 });

    await generateSynth(config, "C:/x/s.fits", true, "C:/x/s_catalog.csv", true, "C:/x/s_groundtruth.fits");

    expect(announced.flatMap(writtenFitsPaths)).toEqual(["C:/x/s.fits", "C:/x/s_groundtruth.fits"]);
  });

  it("still announces the requested FITS and ground truth when the command fails after writing them", async () => {
    typedInvokeMock.mockRejectedValue(new Error("catalog locked"));

    await expect(
      generateSynth(config, "C:/x/s.fits", true, "C:/x/s_catalog.csv", true, "C:/x/s_groundtruth.fits"),
    ).rejects.toThrow("catalog locked");

    expect(announced.flatMap(writtenFitsPaths)).toEqual(["C:/x/s.fits", "C:/x/s_groundtruth.fits"]);
  });

  it("announces only the FITS on failure when no ground truth was requested", async () => {
    typedInvokeMock.mockRejectedValue(new Error("disk full"));

    await expect(generateSynth(config, "C:/x/s.fits")).rejects.toThrow("disk full");

    expect(announced.flatMap(writtenFitsPaths)).toEqual(["C:/x/s.fits"]);
  });

  it("does not announce stack outputs, which are always new files", async () => {
    typedInvokeMock.mockResolvedValue({ output_path: "C:/x/run", width: 1, height: 1, star_count: 0 });

    await generateSynthStack(config, "C:/x/run");

    expect(announced).toEqual([]);
  });
});

describe("generateSynthStack arguments", () => {
  beforeEach(() => {
    typedInvokeMock.mockReset();
    typedInvokeMock.mockResolvedValue({ output_path: "C:/x/run", width: 1, height: 1, star_count: 0 });
  });

  it("sends the catalog and ground-truth flags", async () => {
    await generateSynthStack(config, "C:/x/run", "synth", true, true);

    expect(typedInvokeMock).toHaveBeenCalledWith("generate_synth_stack_cmd", {
      args: { config, output_dir: "C:/x/run", prefix: "synth", save_catalog: true, save_ground_truth: true },
    });
  });

  it("defaults both flags to false", async () => {
    await generateSynthStack(config, "C:/x/run");

    const args = typedInvokeMock.mock.calls[0][1] as { args: Record<string, unknown> };
    expect(args.args.save_catalog).toBe(false);
    expect(args.args.save_ground_truth).toBe(false);
  });
});
