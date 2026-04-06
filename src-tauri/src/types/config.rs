use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub astrometry_api_key: Option<String>,
    pub astrometry_api_url: String,
    pub default_output_dir: Option<String>,
    pub plate_solve_timeout_secs: u64,
    pub plate_solve_max_stars: usize,
    pub auto_stretch_target_bg: f64,
    pub auto_stretch_shadow_k: f64,
    #[serde(default)]
    pub output_max_size_mb: Option<u64>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            astrometry_api_key: None,
            astrometry_api_url: "https://nova.astrometry.net".into(),
            default_output_dir: None,
            plate_solve_timeout_secs: 120,
            plate_solve_max_stars: 100,
            auto_stretch_target_bg: 0.25,
            auto_stretch_shadow_k: -2.8,
            output_max_size_mb: None,
        }
    }
}
