import { safeInvoke } from "../infrastructure/tauri";

export interface SynthFieldConfig {
  width: number;
  height: number;
  n_stars: number;
  flux_min: number;
  flux_max: number;
  seed: number;
}

export interface SynthNoiseParams {
  gain: number;
  readout_noise: number;
  sky_background: number;
  dark_current: number;
  exposure_time: number;
  bias_level: number;
  seed: number;
}

export type FieldType =
  | "Uniform"
  | { KingCluster: { core_radius: number; tidal_radius: number } }
  | { ExponentialDisk: { scale_length: number; inclination_deg: number } };

export type PsfType =
  | { Gaussian: { fwhm: number } }
  | { Moffat: { fwhm: number; beta: number } }
  | { Airy: { lambda_over_d: number } };

export interface SynthConfig {
  field: SynthFieldConfig;
  field_type: FieldType;
  psf_type: PsfType;
  noise: SynthNoiseParams;
  apply_vignette: boolean;
  vignette_strength: number;
  n_frames: number;
}

export interface SynthResult {
  width: number;
  height: number;
  star_count: number;
  output_path: string | null;
}

export function generateSynth(
  config: SynthConfig,
  outputPath: string,
  saveCatalog = false,
  catalogPath?: string,
  saveGroundTruth = false,
  groundTruthPath?: string,
): Promise<SynthResult> {
  return safeInvoke("generate_synth_cmd", {
    args: {
      config,
      output_path: outputPath,
      save_catalog: saveCatalog,
      catalog_path: catalogPath ?? null,
      save_ground_truth: saveGroundTruth,
      ground_truth_path: groundTruthPath ?? null,
    },
  });
}

export function generateSynthStack(
  config: SynthConfig,
  outputDir: string,
  prefix = "synth",
): Promise<SynthResult> {
  return safeInvoke("generate_synth_stack_cmd", {
    args: { config, output_dir: outputDir, prefix },
  });
}
