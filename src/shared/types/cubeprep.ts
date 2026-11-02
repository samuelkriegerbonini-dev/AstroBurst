export const CUBE_PREP_STEPS = ["crop", "units", "align", "cube", "edit", "psfs", "kernels", "match", "export"] as const;
export type CubePrepStepName = (typeof CUBE_PREP_STEPS)[number];

export interface CubePrepInput {
  path: string;
  dataset: string;
  filter: string;
}

export interface CubePrepStep {
  step: CubePrepStepName;
  [parameter: string]: unknown;
}

export interface CubePrepRecipe {
  recipe_version: 1;
  name?: string | null;
  inputs: CubePrepInput[];
  steps: CubePrepStep[];
}

export interface CubePrepFilter {
  dataset: string;
  filter: string;
  instrument: string;
  group: string;
  pivot_angstrom: number;
  aliases: string[];
}

export interface CubePrepFilters {
  datasets: string[];
  filters: CubePrepFilter[];
}

export interface CubePrepValidation {
  recipe: CubePrepRecipe;
  dependencies: string[];
  warnings: string[];
}

export interface CubePrepProgress {
  current: number;
  total: number;
  percent: number;
  stage: string;
}

export interface CubePrepStepReport {
  index: number;
  step: string;
  directory: string;
  parameters: unknown;
  report: unknown;
  warnings: string[];
  elapsed_ms: number;
}

export interface CubePrepReport {
  recipe_version: number;
  name: string | null;
  output_dir: string;
  cube: string | null;
  steps: CubePrepStepReport[];
  files: { path: string; bytes: number; sha256?: string }[];
  warnings: string[];
  elapsed_ms: number;
  provenance?: unknown;
}
