use std::sync::Arc;
use serde_json::json;
use crate::cmd::catalog::{cone_query_for_field, FieldGeometry};
use crate::types::constants::{RES_GMAG, RES_BP_RP, RES_SEPARATION_ARCSEC};
use crate::core::astrometry::catalog::{CatalogHit, CatalogRow, ConeQuery, GAIA_MATCH_FALLBACK_ROWS, GAIA_MATCH_MAG_LIMIT};

pub(in crate::cmd::analysis) const GAIA_MATCH_RADIUS_ARCSEC: f64 = 5.0;
pub(in crate::cmd::analysis) const GAIA_MATCH_CONE_DEG: f64 = 0.01;
const ARCSEC_PER_ARCMIN: f64 = 60.0;

pub(in crate::cmd::analysis) fn gaia_match_outcome(coord_ra: f64, coord_dec: f64, rows: &[&CatalogRow]) -> (serde_json::Value, Option<String>) {
    let cos_dec = coord_dec.to_radians().cos();
    let mut best: Option<(f64, usize)> = None;
    for (i, s) in rows.iter().enumerate() {
        let mut dra = (coord_ra - s.ra).abs();
        if dra > 180.0 {
            dra = 360.0 - dra;
        }
        let sep = ((dra * cos_dec).powi(2) + (coord_dec - s.dec).powi(2)).sqrt() * 3600.0;
        if sep < GAIA_MATCH_RADIUS_ARCSEC && best.map_or(true, |(bd, _)| sep < bd) {
            best = Some((sep, i));
        }
    }
    match best {
        Some((sep, i)) => (
            json!({
                RES_GMAG: rows[i].g,
                RES_BP_RP: rows[i].bp_rp,
                RES_SEPARATION_ARCSEC: sep,
            }),
            None,
        ),
        None => (
            serde_json::Value::Null,
            Some(format!("Gaia: no G<17 star within {GAIA_MATCH_RADIUS_ARCSEC}\"")),
        ),
    }
}

pub(crate) fn gaia_rows_for_match(
    coord: (f64, f64),
    field: &ConeQuery,
    cached: &mut dyn FnMut(&ConeQuery) -> Result<CatalogHit, String>,
) -> Result<Arc<Vec<CatalogRow>>, String> {
    let hit = cached(field)?;
    if hit.rows.len() < field.max_rows {
        return Ok(hit.rows);
    }
    let per_star = ConeQuery {
        ra: coord.0,
        dec: coord.1,
        radius_deg: GAIA_MATCH_CONE_DEG,
        mag_limit: Some(GAIA_MATCH_MAG_LIMIT),
        max_rows: GAIA_MATCH_FALLBACK_ROWS,
    };
    Ok(cached(&per_star)?.rows)
}

pub(in crate::cmd::analysis) fn gaia_match_field(geometry: &FieldGeometry) -> ConeQuery {
    cone_query_for_field(
        geometry,
        Some(geometry.diagonal_arcmin / 2.0 + GAIA_MATCH_RADIUS_ARCSEC / ARCSEC_PER_ARCMIN),
        Some(GAIA_MATCH_MAG_LIMIT),
        None,
    )
}
