export interface AppConfig {
  astrometry_api_key: string | null;
  astrometry_api_url: string;
  default_output_dir: string | null;
  plate_solve_timeout_secs: number;
  plate_solve_max_stars: number;
  auto_stretch_target_bg: number;
  auto_stretch_shadow_k: number;
}

export interface ApiKeyResult {
  key: string | null;
  service: string;
}
