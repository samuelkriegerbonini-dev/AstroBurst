use std::collections::HashMap;

use crate::core::astrometry::catalog::{propagate_epoch, CatalogRow};
use crate::core::registration::poly::{PolyModel, TanParams};

use crate::core::astrometry::local_solve::constants::{
    ARCSEC_PER_DEG, MIN_TAN_PAIRS, REFINE_PASSES, SIP_PASSES, SIP_PASS_RADIUS_GROWTH,
    SIP_PASS_MAX_RADIUS_FRACTION,
};
use crate::core::astrometry::local_solve::catalog_tiers::{RankingKey, CatalogProvider};
use crate::core::astrometry::local_solve::types::{
    Detection, SkyPair, Verification, DistortionFit, LocalSolveParams, CandidateNumbers,
};
use crate::core::astrometry::local_solve::tan_projection::{is_positive, tan_plane, sky_to_pixel, scale_arcsec};
use crate::core::astrometry::local_solve::tan_fit::{clipped_rms, fit_tan};
use crate::core::astrometry::local_solve::pairing::{match_radius, inside};
use crate::core::astrometry::local_solve::verification::{StageGate, verify_pairs, verify_with};
use crate::core::astrometry::local_solve::distortion::{
    SipModel, SipSelection, poly_pairs, fit_sip_model, Distortion, distortion_from,
};
use crate::core::astrometry::local_solve::triangle_match::{MatchSide, best_window_inliers, ransac_inlier_px};

pub(in crate::core::astrometry::local_solve) struct SearchContext<'a> {
    pub(in crate::core::astrometry::local_solve) detections: Vec<Detection>,
    pub(in crate::core::astrometry::local_solve) sip_detections: Vec<Detection>,
    pub(in crate::core::astrometry::local_solve) saturated: usize,
    pub(in crate::core::astrometry::local_solve) size: (usize, usize),
    pub(in crate::core::astrometry::local_solve) catalog: &'a dyn CatalogProvider,
    pub(in crate::core::astrometry::local_solve) ranking: RankingKey,
    pub(in crate::core::astrometry::local_solve) params: &'a LocalSolveParams,
    pub(in crate::core::astrometry::local_solve) epoch_year: Option<f64>,
    pub(in crate::core::astrometry::local_solve) scale_range: (f64, f64),
    pub(in crate::core::astrometry::local_solve) trials: usize,
}

pub(in crate::core::astrometry::local_solve) struct Hypothesis {
    scale_deg: f64,
    pub(in crate::core::astrometry::local_solve) spacing_deg: f64,
    cone_radius_deg: f64,
    bright_skip: usize,
    stage_rms_px: f64,
    pub(in crate::core::astrometry::local_solve) hinted: MatchSide,
    pub(in crate::core::astrometry::local_solve) spiral: Option<MatchSide>,
}

impl Hypothesis {
    pub(in crate::core::astrometry::local_solve) fn new(
        nominal_arcsec: f64,
        points: &[(f64, f64)],
        saturated: usize,
        size: (usize, usize),
        params: &LocalSolveParams,
    ) -> Self {
        let (width, height) = size;
        let scale_deg = nominal_arcsec / ARCSEC_PER_DEG;
        let field_w = width as f64 * scale_deg;
        let field_h = height as f64 * scale_deg;
        let long_side_deg = field_w.max(field_h);
        let inlier_px = ransac_inlier_px(width.max(height), long_side_deg);
        let cone_radius_deg = params.cone_short_side_fraction * field_w.min(field_h);
        let cone_area = std::f64::consts::PI * cone_radius_deg * cone_radius_deg;
        Self {
            scale_deg,
            spacing_deg: spiral_spacing_deg(scale_deg, field_w.min(field_h), cone_radius_deg, params),
            cone_radius_deg,
            bright_skip: (saturated as f64 * cone_area / (field_w * field_h)).round() as usize,
            stage_rms_px: params
                .max_rms_px
                .max(params.tan_rms_px_per_deg * long_side_deg),
            hinted: MatchSide::new(points, params.hinted_match_stars, inlier_px),
            spiral: None,
        }
    }

