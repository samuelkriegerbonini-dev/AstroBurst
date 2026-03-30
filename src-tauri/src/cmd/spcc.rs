use serde_json::json;

use crate::cmd::common::{blocking_cmd, load_from_cache_or_disk};
use crate::core::astrometry::spcc::{
    spcc_calibrate_rgb, SpccConfig, SpccCatalog, WhiteReference,
};
use crate::types::constants::{
    RES_ELAPSED_MS, RES_R_FACTOR, RES_G_FACTOR, RES_B_FACTOR,
    RES_STARS_MATCHED, RES_STARS_TOTAL, RES_AVG_COLOR_INDEX,
    RES_WHITE_REF, RES_CATALOG_NAME,
};

#[tauri::command]
pub async fn spcc_calibrate_cmd(
    r_path: String,
    g_path: String,
    b_path: String,
    wcs_path: Option<String>,
    white_reference: Option<String>,
    min_snr: Option<f64>,
    max_stars: Option<usize>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let r_entry = load_from_cache_or_disk(&r_path)?;
        let g_entry = load_from_cache_or_disk(&g_path)?;
        let b_entry = load_from_cache_or_disk(&b_path)?;

        let header_source = if let Some(ref wp) = wcs_path {
            load_from_cache_or_disk(wp)?
        } else {
            r_entry.clone()
        };

        let header = header_source
            .header()
            .ok_or_else(|| anyhow::anyhow!("No FITS header found. Run Plate Solve first to embed WCS."))?
            .clone();

        let wr = match white_reference.as_deref() {
            Some("g2v") | Some("G2V") => WhiteReference::G2V,
            Some("photopic") => WhiteReference::Photopic,
            Some("spiral") | Some("average_spiral") | None => WhiteReference::AverageSpiral,
            _ => WhiteReference::AverageSpiral,
        };

        let config = SpccConfig {
            min_snr: min_snr.unwrap_or(20.0),
            max_stars: max_stars.unwrap_or(200),
            catalog: SpccCatalog::BuiltinBpRp,
            white_reference: wr,
            ..SpccConfig::default()
        };

        let t0 = std::time::Instant::now();
        let result = spcc_calibrate_rgb(
            r_entry.arr(),
            g_entry.arr(),
            b_entry.arr(),
            &header,
            &config,
        ).map_err(|e| anyhow::anyhow!(e))?;
        let elapsed_ms = t0.elapsed().as_millis() as u64;

        Ok(json!({
            RES_R_FACTOR: result.r_factor,
            RES_G_FACTOR: result.g_factor,
            RES_B_FACTOR: result.b_factor,
            RES_STARS_MATCHED: result.stars_matched,
            RES_STARS_TOTAL: result.stars_total,
            RES_AVG_COLOR_INDEX: result.avg_color_index,
            RES_WHITE_REF: result.white_ref_name,
            RES_CATALOG_NAME: result.catalog_name,
            "is_synthetic_catalog": result.is_synthetic_catalog,
            RES_ELAPSED_MS: elapsed_ms,
        }))
    })
}
