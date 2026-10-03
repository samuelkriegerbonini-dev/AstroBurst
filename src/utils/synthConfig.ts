import type { FieldType, PsfType, SynthConfig, SynthFrameVariation, SynthResult } from "../services/synth";

export type SynthFieldChoice = "uniform" | "king" | "disk";
export type SynthPsfChoice = "gaussian" | "moffat" | "airy";

export interface SynthPanelState {
  width: number;
  height: number;
  nStars: number;
  fluxMin: number;
  fluxMax: number;
  seed: number;
  fieldChoice: SynthFieldChoice;
  coreRadius: number;
  tidalRadius: number;
  scaleLength: number;
  inclination: number;
  psfChoice: SynthPsfChoice;
  fwhm: number;
  beta: number;
  lambdaD: number;
  gain: number;
  readNoise: number;
  skyBg: number;
  darkCurrent: number;
  expTime: number;
  biasLevel: number;
  vignette: boolean;
  vigStrength: number;
  stackMode: boolean;
  nFrames: number;
  varyFrames: boolean;
  ditherPx: number;
  seeingJitterPct: number;
  cosmicRays: boolean;
}

export const SYNTH_PANEL_DEFAULTS: SynthPanelState = {
  width: 2048,
  height: 2048,
  nStars: 500,
  fluxMin: 2000,
  fluxMax: 500000,
  seed: 42,
  fieldChoice: "uniform",
  coreRadius: 50,
  tidalRadius: 400,
  scaleLength: 200,
  inclination: 30,
  psfChoice: "gaussian",
  fwhm: 3,
  beta: 4,
  lambdaD: 2.5,
  gain: 1.5,
  readNoise: 8,
  skyBg: 200,
  darkCurrent: 0.05,
  expTime: 300,
  biasLevel: 1000,
  vignette: false,
  vigStrength: 0.3,
  stackMode: false,
  nFrames: 8,
  varyFrames: true,
  ditherPx: 3,
  seeingJitterPct: 10,
  cosmicRays: true,
};

export const SYNTH_SEED_MAX = 9999;
const SKY_JITTER = 0.05;
const TRANSPARENCY_JITTER = 0.05;
const COSMIC_RAYS_PER_MEGAPIXEL = 25;

function fieldType(s: SynthPanelState): FieldType {
  switch (s.fieldChoice) {
    case "king": return { KingCluster: { core_radius: s.coreRadius, tidal_radius: s.tidalRadius } };
    case "disk": return { ExponentialDisk: { scale_length: s.scaleLength, inclination_deg: s.inclination } };
    default: return "Uniform";
  }
}

function psfType(s: SynthPanelState): PsfType {
  switch (s.psfChoice) {
    case "moffat": return { Moffat: { fwhm: s.fwhm, beta: s.beta } };
    case "airy": return { Airy: { lambda_over_d: s.lambdaD } };
    default: return { Gaussian: { fwhm: s.fwhm } };
  }
}

function frameVariation(s: SynthPanelState): SynthFrameVariation {
  return {
    enabled: s.varyFrames,
    dither_px: s.ditherPx,
    fwhm_jitter: s.seeingJitterPct / 100,
    sky_jitter: SKY_JITTER,
    transparency_jitter: TRANSPARENCY_JITTER,
    cosmic_rays_per_megapixel: s.cosmicRays ? COSMIC_RAYS_PER_MEGAPIXEL : 0,
  };
}

export function buildSynthConfig(s: SynthPanelState): SynthConfig {
  const config: SynthConfig = {
    field: { width: s.width, height: s.height, n_stars: s.nStars, flux_min: s.fluxMin, flux_max: s.fluxMax, seed: s.seed },
    field_type: fieldType(s),
    psf_type: psfType(s),
    noise: { gain: s.gain, readout_noise: s.readNoise, sky_background: s.skyBg, dark_current: s.darkCurrent, exposure_time: s.expTime, bias_level: s.biasLevel },
    apply_vignette: s.vignette,
    vignette_strength: s.vigStrength,
    n_frames: s.stackMode ? s.nFrames : 1,
  };
  if (s.stackMode) config.frame_variation = frameVariation(s);
  return config;
}

export interface SynthOutputChoices {
  stackMode: boolean;
  saveCatalog: boolean;
  saveGroundTruth: boolean;
}

export function synthSettingsSignature(config: SynthConfig, choices: SynthOutputChoices): string {
  return JSON.stringify([config, choices.stackMode, choices.saveCatalog, choices.saveGroundTruth]);
}

export function nextRandomSeed(current: number, random: () => number = Math.random): number {
  const span = SYNTH_SEED_MAX + 1;
  const drawn = Math.min(SYNTH_SEED_MAX, Math.floor(random() * span));
  return drawn === current ? (drawn + 1) % span : drawn;
}

export interface SynthResultCard {
  kind: "single" | "stack";
  width: number;
  height: number;
  stars: number;
  path: string;
  openPath: string | null;
  manifestPath: string | null;
  catalogPath: string | null;
  groundTruthPath: string | null;
  signature: string;
}

export interface SynthResultContext {
  kind: "single" | "stack";
  fallbackPath: string;
  signature: string;
  catalogPath?: string | null;
  groundTruthPath?: string | null;
}

export function synthResultCard(res: SynthResult, ctx: SynthResultContext): SynthResultCard {
  const path = res.output_path || ctx.fallbackPath;
  return {
    kind: ctx.kind,
    width: res.width,
    height: res.height,
    stars: res.star_count,
    path,
    openPath: ctx.kind === "single" ? path : null,
    manifestPath: res.frames_manifest_path || null,
    catalogPath: ctx.catalogPath || null,
    groundTruthPath: ctx.groundTruthPath || null,
    signature: ctx.signature,
  };
}