    pub(in crate::core::astrometry::local_solve) fn side(&self, ring: usize) -> &MatchSide {
        if ring == 0 {
            &self.hinted
        } else {
            self.spiral.as_ref().unwrap_or(&self.hinted)
        }
    }
}

pub(in crate::core::astrometry::local_solve) fn spiral_spacing_deg(
    scale_deg: f64,
    short_side_deg: f64,
    cone_radius_deg: f64,
    params: &LocalSolveParams,
) -> f64 {
    let coarse = params.spiral_spacing_fraction * short_side_deg;
    let cone_rad = cone_radius_deg.to_radians();
    let worst_offset_rad = params.spiral_projective_px * scale_deg.to_radians() / (cone_rad * cone_rad);
    let fine = (3f64.sqrt() * worst_offset_rad).to_degrees();
    if is_positive(fine) {
        coarse.min(fine)
    } else {
        coarse
    }
}

pub(in crate::core::astrometry::local_solve) struct Solved {
    pub(in crate::core::astrometry::local_solve) verification: Verification,
    pub(in crate::core::astrometry::local_solve) distortion: DistortionFit,
    pub(in crate::core::astrometry::local_solve) rms_px: f64,
    pub(in crate::core::astrometry::local_solve) warnings: Vec<String>,
}

pub(in crate::core::astrometry::local_solve) struct CentreOutcome {
    pub(in crate::core::astrometry::local_solve) candidate: Option<CandidateNumbers>,
    pub(in crate::core::astrometry::local_solve) solved: Option<Solved>,
}

pub(in crate::core::astrometry::local_solve) fn ranked_sky(
    rows: &[CatalogRow],
    skip: usize,
    limit: usize,
    ranking: RankingKey,
) -> Vec<(f64, f64)> {
    let mut ranked: Vec<(f64, usize)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| ranking.of(row).map(|key| (key, i)))
        .filter(|&(_, i)| rows[i].ra_epoch.is_finite() && rows[i].dec_epoch.is_finite())
        .collect();
    ranked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    ranked
        .iter()
        .skip(skip)
        .take(limit)
        .map(|&(_, i)| (rows[i].ra_epoch, rows[i].dec_epoch))
        .collect()
}

fn footprint_limit(detections: usize, params: &LocalSolveParams) -> usize {
    (params.footprint_stars_per_detection * detections).max(params.footprint_min_stars)
}

fn footprint_catalogue(tan: &TanParams, ctx: &SearchContext) -> Result<Vec<(f64, f64)>, String> {
    let (width, height) = ctx.size;
    let half_diagonal_deg =
        0.5 * (width as f64).hypot(height as f64) * scale_arcsec(&tan.cd) / ARCSEC_PER_DEG;
    let radius = ctx.params.footprint_margin * half_diagonal_deg;
    let mut rows = ctx.catalog.cone(tan.crval.0, tan.crval.1, radius)?;
    if let Some(epoch) = ctx.epoch_year {
        propagate_epoch(&mut rows, epoch);
    }
    let deepest = ctx.detections.len().max(ctx.sip_detections.len());
    Ok(ranked_sky(&rows, 0, usize::MAX, ctx.ranking)
        .into_iter()
        .filter(|&(ra, dec)| sky_to_pixel(tan, ra, dec).is_some_and(|p| inside(ctx.size, p)))
        .skip(ctx.saturated)
        .take(footprint_limit(deepest, ctx.params))
        .collect())
}

fn is_significant(verification: &Verification, params: &LocalSolveParams) -> bool {
    verification.matched >= params.min_matches
        && verification.false_alarm_log10 < params.max_false_alarm.log10()
        && verification.corners_finite
        && verification.scale_in_range
}

