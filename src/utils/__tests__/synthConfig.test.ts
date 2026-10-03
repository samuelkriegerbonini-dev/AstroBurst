import { describe, it, expect } from "vitest";
import {
  SYNTH_PANEL_DEFAULTS,
  buildSynthConfig,
  nextRandomSeed,
  synthResultCard,
  synthSettingsSignature,
  type SynthPanelState,
} from "../synthConfig";

function state(overrides: Partial<SynthPanelState> = {}): SynthPanelState {
  return { ...SYNTH_PANEL_DEFAULTS, ...overrides };
}

describe("SYNTH_PANEL_DEFAULTS", () => {
  it("uses flux defaults in total electrons per frame that make most stars detectable", () => {
    expect(SYNTH_PANEL_DEFAULTS.fluxMin).toBe(2000);
    expect(SYNTH_PANEL_DEFAULTS.fluxMax).toBe(500000);
  });

  it("varies stack frames by default with a 3 px dither, 10 % seeing jitter and cosmic rays", () => {
    expect(SYNTH_PANEL_DEFAULTS.varyFrames).toBe(true);
    expect(SYNTH_PANEL_DEFAULTS.ditherPx).toBe(3);
    expect(SYNTH_PANEL_DEFAULTS.seeingJitterPct).toBe(10);
    expect(SYNTH_PANEL_DEFAULTS.cosmicRays).toBe(true);
  });
});

describe("buildSynthConfig", () => {
  it("sends one seed and no derived noise seed", () => {
    const config = buildSynthConfig(state({ seed: 77 }));
    expect(config.field.seed).toBe(77);
    expect(config.noise).not.toHaveProperty("seed");
    expect(JSON.stringify(config)).not.toContain("1077");
  });

  it("maps the panel defaults onto the backend field and noise blocks", () => {
    const config = buildSynthConfig(state());
    expect(config.field).toEqual({ width: 2048, height: 2048, n_stars: 500, flux_min: 2000, flux_max: 500000, seed: 42 });
    expect(config.noise).toEqual({ gain: 1.5, readout_noise: 8, sky_background: 200, dark_current: 0.05, exposure_time: 300, bias_level: 1000 });
    expect(config.field_type).toBe("Uniform");
    expect(config.psf_type).toEqual({ Gaussian: { fwhm: 3 } });
    expect(config.n_frames).toBe(1);
  });

  it("builds king, disk, moffat and airy variants from their own parameters", () => {
    expect(buildSynthConfig(state({ fieldChoice: "king", coreRadius: 20, tidalRadius: 300 })).field_type)
      .toEqual({ KingCluster: { core_radius: 20, tidal_radius: 300 } });
    expect(buildSynthConfig(state({ fieldChoice: "disk", scaleLength: 150, inclination: 45 })).field_type)
      .toEqual({ ExponentialDisk: { scale_length: 150, inclination_deg: 45 } });
    expect(buildSynthConfig(state({ psfChoice: "moffat", fwhm: 4, beta: 2.5 })).psf_type)
      .toEqual({ Moffat: { fwhm: 4, beta: 2.5 } });
    expect(buildSynthConfig(state({ psfChoice: "airy", lambdaD: 1.5 })).psf_type)
      .toEqual({ Airy: { lambda_over_d: 1.5 } });
  });

  it("leaves frame_variation out of single-image configs", () => {
    const config = buildSynthConfig(state({ stackMode: false }));
    expect(config).not.toHaveProperty("frame_variation");
  });

  it("sends the default frame variation in stack mode", () => {
    const config = buildSynthConfig(state({ stackMode: true, nFrames: 6 }));
    expect(config.n_frames).toBe(6);
    expect(config.frame_variation).toEqual({
      enabled: true,
      dither_px: 3,
      fwhm_jitter: 0.1,
      sky_jitter: 0.05,
      transparency_jitter: 0.05,
      cosmic_rays_per_megapixel: 25,
    });
  });

  it("turns the seeing percentage into a fraction and the cosmic-ray toggle into a rate", () => {
    const config = buildSynthConfig(state({ stackMode: true, seeingJitterPct: 25, ditherPx: 7.5, cosmicRays: false }));
    expect(config.frame_variation?.fwhm_jitter).toBeCloseTo(0.25, 12);
    expect(config.frame_variation?.dither_px).toBe(7.5);
    expect(config.frame_variation?.cosmic_rays_per_megapixel).toBe(0);
  });

  it("disables frame variation when Vary frames is off", () => {
    const config = buildSynthConfig(state({ stackMode: true, varyFrames: false }));
    expect(config.frame_variation?.enabled).toBe(false);
  });
});

