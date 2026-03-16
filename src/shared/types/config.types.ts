export interface AppConfig {
  has_api_key: boolean;
  astrometry_api_url: string;
  default_output_dir: string;
  plate_solve_timeout_secs: number;
  plate_solve_max_stars: number;
  auto_stretch_target_bg: number;
  auto_stretch_shadow_k: number;
}