pub(in crate::core::astrometry::local_solve) fn refine_on(
    mut tan: TanParams,
    catalogue: &[(f64, f64)],
    ctx: &SearchContext,
    gate: StageGate,
) -> (TanParams, Verification, Vec<SkyPair>) {
    let (mut verification, mut pairs) = verify_pairs(
        &tan,
        &ctx.detections,
        catalogue,
        ctx.size,
        Some(ctx.scale_range),
        ctx.params,
        gate,
    );
    for _ in 0..REFINE_PASSES {
        if !is_significant(&verification, ctx.params) {
            break;
        }
        let Some(refined) = fit_tan(&pairs, ctx.size, tan.crval, ctx.params) else {
            break;
        };
        let (again, again_pairs) = verify_pairs(
            &refined.tan,
            &ctx.detections,
            catalogue,
            ctx.size,
            Some(ctx.scale_range),
            ctx.params,
            gate,
        );
        let worse = again.rms_px > verification.rms_px && again.matched <= verification.matched;
        if !is_significant(&again, ctx.params) || worse {
            break;
        }
        tan = refined.tan;
        verification = again;
        pairs = again_pairs;
    }
    (tan, verification, pairs)
}

fn model_rms_px(
    model: &PolyModel,
    pairs: &[SkyPair],
    tan: &TanParams,
    fallback: f64,
    params: &LocalSolveParams,
) -> f64 {
    let residuals = model.residuals(&poly_pairs(pairs, tan));
    if residuals.is_empty() {
        fallback
    } else {
        clipped_rms(&residuals, params.clip_sigma)
    }
}

struct Matched {
    verification: Verification,
    pairs: Vec<SkyPair>,
}

fn verify_through(
    model: &SipModel,
    radius: f64,
    tan: &TanParams,
    catalogue: &[(f64, f64)],
    ctx: &SearchContext,
    gate: StageGate,
) -> Matched {
    let predict =
        |ra: f64, dec: f64| sky_to_pixel(tan, ra, dec).map(|(tx, ty)| model.inverse.eval(tx, ty));
    let (verification, pairs) = verify_with(
        &predict,
        radius,
        tan,
        &ctx.sip_detections,
        catalogue,
        ctx.size,
        Some(ctx.scale_range),
        ctx.params,
        gate,
    );
    Matched {
        verification,
        pairs,
    }
}

fn sip_passes(
    tan: &TanParams,
    first: Matched,
    catalogue: &[(f64, f64)],
    ctx: &SearchContext,
    gate: StageGate,
) -> (Matched, Option<SipSelection>) {
    let Some(SipSelection { considered, model }) =
        fit_sip_model(&first.pairs, tan, ctx.size, ctx.params)
    else {
        return (first, None);
    };
    let Some(first_model) = model else {
        return (
            first,
            Some(SipSelection {
                considered,
                model: None,
            }),
        );
    };
    let base_radius = match_radius(&ctx.detections, ctx.params);
    let long_side_px = ctx.size.0.max(ctx.size.1) as f64;
    let max_radius = base_radius.max(SIP_PASS_MAX_RADIUS_FRACTION * long_side_px);
    let mut radius = base_radius;
    let mut current = first_model.clone();
    let mut grew = false;
    for _ in 0..SIP_PASSES {
        radius = (radius * SIP_PASS_RADIUS_GROWTH).min(max_radius);
        let matched = verify_through(&current, radius, tan, catalogue, ctx, gate);
        match fit_sip_model(&matched.pairs, tan, ctx.size, ctx.params).and_then(|s| s.model) {
            Some(model) if model.kept.len() > current.kept.len() => {
                current = model;
                grew = true;
            }
            _ => break,
        }
    }
    let first_selection = SipSelection {
        considered,
        model: Some(first_model),
    };
    if !grew {
        return (first, Some(first_selection));
    }
    let matched = verify_through(&current, base_radius, tan, catalogue, ctx, gate);
    match fit_sip_model(&matched.pairs, tan, ctx.size, ctx.params) {
        Some(selection) if matched.verification.accepted && selection.model.is_some() => {
            (matched, Some(selection))
        }
        _ => (first, Some(first_selection)),
    }
}