describe("synthSettingsSignature", () => {
  const base = { stackMode: false, saveCatalog: true, saveGroundTruth: false };

  it("is stable for the same settings", () => {
    expect(synthSettingsSignature(buildSynthConfig(state()), base)).toBe(synthSettingsSignature(buildSynthConfig(state()), base));
  });

  it("changes when any generation parameter changes", () => {
    const ref = synthSettingsSignature(buildSynthConfig(state()), base);
    expect(synthSettingsSignature(buildSynthConfig(state({ width: 1024 })), base)).not.toBe(ref);
    expect(synthSettingsSignature(buildSynthConfig(state({ seed: 43 })), base)).not.toBe(ref);
    expect(synthSettingsSignature(buildSynthConfig(state({ psfChoice: "airy" })), base)).not.toBe(ref);
    expect(synthSettingsSignature(buildSynthConfig(state()), { ...base, saveGroundTruth: true })).not.toBe(ref);
  });
});

describe("nextRandomSeed", () => {
  it("draws a seed in [0, 9999]", () => {
    expect(nextRandomSeed(42, () => 0)).toBe(0);
    expect(nextRandomSeed(42, () => 0.99999999)).toBe(9999);
    expect(nextRandomSeed(42, () => 0.5)).toBe(5000);
  });

  it("never returns the current seed", () => {
    expect(nextRandomSeed(5000, () => 0.5)).toBe(5001);
    expect(nextRandomSeed(9999, () => 0.99999999)).toBe(0);
  });
});

describe("synthResultCard", () => {
  it("reports the generated size from the response, not the requested size", () => {
    const card = synthResultCard(
      { width: 512, height: 256, star_count: 120, output_path: "C:/d/synthetic.fits" },
      { kind: "single", fallbackPath: "C:/d/other.fits", signature: "sig" },
    );
    expect(card).toMatchObject({ width: 512, height: 256, stars: 120, path: "C:/d/synthetic.fits", openPath: "C:/d/synthetic.fits", manifestPath: null, signature: "sig" });
  });

  it("falls back to the requested path when the backend returns none", () => {
    const card = synthResultCard(
      { width: 64, height: 64, star_count: 1, output_path: null },
      { kind: "single", fallbackPath: "C:/d/synthetic.fits", signature: "s" },
    );
    expect(card.path).toBe("C:/d/synthetic.fits");
    expect(card.openPath).toBe("C:/d/synthetic.fits");
  });

  it("keeps the frames manifest of a stack and offers no single file to open", () => {
    const card = synthResultCard(
      { width: 512, height: 512, star_count: 500, output_path: "C:/d/run", frames_manifest_path: "C:/d/run/synth_frames.csv" },
      { kind: "stack", fallbackPath: "C:/d/run", signature: "s" },
    );
    expect(card.manifestPath).toBe("C:/d/run/synth_frames.csv");
    expect(card.openPath).toBeNull();
  });

  it("lists the saved sidecar files and nothing for unsaved ones", () => {
    const saved = synthResultCard(
      { width: 8, height: 8, star_count: 1, output_path: "C:/d/s.fits" },
      { kind: "single", fallbackPath: "C:/d/s.fits", signature: "s", catalogPath: "C:/d/s_catalog.csv", groundTruthPath: "C:/d/s_groundtruth.fits" },
    );
    expect(saved.catalogPath).toBe("C:/d/s_catalog.csv");
    expect(saved.groundTruthPath).toBe("C:/d/s_groundtruth.fits");
    const bare = synthResultCard(
      { width: 8, height: 8, star_count: 1, output_path: "C:/d/s.fits" },
      { kind: "single", fallbackPath: "C:/d/s.fits", signature: "s" },
    );
    expect(bare.catalogPath).toBeNull();
    expect(bare.groundTruthPath).toBeNull();
  });

  it("treats a missing or empty manifest path as absent", () => {
    const card = synthResultCard(
      { width: 512, height: 512, star_count: 500, output_path: "C:/d/run", frames_manifest_path: "" },
      { kind: "stack", fallbackPath: "C:/d/run", signature: "s" },
    );
    expect(card.manifestPath).toBeNull();
  });
});
