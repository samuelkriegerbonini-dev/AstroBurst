use crate::core::astrometry::gwcs::SipFitOptions;
use crate::core::registration::poly::{self, PolyModel, TanParams};

use crate::core::astrometry::local_solve::constants::{ARCSEC_PER_DEG, CLIP_FLOOR_PX, MAX_CLIP_ROUNDS, SIP_FOLDS};
use crate::core::astrometry::local_solve::types::{SkyPair, DistortionFit, LocalSolveParams};
use super::tan_projection::{sky_to_pixel, scale_arcsec, tan_cards};
use super::tan_fit::clipped_rms;
use super::verification::quadrants;

fn plain_tan(tan: &TanParams) -> DistortionFit {
    DistortionFit {
        order: 1,
        considered: Vec::new(),
        wcs_cards: tan_cards(tan),
        sip_max_err_px: None,
        tan: *tan,
    }
}

#[derive(Clone)]
pub(in crate::core::astrometry::local_solve) struct SipModel {
    pub(in crate::core::astrometry::local_solve) forward: PolyModel,
    pub(in crate::core::astrometry::local_solve) inverse: PolyModel,
    pub(in crate::core::astrometry::local_solve) kept: Vec<SkyPair>,
}

pub(in crate::core::astrometry::local_solve) struct SipSelection {
    pub(in crate::core::astrometry::local_solve) considered: Vec<(u8, f64, f64)>,
    pub(in crate::core::astrometry::local_solve) model: Option<SipModel>,
}

pub(in crate::core::astrometry::local_solve) fn poly_pairs(pairs: &[SkyPair], tan: &TanParams) -> Vec<(f64, f64, f64, f64)> {
    pairs
        .iter()
        .filter_map(|p| sky_to_pixel(tan, p.ra, p.dec).map(|(tx, ty)| (p.x, p.y, tx, ty)))
        .collect()
}

fn max_sip_order(tan: &TanParams, size: (usize, usize), params: &LocalSolveParams) -> u8 {
    if let Some(cap) = params.sip_max_order {
        return cap;
    }
    let long_side_deg = size.0.max(size.1) as f64 * scale_arcsec(&tan.cd) / ARCSEC_PER_DEG;
    if long_side_deg >= params.sip_order_4_above_deg {
        4
    } else {
        3
    }
}

pub(in crate::core::astrometry::local_solve) fn fit_sip_model(
    pairs: &[SkyPair],
    tan: &TanParams,
    size: (usize, usize),
    params: &LocalSolveParams,
) -> Option<SipSelection> {
    if pairs.len() < params.sip_min_inliers || quadrants(pairs, size) < params.sip_min_quadrants {
        return None;
    }
    let max_order = max_sip_order(tan, size, params);
    let mut kept: Vec<SkyPair> = pairs.to_vec();
    for round in 0..MAX_CLIP_ROUNDS {
        let rows = poly_pairs(&kept, tan);
        let selection = poly::fit_select(&rows, 1..=max_order, SIP_FOLDS)?;
        let residuals = selection.model.residuals(&rows);
        let threshold =
            (params.clip_sigma * clipped_rms(&residuals, params.clip_sigma)).max(CLIP_FLOOR_PX);
        let next: Vec<SkyPair> = kept
            .iter()
            .zip(&residuals)
            .filter(|(_, r)| **r <= threshold)
            .map(|(p, _)| *p)
            .collect();
        let stable = next.len() == kept.len()
            || next.len() < params.sip_min_inliers
            || round + 1 == MAX_CLIP_ROUNDS;
        if !stable {
            kept = next;
            continue;
        }
        if quadrants(&kept, size) < params.sip_min_quadrants {
            return None;
        }
        let considered = selection.considered;
        if selection.model.order <= 1 {
            return Some(SipSelection {
                considered,
                model: None,
            });
        }
        let swapped: Vec<(f64, f64, f64, f64)> =
            rows.iter().map(|&(x, y, tx, ty)| (tx, ty, x, y)).collect();
        let inverse = poly::fit(&swapped, selection.model.order);
        return Some(SipSelection {
            considered,
            model: inverse.map(|inverse| SipModel {
                forward: selection.model,
                inverse,
                kept,
            }),
        });
    }
    None
}

fn sip_cards(
    model: &PolyModel,
    considered: Vec<(u8, f64, f64)>,
    tan: &TanParams,
    size: (usize, usize),
) -> Result<DistortionFit, String> {
    let bbox = [[-0.5, size.0 as f64 - 0.5], [-0.5, size.1 as f64 - 0.5]];
    let options = SipFitOptions {
        degree: Some(vec![model.order]),
        crpix: Some([tan.crpix.0, tan.crpix.1]),
        ..SipFitOptions::default()
    };
    let sip = model
        .to_sip_cards(tan, bbox, &options)
        .map_err(|error| error.to_string())?;
    let sip_tan = TanParams {
        crval: (sip.crval[0], sip.crval[1]),
        crpix: (sip.crpix[0], sip.crpix[1]),
        cd: sip.cd,
    };
    Ok(DistortionFit {
        order: sip.a_order,
        considered,
        wcs_cards: sip.header_cards(),
        sip_max_err_px: Some(sip.max_err_px),
        tan: sip_tan,
    })
}

pub(in crate::core::astrometry::local_solve) struct Distortion {
    pub(in crate::core::astrometry::local_solve) fit: DistortionFit,
    pub(in crate::core::astrometry::local_solve) model: Option<PolyModel>,
    pub(in crate::core::astrometry::local_solve) warnings: Vec<String>,
}

pub(in crate::core::astrometry::local_solve) fn distortion_from(
    selection: Option<SipSelection>,
    tan: &TanParams,
    size: (usize, usize),
) -> Distortion {
    let plain = |considered: Vec<(u8, f64, f64)>| Distortion {
        fit: DistortionFit {
            considered,
            ..plain_tan(tan)
        },
        model: None,
        warnings: Vec::new(),
    };
    let Some(SipSelection { considered, model }) = selection else {
        return plain(Vec::new());
    };
    let Some(model) = model else {
        return plain(considered);
    };
    let populated = quadrants(&model.kept, size);
    let extrapolated = (populated < 4).then(|| {
        format!(
            "SIP fitted on {populated} of 4 image quadrants: the WCS is extrapolated in the quadrant without matched stars"
        )
    });
    match sip_cards(&model.forward, considered, tan, size) {
        Ok(fit) => Distortion {
            fit,
            model: Some(model.forward),
            warnings: extrapolated.into_iter().collect(),
        },
        Err(error) => Distortion {
            warnings: vec![format!("SIP fit skipped: {error}")],
            ..plain(Vec::new())
        },
    }
}

#[cfg(test)]
pub(in crate::core::astrometry::local_solve) fn fit_distortion(
    pairs: &[SkyPair],
    tan: &TanParams,
    size: (usize, usize),
    params: &LocalSolveParams,
) -> (DistortionFit, Vec<String>) {
    let distortion = distortion_from(fit_sip_model(pairs, tan, size, params), tan, size);
    (distortion.fit, distortion.warnings)
}