pub(in crate::core::astrometry::local_solve) fn try_centre(
    centre: (f64, f64),
    hypothesis: &Hypothesis,
    side: &MatchSide,
    ctx: &SearchContext,
) -> Result<CentreOutcome, String> {
    let mut rows = ctx
        .catalog
        .cone(centre.0, centre.1, hypothesis.cone_radius_deg)?;
    if let Some(epoch) = ctx.epoch_year {
        propagate_epoch(&mut rows, epoch);
    }
    let sky = ranked_sky(
        &rows,
        hypothesis.bright_skip,
        ctx.params.catalogue_stars,
        ctx.ranking,
    );
    let plane: Vec<((f64, f64), (f64, f64))> = sky
        .iter()
        .filter_map(|&(ra, dec)| {
            let (x, y) = tan_plane(centre, ra, dec)?;
            Some((
                (-x / hypothesis.scale_deg, y / hypothesis.scale_deg),
                (ra, dec),
            ))
        })
        .collect();
    let none = CentreOutcome {
        candidate: None,
        solved: None,
    };
    if plane.len() < MIN_TAN_PAIRS {
        return Ok(none);
    }
    let catalogue: Vec<(f64, f64)> = plane.iter().map(|(p, _)| *p).collect();
    let by_plane: HashMap<(u64, u64), (f64, f64)> = plane
        .iter()
        .map(|&((x, y), sky)| ((x.to_bits(), y.to_bits()), sky))
        .collect();
    let Some(inliers) = best_window_inliers(side, &catalogue) else {
        return Ok(none);
    };
    let sky_pairs: Vec<SkyPair> = inliers
        .iter()
        .filter_map(|&(rx, ry, tx, ty)| {
            let &(ra, dec) = by_plane.get(&(rx.to_bits(), ry.to_bits()))?;
            Some(SkyPair {
                x: tx,
                y: ty,
                ra,
                dec,
            })
        })
        .collect();
    let Some(fit) = fit_tan(&sky_pairs, ctx.size, centre, ctx.params) else {
        return Ok(none);
    };
    let gate = StageGate {
        max_rms_px: hypothesis.stage_rms_px,
        trials: ctx.trials,
    };
    let (coarse_tan, coarse, _) = refine_on(fit.tan, &sky, ctx, gate);
    let numbers = |tan: &TanParams, v: &Verification, rms_px: f64| CandidateNumbers {
        centre: tan.crval,
        matched: v.matched,
        predicted: v.predicted,
        rms_px,
        false_alarm_log10: v.false_alarm_log10,
        coverage_log10: v.coverage_log10,
        coverage_chance_log10: v.coverage_chance_log10,
    };
    if !coarse.accepted {
        return Ok(CentreOutcome {
            candidate: Some(numbers(&coarse_tan, &coarse, coarse.rms_px)),
            solved: None,
        });
    }
    let footprint = footprint_catalogue(&coarse_tan, ctx)?;
    let tan_footprint =
        &footprint[..footprint.len().min(footprint_limit(ctx.detections.len(), ctx.params))];
    let (tan, verification, pairs) = refine_on(coarse_tan, tan_footprint, ctx, gate);
    if !verification.accepted {
        return Ok(CentreOutcome {
            candidate: Some(numbers(&tan, &verification, verification.rms_px)),
            solved: None,
        });
    }
    let (matched, selection) = sip_passes(
        &tan,
        Matched {
            verification,
            pairs,
        },
        &footprint,
        ctx,
        gate,
    );
    let Matched {
        verification,
        pairs,
    } = matched;
    let Distortion {
        fit: distortion,
        model,
        warnings,
    } = distortion_from(selection, &tan, ctx.size);
    let rms_px = model.map_or(verification.rms_px, |model| {
        model_rms_px(&model, &pairs, &tan, verification.rms_px, ctx.params)
    });
    let candidate = numbers(&tan, &verification, rms_px);
    let covered = verification.covered();
    let solved = (rms_px <= ctx.params.max_rms_px && covered).then_some(Solved {
        verification,
        distortion,
        rms_px,
        warnings,
    });
    Ok(CentreOutcome {
        candidate: Some(candidate),
        solved,
    })
}
