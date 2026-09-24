export interface AppConfig {
  astrometry_api_key: string | null;
  astrometry_api_url: string;
  plate_solve_timeout_secs: number;
  output_max_size_mb: number | null;
}

export interface ApiKeyResult {
  key: string | null;
  service: string;
}
