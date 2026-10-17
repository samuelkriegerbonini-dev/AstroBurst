use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub astrometry_api_key: Option<String>,
    pub astrometry_api_url: String,
    pub plate_solve_timeout_secs: u64,
    #[serde(default)]
    pub output_max_size_mb: Option<u64>,
    #[serde(default)]
    pub mast_token_transport: Option<String>,
    #[serde(default)]
    pub data_dir_override: Option<String>,
    #[serde(default)]
    pub mast_quota_gb: Option<f64>,
    #[serde(default)]
    pub external_catalogs: BTreeMap<String, String>,
    #[serde(default)]
    pub siril_catalog_dir: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            astrometry_api_key: None,
            astrometry_api_url: "https://nova.astrometry.net".into(),
            plate_solve_timeout_secs: 120,
            output_max_size_mb: None,
            mast_token_transport: None,
            data_dir_override: None,
            mast_quota_gb: None,
            external_catalogs: BTreeMap::new(),
            siril_catalog_dir: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    const RETIRED_FIELDS: [&str; 4] = [
        "default_output_dir",
        "plate_solve_max_stars",
        "auto_stretch_target_bg",
        "auto_stretch_shadow_k",
    ];

    #[test]
    fn settings_that_nothing_reads_are_not_reported() {
        let value = serde_json::to_value(AppConfig::default()).unwrap();
        for field in RETIRED_FIELDS {
            assert!(value.get(field).is_none(), "{field} is still sent to the settings panel");
        }
    }

    #[test]
    fn app_config_round_trips_mast_token_transport() {
        let stored = serde_json::json!({
            "astrometry_api_key": null,
            "astrometry_api_url": "https://nova.astrometry.net",
            "plate_solve_timeout_secs": 120,
            "mast_token_transport": "cookie"
        });
        let config: AppConfig = serde_json::from_value(stored).unwrap();
        let written = serde_json::to_value(&config).unwrap();
        assert_eq!(written["mast_token_transport"], serde_json::json!("cookie"));
        let defaults = serde_json::to_value(AppConfig::default()).unwrap();
        assert_eq!(defaults["mast_token_transport"], serde_json::Value::Null);
    }

    #[test]
    fn a_stored_config_with_retired_settings_still_loads() {
        let stored = serde_json::json!({
            "astrometry_api_key": null,
            "astrometry_api_url": "https://nova.astrometry.net",
            "default_output_dir": "/data/out",
            "plate_solve_timeout_secs": 300,
            "plate_solve_max_stars": 250,
            "auto_stretch_target_bg": 0.15,
            "auto_stretch_shadow_k": -1.5
        });
        let config: AppConfig = serde_json::from_value(stored).unwrap();
        assert_eq!(config.plate_solve_timeout_secs, 300);
        let written = serde_json::to_value(&config).unwrap();
        for field in RETIRED_FIELDS {
            assert!(written.get(field).is_none(), "{field} was written back");
        }
    }
}
