// WCS engine migration to the CDS wcs-rs crate (full FITS projection coverage, wrapper-side SIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::{Map, Number, Value};
use wcs::{ImgXY, LonLat, WCSParams};

use crate::core::astrometry::frames::{convert_from_icrs, convert_to_icrs, SkyFrame};
pub use crate::core::astrometry::gwcs::GWCS_INVERSE_ACCEPT_PX;
use crate::core::astrometry::gwcs::{GwcsPipeline, GwcsSource, NewtonOptions};
use crate::types::header::{parse_fits_float, HduHeader};

const J2000_EQUINOX: f64 = 2000.0;
const EQUINOX_TOLERANCE_YEARS: f64 = 1e-3;
const FK5_FIRST_EQUINOX: f64 = 1984.0;

pub const GWCS_NEWTON_SKIP_MARGIN_PX: f64 = 8.0;
pub const GWCS_SIP_COMPARISON_GRID: usize = 9;
const GWCS_JACOBIAN_STEP_PX: f64 = 0.5;
const GWCS_PROJECTION: &str = "gWCS";
const MAS_PER_DEG: f64 = 3_600_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WcsKind {
    Header,
    Gwcs,
}

impl WcsKind {
    pub fn name(self) -> &'static str {
        match self {
            WcsKind::Header => "header",
            WcsKind::Gwcs => "gwcs",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GwcsInfo {
    pub n_steps: usize,
    pub frames: Vec<String>,
    pub source: String,
    pub vs_header_sip_max_mas: Option<f64>,
    pub vs_header_sip_max_px: Option<f64>,
}

/// SIP distortion polynomial coefficients.
///
/// SIP math is applied by this wrapper itself, NOT by the `wcs` crate: mapproj
/// 0.4.0's own `SipCoeff::p` evaluator has a confirmed bug (it advances powers of
/// `u`/`v` by repeated squaring -- `y *= y` -- instead of by the polynomial degree,
/// so it evaluates the wrong basis entirely for any nonzero-order SIP polynomial;
/// this was caught by cross-checking against astropy, where it produced wildly
/// wrong -- not merely imprecise -- sky coordinates for a SIP header away from the
/// reference pixel). This matches the wcs-rs README's own admission that SIP
/// support is "not tested". `WcsTransform` therefore always strips any `-SIP`
/// suffix before constructing the wcs-rs engine (so its internal SIP path never
/// engages) and applies/inverts SIP itself using this type, exactly as the
/// original hand-rolled implementation did.
#[derive(Debug, Clone, Default)]
pub struct SipPoly {
    terms: Vec<(i32, i32, f64)>,
}

impl SipPoly {
    fn parse(header: &HduHeader, prefix: &str) -> Option<SipPoly> {
        let order = header.get_f64(&format!("{}_ORDER", prefix))?;
        if !order.is_finite() || !(0.0..=9.0).contains(&order) {
            return None;
        }
        let order = order as i32;
        let mut terms = Vec::new();
        for p in 0..=order {
            for q in 0..=(order - p) {
                if let Some(c) = header.get_f64(&format!("{}_{}_{}", prefix, p, q)) {
                    if c.is_finite() && c != 0.0 {
                        terms.push((p, q, c));
                    }
                }
            }
        }
        if terms.is_empty() {
            None
        } else {
            Some(SipPoly { terms })
        }
    }

    #[inline]
    fn eval(&self, u: f64, v: f64) -> f64 {
        let mut sum = 0.0;
        for &(p, q, c) in &self.terms {
            sum += c * u.powi(p) * v.powi(q);
        }
        sum
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CelestialCoord {
    pub ra: f64,
    pub dec: f64,
}

impl std::fmt::Display for CelestialCoord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ra_h = self.ra / 15.0;
        let h = ra_h.floor() as u32;
        let m = ((ra_h - h as f64) * 60.0).floor() as u32;
        let s = (ra_h - h as f64) * 3600.0 - m as f64 * 60.0;

        let dec_sign = if self.dec >= 0.0 { "+" } else { "-" };
        let dec_abs = self.dec.abs();
        let d = dec_abs.floor() as u32;
        let dm = ((dec_abs - d as f64) * 60.0).floor() as u32;
        let ds = (dec_abs - d as f64) * 3600.0 - dm as f64 * 60.0;

        write!(
            f,
            "{:02}h{:02}m{:05.2}s {}{}°{:02}'{:05.2}\"",
            h, m, s, dec_sign, d, dm, ds
        )
    }
}

pub const SIP_RESIDUAL_KEYS: [&str; 2] = ["SIPMXERR", "SIPIVERR"];

#[derive(Debug)]
struct HeaderBackend {
    engine: wcs::WCS,
    sip_a: Option<SipPoly>,
    sip_b: Option<SipPoly>,
    sip_ap: Option<SipPoly>,
    sip_bp: Option<SipPoly>,
}

#[derive(Debug)]
enum Backend {
    Header(HeaderBackend),
    Gwcs(Arc<GwcsSource>),
}

#[derive(Debug)]
pub struct WcsTransform {
    backend: Backend,
    crpix1: f64,
    crpix2: f64,
    crval1: f64,
    crval2: f64,
    cd: [[f64; 2]; 2],
    cd_inv: [[f64; 2]; 2],
    frame: SkyFrame,
    projection: String,
    sip_max_err_px: Option<f64>,
    sip_inv_err_px: Option<f64>,
    gwcs_info: Option<GwcsInfo>,
    gwcs_refusal: Option<String>,
}

fn is_projection_param_key(key: &str) -> bool {
    matches!(key, "LONPOLE" | "LATPOLE")
        || key
            .strip_prefix("PV1_")
            .or_else(|| key.strip_prefix("PV2_"))
            .is_some_and(|m| m.parse::<u32>().is_ok())
}

fn insert_numeric_card(map: &mut Map<String, Value>, key: &str, raw: &str) {
    let trimmed = raw.trim().trim_matches('\'').trim();
    if let Some(n) = parse_fits_float(trimmed).and_then(Number::from_f64) {
        map.insert(key.to_string(), Value::Number(n));
    }
}

fn number(value: f64) -> Result<Value> {
    Number::from_f64(value)
        .map(Value::Number)
        .context("Non-finite WCS parameter")
}

pub fn read_effective_cd(header: &HduHeader) -> Result<[[f64; 2]; 2]> {
    let cd11 = header.get_f64("CD1_1");
    let cd12 = header.get_f64("CD1_2");
    let cd21 = header.get_f64("CD2_1");
    let cd22 = header.get_f64("CD2_2");
    if cd11.is_some() || cd12.is_some() || cd21.is_some() || cd22.is_some() {
        return Ok([
            [cd11.unwrap_or(1.0), cd12.unwrap_or(0.0)],
            [cd21.unwrap_or(0.0), cd22.unwrap_or(1.0)],
        ]);
    }

    let pc11 = header.get_f64("PC1_1");
    let pc12 = header.get_f64("PC1_2");
    let pc21 = header.get_f64("PC2_1");
    let pc22 = header.get_f64("PC2_2");
    if pc11.is_some() || pc12.is_some() || pc21.is_some() || pc22.is_some() {
        let cdelt1 = header.get_f64("CDELT1").unwrap_or(1.0);
        let cdelt2 = header.get_f64("CDELT2").unwrap_or(1.0);
        return Ok([
            [cdelt1 * pc11.unwrap_or(1.0), cdelt1 * pc12.unwrap_or(0.0)],
            [cdelt2 * pc21.unwrap_or(0.0), cdelt2 * pc22.unwrap_or(1.0)],
        ]);
    }

    let cdelt1 = header
        .get_f64("CDELT1")
        .context("Missing CD matrix, PC matrix, and CDELT1")?;
    let cdelt2 = header
        .get_f64("CDELT2")
        .context("Missing CD matrix, PC matrix, and CDELT2")?;
    let crota2 = header.get_f64("CROTA2").unwrap_or(0.0);
    let (sin_t, cos_t) = crota2.to_radians().sin_cos();

    Ok([
        [cdelt1 * cos_t, -cdelt2 * sin_t],
        [cdelt1 * sin_t, cdelt2 * cos_t],
    ])
}

fn invert_cd(cd: &[[f64; 2]; 2]) -> Result<[[f64; 2]; 2]> {
    let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
    if !det.is_finite() || det.abs() < 1e-30 {
        bail!("Singular or non-finite CD matrix");
    }
    Ok([
        [cd[1][1] / det, -cd[0][1] / det],
        [-cd[1][0] / det, cd[0][0] / det],
    ])
}

#[inline]
fn apply_linear(m: &[[f64; 2]; 2], a: f64, b: f64) -> (f64, f64) {
    (m[0][0] * a + m[0][1] * b, m[1][0] * a + m[1][1] * b)
}

fn header_string(header: &HduHeader, key: &str) -> Option<String> {
    header
        .get(key)
        .map(|v| v.trim().trim_matches('\'').trim().to_ascii_uppercase())
        .filter(|v| !v.is_empty())
}

fn is_j2000(equinox: f64) -> bool {
    (equinox - J2000_EQUINOX).abs() <= EQUINOX_TOLERANCE_YEARS
}

fn equatorial_frame(header: &HduHeader) -> Result<SkyFrame> {
    let equinox = header.get_f64("EQUINOX").filter(|v| v.is_finite());
    let radesys = header_string(header, "RADESYS").or_else(|| header_string(header, "RADECSYS"));
    match radesys.as_deref() {
        Some("ICRS") => Ok(SkyFrame::Icrs),
        Some("FK5") => match equinox {
            Some(eq) if !is_j2000(eq) => {
                bail!("FK5 coordinates of equinox {eq} are not supported; only J2000 is")
            }
            _ => Ok(SkyFrame::Icrs),
        },
        Some(other) => bail!(
            "RADESYS '{other}' is not supported; only ICRS and FK5 J2000 celestial coordinates are"
        ),
        None => {
            let legacy = header
                .get_f64("EPOCH")
                .filter(|v| v.is_finite() && *v < FK5_FIRST_EQUINOX);
            match equinox.or(legacy) {
                Some(eq) if eq < FK5_FIRST_EQUINOX => bail!(
                    "equinox {eq} without RADESYS means FK4 (B1950-style) coordinates, which are not supported"
                ),
                Some(eq) if !is_j2000(eq) => {
                    bail!("FK5 coordinates of equinox {eq} are not supported; only J2000 is")
                }
                _ => Ok(SkyFrame::Icrs),
            }
        }
    }
}

fn celestial_frame(header: &HduHeader, axis_type: &str) -> Result<SkyFrame> {
    match axis_type {
        "RA" => equatorial_frame(header),
        "GLON" => Ok(SkyFrame::Galactic),
        "ELON" => match header.get_f64("EQUINOX").filter(|v| v.is_finite()) {
            Some(eq) if !is_j2000(eq) => {
                bail!("ecliptic coordinates of equinox {eq} are not supported; only J2000 is")
            }
            _ => Ok(SkyFrame::EclipticJ2000),
        },
        other => bail!(
            "CTYPE1 axis '{other}' is not a supported celestial longitude (RA, GLON or ELON)"
        ),
    }
}

const CELESTIAL_AXIS_PAIRS: [(&str, &str); 3] = [("RA", "DEC"), ("GLON", "GLAT"), ("ELON", "ELAT")];

fn ctype_card<'a>(header: &'a HduHeader, key: &str) -> Option<&'a str> {
    header
        .get(key)
        .map(|v| v.trim().trim_matches('\'').trim())
        .filter(|v| !v.is_empty())
}

fn axis_code(ctype: &str) -> String {
    ctype[..ctype.len().min(4)].trim_end_matches('-').to_ascii_uppercase()
}

fn malformed_ctype(axis: &str, raw: &str) -> anyhow::Error {
    anyhow::anyhow!("{axis} '{raw}' is not an 8-character ASCII FITS celestial axis type")
}

fn celestial_axis_pair(header: &HduHeader) -> Result<(String, String, SkyFrame)> {
    let (Some(ctype1_raw), Some(ctype2_raw)) = (ctype_card(header, "CTYPE1"), ctype_card(header, "CTYPE2")) else {
        bail!("CTYPE1/CTYPE2 missing: the header has no celestial axis types (FITS treats a missing CTYPE as a linear axis), so no sky WCS is read");
    };
    let ctype1 = ctype1_raw.trim_end_matches("-SIP");
    let ctype2 = ctype2_raw.trim_end_matches("-SIP");
    if !ctype1.is_ascii() || ctype1.len() < 8 {
        return Err(malformed_ctype("CTYPE1", ctype1_raw));
    }
    if !ctype2.is_ascii() {
        return Err(malformed_ctype("CTYPE2", ctype2_raw));
    }
    let axis1 = axis_code(ctype1);
    let axis2 = axis_code(ctype2);
    if CELESTIAL_AXIS_PAIRS.iter().any(|(_, lat)| *lat == axis1) {
        bail!("CTYPE1 '{ctype1_raw}' puts latitude on axis 1; only longitude-first celestial headers are supported");
    }
    let frame = celestial_frame(header, &axis1)?;
    if !CELESTIAL_AXIS_PAIRS.iter().any(|(lon, lat)| *lon == axis1 && *lat == axis2) {
        bail!("CTYPE1 '{ctype1_raw}' and CTYPE2 '{ctype2_raw}' are not a celestial longitude/latitude pair");
    }
    if ctype2.len() < 8 {
        return Err(malformed_ctype("CTYPE2", ctype2_raw));
    }
    let projection = ctype1[5..8].to_ascii_uppercase();
    let projection2 = ctype2[5..8].to_ascii_uppercase();
    if projection != projection2 {
        bail!("CTYPE1 projection '{projection}' and CTYPE2 projection '{projection2}' differ");
    }
    Ok((axis1, projection, frame))
}

fn sip_residual_card(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|v| v.is_finite() && *v >= 0.0)
}

pub fn pixel_center(naxis1: usize, naxis2: usize) -> (f64, f64) {
    ((naxis1 as f64 - 1.0) / 2.0, (naxis2 as f64 - 1.0) / 2.0)
}

pub fn pixel_edge_corners(naxis1: usize, naxis2: usize) -> [(f64, f64); 4] {
    let right = naxis1 as f64 - 0.5;
    let top = naxis2 as f64 - 0.5;
    [(-0.5, -0.5), (right, -0.5), (right, top), (-0.5, top)]
}

impl WcsTransform {
    pub fn from_header(header: &HduHeader) -> Result<Self> {
        let crpix1 = header.get_f64("CRPIX1").context("Missing CRPIX1")?;
        let crpix2 = header.get_f64("CRPIX2").context("Missing CRPIX2")?;
        let crval1 = header.get_f64("CRVAL1").context("Missing CRVAL1")?;
        let crval2 = header.get_f64("CRVAL2").context("Missing CRVAL2")?;
        if !crpix1.is_finite() || !crpix2.is_finite() || !crval1.is_finite() || !crval2.is_finite() {
            bail!("Non-finite CRPIX/CRVAL values");
        }

        let cd = read_effective_cd(header)?;
        let cd_inv = invert_cd(&cd)?;

        let (_, projection, frame) = celestial_axis_pair(header)?;

        let naxis1 = header.get_i64("NAXIS1").context("Missing NAXIS1")?;
        let naxis2 = header.get_i64("NAXIS2").context("Missing NAXIS2")?;

        let mut map: Map<String, Value> = Map::new();
        for (key, val) in &header.cards {
            let ku = key.to_uppercase();
            if is_projection_param_key(&ku) {
                insert_numeric_card(&mut map, &ku, val);
            }
        }
        map.insert("NAXIS".into(), Value::Number(2.into()));
        map.insert("NAXIS1".into(), Value::Number(naxis1.into()));
        map.insert("NAXIS2".into(), Value::Number(naxis2.into()));
        map.insert("CTYPE1".into(), Value::String(format!("RA---{projection}")));
        map.insert("CTYPE2".into(), Value::String(format!("DEC--{projection}")));
        map.insert("CRVAL1".into(), number(crval1)?);
        map.insert("CRVAL2".into(), number(crval2)?);
        map.insert("CRPIX1".into(), number(0.0)?);
        map.insert("CRPIX2".into(), number(0.0)?);
        map.insert("CD1_1".into(), number(1.0)?);
        map.insert("CD1_2".into(), number(0.0)?);
        map.insert("CD2_1".into(), number(0.0)?);
        map.insert("CD2_2".into(), number(1.0)?);

        let params: WCSParams = serde_json::from_value(Value::Object(map))
            .context("Building WCSParams from header")?;
        let engine =
            wcs::WCS::new(&params).map_err(|e| anyhow::anyhow!("wcs::WCS::new failed: {e}"))?;

        Ok(WcsTransform {
            backend: Backend::Header(HeaderBackend {
                engine,
                sip_a: SipPoly::parse(header, "A"),
                sip_b: SipPoly::parse(header, "B"),
                sip_ap: SipPoly::parse(header, "AP"),
                sip_bp: SipPoly::parse(header, "BP"),
            }),
            crpix1,
            crpix2,
            crval1,
            crval2,
            cd,
            cd_inv,
            frame,
            projection,
            sip_max_err_px: sip_residual_card(header, SIP_RESIDUAL_KEYS[0]),
            sip_inv_err_px: sip_residual_card(header, SIP_RESIDUAL_KEYS[1]),
            gwcs_info: None,
            gwcs_refusal: None,
        })
    }

    pub fn from_gwcs(source: Arc<GwcsSource>, header: &HduHeader) -> Result<Self> {
        let pipeline = &source.pipeline;
        let (cx, cy) = match pipeline.bbox_centre() {
            Some([cx, cy]) => (cx, cy),
            None => header_dims(header).map_or((0.0, 0.0), |(n1, n2)| pixel_center(n1, n2)),
        };
        let centre = pipeline.forward(cx, cy, false);
        if !(centre[0].is_finite() && centre[1].is_finite()) {
            bail!("gWCS evaluates to a non-finite sky position at the reference pixel ({cx}, {cy})");
        }
        let cd = pipeline.local_jacobian_deg_per_px(cx, cy, GWCS_JACOBIAN_STEP_PX);
        let cd_inv = invert_cd(&cd)?;
        let crval = gwcs_coord(centre);
        let header_sip = header_has_sip(header).then(|| Self::from_header(header).ok()).flatten();
        let mut wcs = WcsTransform {
            backend: Backend::Gwcs(Arc::clone(&source)),
            crpix1: cx + 1.0,
            crpix2: cy + 1.0,
            crval1: crval.ra,
            crval2: crval.dec,
            cd,
            cd_inv,
            frame: SkyFrame::Icrs,
            projection: GWCS_PROJECTION.to_string(),
            sip_max_err_px: source.wcsinfo_sip_max_err_px.or_else(|| sip_residual_card(header, SIP_RESIDUAL_KEYS[0])),
            sip_inv_err_px: source.wcsinfo_sip_inv_err_px.or_else(|| sip_residual_card(header, SIP_RESIDUAL_KEYS[1])),
            gwcs_info: None,
            gwcs_refusal: None,
        };
        let vs_header_sip_max_mas = header_sip.and_then(|sip| wcs.separation_from_header_sip_mas(&sip, header));
        let vs_header_sip_max_px = vs_header_sip_max_mas.map(|mas| mas / (wcs.pixel_scale_arcsec() * 1000.0));
        wcs.gwcs_info = Some(GwcsInfo {
            n_steps: pipeline.n_steps(),
            frames: pipeline.frame_names(),
            source: source.origin.describe(),
            vs_header_sip_max_mas,
            vs_header_sip_max_px,
        });
        Ok(wcs)
    }

    fn separation_from_header_sip_mas(&self, header_sip: &WcsTransform, header: &HduHeader) -> Option<f64> {
        let pipeline = self.gwcs_pipeline()?;
        let dims = header_dims(header);
        let mut box_axes = [[f64::NAN; 2]; 2];
        for axis in 0..2 {
            let from_bbox = pipeline.bounding_box.filter(|b| !b.ignore[axis]).map(|b| b.intervals[axis]);
            let from_dims = dims.map(|(n1, n2)| [-0.5, if axis == 0 { n1 } else { n2 } as f64 - 0.5]);
            box_axes[axis] = from_bbox.or(from_dims)?;
        }
        let steps = (GWCS_SIP_COMPARISON_GRID - 1) as f64;
        let coordinate = |axis: usize, k: usize| box_axes[axis][0] + (box_axes[axis][1] - box_axes[axis][0]) * k as f64 / steps;
        let mut max_mas = f64::NEG_INFINITY;
        for j in 0..GWCS_SIP_COMPARISON_GRID {
            for i in 0..GWCS_SIP_COMPARISON_GRID {
                let (x, y) = (coordinate(0, i), coordinate(1, j));
                let g = gwcs_coord(pipeline.forward(x, y, false));
                let h = header_sip.pixel_to_world(x, y);
                let sep = angular_separation(g.ra, g.dec, h.ra, h.dec) * MAS_PER_DEG;
                if sep.is_finite() {
                    max_mas = max_mas.max(sep);
                }
            }
        }
        max_mas.is_finite().then_some(max_mas)
    }

    pub fn with_gwcs_refusal(mut self, reason: String) -> Self {
        self.gwcs_refusal = Some(reason);
        self
    }

    pub fn kind(&self) -> WcsKind {
        match self.backend {
            Backend::Header(_) => WcsKind::Header,
            Backend::Gwcs(_) => WcsKind::Gwcs,
        }
    }

    pub fn gwcs_info(&self) -> Option<&GwcsInfo> {
        self.gwcs_info.as_ref()
    }

    pub fn gwcs_refusal(&self) -> Option<&str> {
        self.gwcs_refusal.as_deref()
    }

    pub fn gwcs_pipeline(&self) -> Option<&GwcsPipeline> {
        match &self.backend {
            Backend::Gwcs(source) => Some(&source.pipeline),
            Backend::Header(_) => None,
        }
    }

    pub fn sip_fit_residuals(&self) -> (Option<f64>, Option<f64>) {
        (self.sip_max_err_px, self.sip_inv_err_px)
    }

    pub fn sip_forward_terms(&self) -> (Option<&SipPoly>, Option<&SipPoly>) {
        match &self.backend {
            Backend::Header(h) => (h.sip_a.as_ref(), h.sip_b.as_ref()),
            Backend::Gwcs(_) => (None, None),
        }
    }
}

impl HeaderBackend {
    /// Applies forward SIP distortion: `(u, v) = (dx + A(dx,dy), dy + B(dx,dy))`.
    /// A no-op if no SIP coefficients were present in the header.
    #[inline]
    fn sip_forward(&self, dx: f64, dy: f64) -> (f64, f64) {
        if self.sip_a.is_none() && self.sip_b.is_none() {
            return (dx, dy);
        }
        (
            dx + self.sip_a.as_ref().map_or(0.0, |p| p.eval(dx, dy)),
            dy + self.sip_b.as_ref().map_or(0.0, |p| p.eval(dx, dy)),
        )
    }

    /// Inverts forward SIP distortion: analytically via AP/BP if present, else by
    /// a 12-iteration fixed-point (Newton-like) iteration -- identical to the
    /// original hand-rolled implementation this replaces.
    fn sip_inverse(&self, u_lin: f64, v_lin: f64) -> (f64, f64) {
        if self.sip_a.is_none() && self.sip_b.is_none() {
            return (u_lin, v_lin);
        }
        if self.sip_ap.is_some() || self.sip_bp.is_some() {
            return (
                u_lin + self.sip_ap.as_ref().map_or(0.0, |p| p.eval(u_lin, v_lin)),
                v_lin + self.sip_bp.as_ref().map_or(0.0, |p| p.eval(u_lin, v_lin)),
            );
        }
        let mut u = u_lin;
        let mut v = v_lin;
        for _ in 0..12 {
            let nu = u_lin - self.sip_a.as_ref().map_or(0.0, |p| p.eval(u, v));
            let nv = v_lin - self.sip_b.as_ref().map_or(0.0, |p| p.eval(u, v));
            if (nu - u).abs() < 1e-10 && (nv - v).abs() < 1e-10 {
                return (nu, nv);
            }
            u = nu;
            v = nv;
        }
        (u, v)
    }
}

impl WcsTransform {

    pub fn raw_params(&self) -> (f64, f64, f64, f64, [[f64; 2]; 2], &str) {
        (
            self.crpix1,
            self.crpix2,
            self.crval1,
            self.crval2,
            self.cd,
            self.projection.as_str(),
        )
    }

    pub fn pixel_to_world(&self, x: f64, y: f64) -> CelestialCoord {
        let header = match &self.backend {
            Backend::Header(h) => h,
            Backend::Gwcs(source) => return gwcs_coord(source.pipeline.forward(x, y, false)),
        };
        let dx = x - self.crpix1 + 1.0;
        let dy = y - self.crpix2 + 1.0;
        let (u, v) = header.sip_forward(dx, dy);
        let (ix, iy) = apply_linear(&self.cd, u, v);
        if !(ix * ix + iy * iy).is_finite() {
            return CelestialCoord {
                ra: f64::NAN,
                dec: f64::NAN,
            };
        }

        match header.engine.unproj(&ImgXY::new(ix, iy)) {
            Some(ll) => {
                let (ra, dec) = convert_to_icrs(self.frame, ll.lon().to_degrees(), ll.lat().to_degrees());
                CelestialCoord { ra, dec }
            }
            None => CelestialCoord {
                ra: f64::NAN,
                dec: f64::NAN,
            },
        }
    }

    pub fn pixel_to_world_bounded(&self, x: f64, y: f64) -> CelestialCoord {
        match &self.backend {
            Backend::Gwcs(source) => gwcs_coord(source.pipeline.forward(x, y, true)),
            Backend::Header(_) => self.pixel_to_world(x, y),
        }
    }

    pub fn world_to_pixel(&self, ra: f64, dec: f64) -> (f64, f64) {
        if !ra.is_finite() || !dec.is_finite() {
            return (f64::NAN, f64::NAN);
        }
        let header = match &self.backend {
            Backend::Header(h) => h,
            Backend::Gwcs(source) => return gwcs_world_to_pixel(&source.pipeline, ra, dec),
        };
        let (lon, lat) = match self.frame {
            SkyFrame::Icrs => (ra, dec),
            frame => convert_from_icrs(frame, ra, dec),
        };
        match header.engine.proj(&LonLat::new(lon.to_radians(), lat.to_radians())) {
            Some(xy) => {
                let (u_lin, v_lin) = apply_linear(&self.cd_inv, xy.x(), xy.y());
                let (dx, dy) = header.sip_inverse(u_lin, v_lin);
                (dx + self.crpix1 - 1.0, dy + self.crpix2 - 1.0)
            }
            None => (f64::NAN, f64::NAN),
        }
    }

    pub fn pixel_scale_arcsec(&self) -> f64 {
        let scale_x = (self.cd[0][0].powi(2) + self.cd[1][0].powi(2)).sqrt();
        let scale_y = (self.cd[0][1].powi(2) + self.cd[1][1].powi(2)).sqrt();
        ((scale_x + scale_y) / 2.0) * 3600.0
    }

    pub fn field_of_view(&self, naxis1: usize, naxis2: usize) -> (f64, f64) {
        let scale_x = (self.cd[0][0].powi(2) + self.cd[1][0].powi(2)).sqrt();
        let scale_y = (self.cd[0][1].powi(2) + self.cd[1][1].powi(2)).sqrt();
        (naxis1 as f64 * scale_x * 60.0, naxis2 as f64 * scale_y * 60.0)
    }

    pub fn pixel_to_world_batch(&self, coords: &[(f64, f64)]) -> Vec<CelestialCoord> {
        if coords.len() > 1024 {
            coords
                .par_iter()
                .map(|&(x, y)| self.pixel_to_world(x, y))
                .collect()
        } else {
            coords
                .iter()
                .map(|&(x, y)| self.pixel_to_world(x, y))
                .collect()
        }
    }

    pub fn pixel_to_world_bounded_batch(&self, coords: &[(f64, f64)]) -> Vec<CelestialCoord> {
        if coords.len() > 1024 {
            coords
                .par_iter()
                .map(|&(x, y)| self.pixel_to_world_bounded(x, y))
                .collect()
        } else {
            coords
                .iter()
                .map(|&(x, y)| self.pixel_to_world_bounded(x, y))
                .collect()
        }
    }

    pub fn world_to_pixel_batch(&self, coords: &[(f64, f64)]) -> Vec<(f64, f64)> {
        if coords.len() > 1024 {
            coords
                .par_iter()
                .map(|&(ra, dec)| self.world_to_pixel(ra, dec))
                .collect()
        } else {
            coords
                .iter()
                .map(|&(ra, dec)| self.world_to_pixel(ra, dec))
                .collect()
        }
    }
}

fn gwcs_coord(world: [f64; 2]) -> CelestialCoord {
    CelestialCoord { ra: if world[0] == 360.0 { 0.0 } else { world[0] }, dec: world[1] }
}

fn header_dims(header: &HduHeader) -> Option<(usize, usize)> {
    let dim = |key: &str| header.get_i64(key).filter(|n| *n > 0).and_then(|n| usize::try_from(n).ok());
    Some((dim("NAXIS1")?, dim("NAXIS2")?))
}

pub(crate) fn header_has_sip(header: &HduHeader) -> bool {
    ctype_card(header, "CTYPE1").is_some_and(|c| c.to_ascii_uppercase().ends_with("-SIP")) || header.get("A_ORDER").is_some()
}

fn start_is_beyond_the_margin(pipeline: &GwcsPipeline, start: [f64; 2]) -> bool {
    let Some(bbox) = pipeline.bounding_box else { return false };
    (0..2).any(|axis| {
        !bbox.ignore[axis]
            && (start[axis] < bbox.intervals[axis][0] - GWCS_NEWTON_SKIP_MARGIN_PX
                || start[axis] > bbox.intervals[axis][1] + GWCS_NEWTON_SKIP_MARGIN_PX)
    })
}

fn gwcs_world_to_pixel(pipeline: &GwcsPipeline, ra: f64, dec: f64) -> (f64, f64) {
    if let Ok(start) = pipeline.backward_analytic(ra, dec) {
        if start[0].is_finite() && start[1].is_finite() && start_is_beyond_the_margin(pipeline, start) {
            return (start[0], start[1]);
        }
    }
    let r = pipeline.backward_exact(ra, dec, &NewtonOptions::default());
    (r.pixel[0], r.pixel[1])
}

pub fn angular_separation(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let ra1 = ra1.to_radians();
    let dec1 = dec1.to_radians();
    let ra2 = ra2.to_radians();
    let dec2 = dec2.to_radians();
    let d_ra = ra2 - ra1;
    let d_dec = dec2 - dec1;
    let a =
        (d_dec / 2.0).sin().powi(2) + dec1.cos() * dec2.cos() * (d_ra / 2.0).sin().powi(2);
    (2.0 * a.sqrt().clamp(-1.0, 1.0).asin()).to_degrees()
}

pub fn position_angle_deg(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let d_ra = (ra2 - ra1).to_radians();
    let dec1 = dec1.to_radians();
    let dec2 = dec2.to_radians();
    let y = d_ra.sin() * dec2.cos();
    let x = dec1.cos() * dec2.sin() - dec1.sin() * dec2.cos() * d_ra.cos();
    y.atan2(x).to_degrees().rem_euclid(360.0)
}

const ORIENTATION_STEP_PIXELS: f64 = 10.0;
const ORIENTATION_MAX_FOV_FRACTION: f64 = 0.25;
const MIN_COS_DEC: f64 = 1e-9;

#[derive(Debug, Clone, Serialize)]
pub struct WcsOrientation {
    pub rotation_deg: f64,
    pub flipped: bool,
    pub pixel_scale_x_arcsec: f64,
    pub pixel_scale_y_arcsec: f64,
    pub projection: String,
    pub sip_present: bool,
    pub sip_max_err_px: Option<f64>,
    pub sip_inv_err_px: Option<f64>,
    pub north_vec: Option<(f64, f64)>,
    pub east_vec: Option<(f64, f64)>,
    pub wcs_kind: WcsKind,
    pub gwcs: Option<GwcsInfo>,
}

fn unit_vector_from(origin: (f64, f64), point: (f64, f64), sign: f64) -> Option<(f64, f64)> {
    let dx = (point.0 - origin.0) * sign;
    let dy = (point.1 - origin.1) * sign;
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= 0.0 {
        return None;
    }
    Some((dx / len, dy / len))
}

impl WcsTransform {
    pub fn orientation(&self, naxis1: usize, naxis2: usize) -> WcsOrientation {
        let cd = self.cd;
        let (cd11, cd12, cd21, cd22) = (cd[0][0], cd[0][1], cd[1][0], cd[1][1]);
        let pixel_scale_x_arcsec = (cd11 * cd11 + cd21 * cd21).sqrt() * 3600.0;
        let pixel_scale_y_arcsec = (cd12 * cd12 + cd22 * cd22).sqrt() * 3600.0;
        let rotation_deg = (-cd12).atan2(cd22).to_degrees();
        let flipped = cd11 * cd22 - cd12 * cd21 > 0.0;
        let (sip_a, sip_b) = self.sip_forward_terms();
        let sip_present = sip_a.is_some() || sip_b.is_some();

        let (north_vec, east_vec) = self.cardinal_vectors(naxis1, naxis2);

        WcsOrientation {
            rotation_deg,
            flipped,
            pixel_scale_x_arcsec,
            pixel_scale_y_arcsec,
            projection: self.projection.clone(),
            sip_present,
            sip_max_err_px: self.sip_max_err_px,
            sip_inv_err_px: self.sip_inv_err_px,
            north_vec,
            east_vec,
            wcs_kind: self.kind(),
            gwcs: self.gwcs_info.clone(),
        }
    }

    fn cardinal_vectors(&self, naxis1: usize, naxis2: usize) -> (Option<(f64, f64)>, Option<(f64, f64)>) {
        let has_dims = naxis1 > 0 && naxis2 > 0;
        let centre = if has_dims {
            pixel_center(naxis1, naxis2)
        } else {
            (self.crpix1 - 1.0, self.crpix2 - 1.0)
        };
        let sky0 = self.pixel_to_world(centre.0, centre.1);
        if !sky0.ra.is_finite() || !sky0.dec.is_finite() {
            return (None, None);
        }
        let mut step_deg = ORIENTATION_STEP_PIXELS * self.pixel_scale_arcsec() / 3600.0;
        if has_dims {
            let (fov_w, fov_h) = self.field_of_view(naxis1, naxis2);
            let quarter_fov_deg = fov_w.min(fov_h) / 60.0 * ORIENTATION_MAX_FOV_FRACTION;
            step_deg = step_deg.min(quarter_fov_deg);
        }
        if !step_deg.is_finite() || step_deg <= 0.0 {
            return (None, None);
        }

        let (north_dec, north_sign) = if sky0.dec + step_deg > 90.0 {
            (sky0.dec - step_deg, -1.0)
        } else {
            (sky0.dec + step_deg, 1.0)
        };
        let north_vec = unit_vector_from(centre, self.world_to_pixel(sky0.ra, north_dec), north_sign);

        let (sin_dec, cos_dec) = sky0.dec.to_radians().sin_cos();
        let east_vec = if cos_dec.abs() < MIN_COS_DEC {
            None
        } else {
            let (sin_step, cos_step) = step_deg.to_radians().sin_cos();
            let east_ra = sky0.ra + sin_step.atan2(cos_dec * cos_step).to_degrees();
            let east_dec = (sin_dec * cos_step).atan2((cos_dec * cos_step).hypot(sin_step)).to_degrees();
            unit_vector_from(centre, self.world_to_pixel(east_ra, east_dec), 1.0)
        };
        (north_vec, east_vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut index = HashMap::new();
        let mut cards = Vec::new();
        for &(k, v) in pairs {
            index.insert(k.to_string(), v.to_string());
            cards.push((k.to_string(), v.to_string()));
        }
        HduHeader { cards, index, string_keys: None }
    }

    #[test]
    fn test_identity_tan() {
        // Also serves as the empirical probe for wcs-rs's LonLat radian/degree
        // convention: mapproj's own doc comment says radians, and this assertion
        // (pixel_to_world returning CRVAL exactly) is the guardrail confirming it.
        let h = make_header(&[
            ("NAXIS1", "1024"),
            ("NAXIS2", "1024"),
            ("CRPIX1", "512"),
            ("CRPIX2", "512"),
            ("CRVAL1", "180.0"),
            ("CRVAL2", "45.0"),
            ("CDELT1", "-0.001"),
            ("CDELT2", "0.001"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);

        let wcs = WcsTransform::from_header(&h).unwrap();
        let coord = wcs.pixel_to_world(511.0, 511.0);
        assert!((coord.ra - 180.0).abs() < 1e-6, "ra={}", coord.ra);
        assert!((coord.dec - 45.0).abs() < 1e-6, "dec={}", coord.dec);
    }

    #[test]
    fn test_roundtrip_tan() {
        let h = make_header(&[
            ("NAXIS1", "200"),
            ("NAXIS2", "200"),
            ("CRPIX1", "100"),
            ("CRPIX2", "100"),
            ("CRVAL1", "83.633"),
            ("CRVAL2", "22.014"),
            ("CD1_1", "-7.27778E-05"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "7.27778E-05"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);

        let wcs = WcsTransform::from_header(&h).unwrap();

        let coord = wcs.pixel_to_world(150.0, 199.0);
        let (px, py) = wcs.world_to_pixel(coord.ra, coord.dec);
        assert!((px - 150.0).abs() < 1e-3);
        assert!((py - 199.0).abs() < 1e-3);
    }

    #[test]
    fn test_world_to_pixel_batch_matches_single() {
        let h = make_header(&[
            ("NAXIS1", "512"),
            ("NAXIS2", "512"),
            ("CRPIX1", "256"),
            ("CRPIX2", "256"),
            ("CRVAL1", "10.684"),
            ("CRVAL2", "41.269"),
            ("CDELT1", "-0.0003"),
            ("CDELT2", "0.0003"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);
        let wcs = WcsTransform::from_header(&h).unwrap();

        let pixels = [(0.0, 0.0), (100.0, 300.0), (255.0, 255.0), (511.0, 40.0)];
        let sky: Vec<(f64, f64)> = pixels
            .iter()
            .map(|&(x, y)| {
                let c = wcs.pixel_to_world(x, y);
                (c.ra, c.dec)
            })
            .collect();

        let batch = wcs.world_to_pixel_batch(&sky);
        assert_eq!(batch.len(), pixels.len());
        for (i, (&(bx, by), &(sx, sy))) in batch.iter().zip(pixels.iter()).enumerate() {
            let (single_x, single_y) = wcs.world_to_pixel(sky[i].0, sky[i].1);
            assert!((bx - single_x).abs() < 1e-9);
            assert!((by - single_y).abs() < 1e-9);
            assert!((bx - sx).abs() < 1e-3, "x round-trip {bx} vs {sx}");
            assert!((by - sy).abs() < 1e-3, "y round-trip {by} vs {sy}");
        }
    }

    #[test]
    fn test_angular_separation_reference_values() {
        assert!(angular_separation(10.0, 20.0, 10.0, 20.0).abs() < 1e-12);

        let s = angular_separation(30.0, 10.0, 30.0, 11.0);
        assert!((s - 1.0).abs() < 1e-9, "expected 1.0 deg, got {s}");

        let s = angular_separation(0.0, 0.0, 1.0, 0.0);
        assert!((s - 1.0).abs() < 1e-9, "expected 1.0 deg, got {s}");

        let s = angular_separation(0.0, 60.0, 1.0, 60.0);
        assert!((s - 0.5).abs() < 1e-4, "expected ~0.5 deg, got {s}");

        let s = angular_separation(0.0, 0.0, 180.0, 0.0);
        assert!((s - 180.0).abs() < 1e-6, "expected 180 deg, got {s}");
    }

    #[test]
    fn test_crpix_center() {
        let h = make_header(&[
            ("NAXIS1", "512"),
            ("NAXIS2", "512"),
            ("CRPIX1", "256"),
            ("CRPIX2", "256"),
            ("CRVAL1", "10.684"),
            ("CRVAL2", "41.269"),
            ("CDELT1", "-0.0003"),
            ("CDELT2", "0.0003"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);

        let wcs = WcsTransform::from_header(&h).unwrap();
        let center = wcs.pixel_to_world(255.0, 255.0);
        assert!((center.ra - 10.684).abs() < 0.01);
        assert!((center.dec - 41.269).abs() < 0.01);
    }

    #[test]
    fn test_pixel_scale() {
        let h = make_header(&[
            ("NAXIS1", "10"),
            ("NAXIS2", "10"),
            ("CRPIX1", "1"),
            ("CRPIX2", "1"),
            ("CRVAL1", "0.0"),
            ("CRVAL2", "0.0"),
            ("CDELT1", "-0.001"),
            ("CDELT2", "0.001"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);

        let wcs = WcsTransform::from_header(&h).unwrap();
        let scale = wcs.pixel_scale_arcsec();
        assert!((scale - 3.6).abs() < 0.01);
    }

    #[test]
    fn test_celestial_display() {
        let c = CelestialCoord { ra: 83.633, dec: 22.014 };
        let s = format!("{}", c);
        assert!(s.contains("h"));
        assert!(s.contains("°"));
    }

    #[test]
    fn test_raw_params() {
        let h = make_header(&[
            ("NAXIS1", "400"),
            ("NAXIS2", "400"),
            ("CRPIX1", "100"),
            ("CRPIX2", "200"),
            ("CRVAL1", "83.633"),
            ("CRVAL2", "22.014"),
            ("CD1_1", "-7.27778E-05"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "7.27778E-05"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);

        let wcs = WcsTransform::from_header(&h).unwrap();
        let (crpix1, crpix2, crval1, _, cd, proj) = wcs.raw_params();
        assert!((crpix1 - 100.0).abs() < 1e-10);
        assert!((crpix2 - 200.0).abs() < 1e-10);
        assert!((crval1 - 83.633).abs() < 1e-10);
        assert_eq!(proj, "TAN");
        assert!((cd[0][0] - (-7.27778e-05)).abs() < 1e-12);
    }

    #[test]
    fn test_projection_code_variants() {
        // Replaces the old `detect_projection`-based test: the engine no longer
        // funnels projections through a 4-variant enum, so assert on the string
        // `raw_params()` reports instead. Includes a projection beyond the legacy
        // four (AIT) to demonstrate the widened coverage.
        for (ctype, ctype2, expected) in [
            ("RA---TAN", "DEC--TAN", "TAN"),
            ("RA---SIN", "DEC--SIN", "SIN"),
            ("RA---ARC", "DEC--ARC", "ARC"),
            ("RA---CAR", "DEC--CAR", "CAR"),
            ("GLON-TAN", "GLAT-TAN", "TAN"),
            ("RA---TAN-SIP", "DEC--TAN-SIP", "TAN"),
            ("RA---SIN-SIP", "DEC--SIN-SIP", "SIN"),
            ("RA---AIT", "DEC--AIT", "AIT"),
        ] {
            let h = make_header(&[
                ("NAXIS1", "512"),
                ("NAXIS2", "512"),
                ("CRPIX1", "256"),
                ("CRPIX2", "256"),
                ("CRVAL1", "10.0"),
                ("CRVAL2", "20.0"),
                ("CDELT1", "-0.001"),
                ("CDELT2", "0.001"),
                ("CTYPE1", ctype),
                ("CTYPE2", ctype2),
            ]);
            let wcs = WcsTransform::from_header(&h).unwrap();
            assert_eq!(wcs.raw_params().5, expected, "Failed for {ctype}");
        }
    }

    fn sip_header() -> HduHeader {
        make_header(&[
            ("NAXIS1", "2048"),
            ("NAXIS2", "2048"),
            ("CRPIX1", "1024"),
            ("CRPIX2", "1024"),
            ("CRVAL1", "83.633"),
            ("CRVAL2", "22.014"),
            ("CD1_1", "-2.7778E-04"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "2.7778E-04"),
            ("CTYPE1", "RA---TAN-SIP"),
            ("CTYPE2", "DEC--TAN-SIP"),
            ("A_ORDER", "2"),
            ("A_2_0", "2.5E-06"),
            ("A_0_2", "1.2E-06"),
            ("A_1_1", "-1.5E-06"),
            ("B_ORDER", "2"),
            ("B_2_0", "-1.1E-06"),
            ("B_0_2", "2.2E-06"),
            ("B_1_1", "0.9E-06"),
        ])
    }

    #[test]
    fn test_sip_changes_corner_coordinates() {
        let with_sip = WcsTransform::from_header(&sip_header()).unwrap();

        let mut cards: Vec<(&str, &str)> = vec![
            ("NAXIS1", "2048"),
            ("NAXIS2", "2048"),
            ("CRPIX1", "1024"),
            ("CRPIX2", "1024"),
            ("CRVAL1", "83.633"),
            ("CRVAL2", "22.014"),
            ("CD1_1", "-2.7778E-04"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "2.7778E-04"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ];
        cards.retain(|(k, _)| !k.starts_with("A_") && !k.starts_with("B_"));
        let without_sip = WcsTransform::from_header(&make_header(&cards)).unwrap();

        let center_sip = with_sip.pixel_to_world(1023.0, 1023.0);
        let center_lin = without_sip.pixel_to_world(1023.0, 1023.0);
        assert!((center_sip.ra - center_lin.ra).abs() < 1e-9);
        assert!((center_sip.dec - center_lin.dec).abs() < 1e-9);

        let corner_sip = with_sip.pixel_to_world(0.0, 0.0);
        let corner_lin = without_sip.pixel_to_world(0.0, 0.0);
        let dra = (corner_sip.ra - corner_lin.ra).abs() * 3600.0;
        let ddec = (corner_sip.dec - corner_lin.dec).abs() * 3600.0;
        assert!(dra + ddec > 1.0, "SIP had no effect at corner: {} {}", dra, ddec);
    }

    #[test]
    fn test_sip_roundtrip_iterative_inverse() {
        // SIP is applied/inverted by this wrapper itself (see `SipPoly`'s doc
        // comment), so this exercises our own 12-iteration fixed-point inverse
        // (sip_header() has no AP/BP), independent of the engine.
        let wcs = WcsTransform::from_header(&sip_header()).unwrap();
        for &(x, y) in &[(0.0, 0.0), (100.0, 1900.0), (2047.0, 2047.0), (1024.0, 512.0)] {
            let coord = wcs.pixel_to_world(x, y);
            let (px, py) = wcs.world_to_pixel(coord.ra, coord.dec);
            assert!(
                (px - x).abs() < 1e-4 && (py - y).abs() < 1e-4,
                "roundtrip failed at ({}, {}): got ({}, {})",
                x,
                y,
                px,
                py
            );
        }
    }

    #[test]
    fn test_no_sip_keys_behaves_linearly() {
        let h = make_header(&[
            ("NAXIS1", "200"),
            ("NAXIS2", "200"),
            ("CRPIX1", "100"),
            ("CRPIX2", "100"),
            ("CRVAL1", "83.633"),
            ("CRVAL2", "22.014"),
            ("CD1_1", "-7.27778E-05"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "7.27778E-05"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);
        let wcs = WcsTransform::from_header(&h).unwrap();
        assert!(wcs.sip_forward_terms().0.is_none());
        assert!(wcs.sip_forward_terms().1.is_none());
        let coord = wcs.pixel_to_world(150.0, 199.0);
        let (px, py) = wcs.world_to_pixel(coord.ra, coord.dec);
        assert!((px - 150.0).abs() < 1e-3);
        assert!((py - 199.0).abs() < 1e-3);
    }

    #[test]
    fn test_pc_matrix_is_honored() {
        let cdelt1 = "-0.0005";
        let cdelt2 = "0.0005";
        let with_pc = make_header(&[
            ("NAXIS1", "1024"),
            ("NAXIS2", "1024"),
            ("CRPIX1", "512"),
            ("CRPIX2", "512"),
            ("CRVAL1", "150.0"),
            ("CRVAL2", "30.0"),
            ("CDELT1", cdelt1),
            ("CDELT2", cdelt2),
            ("PC1_1", "0.98"),
            ("PC1_2", "0.05"),
            ("PC2_1", "-0.03"),
            ("PC2_2", "0.99"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);
        let without_pc = make_header(&[
            ("NAXIS1", "1024"),
            ("NAXIS2", "1024"),
            ("CRPIX1", "512"),
            ("CRPIX2", "512"),
            ("CRVAL1", "150.0"),
            ("CRVAL2", "30.0"),
            ("CDELT1", cdelt1),
            ("CDELT2", cdelt2),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);

        let wcs_with_pc = WcsTransform::from_header(&with_pc).unwrap();
        let wcs_without_pc = WcsTransform::from_header(&without_pc).unwrap();

        let corner_with = wcs_with_pc.pixel_to_world(0.0, 0.0);
        let corner_without = wcs_without_pc.pixel_to_world(0.0, 0.0);
        let dra = (corner_with.ra - corner_without.ra).abs() * 3600.0;
        let ddec = (corner_with.dec - corner_without.dec).abs() * 3600.0;
        assert!(
            dra + ddec > 1.0,
            "PC matrix had no effect: dra={} ddec={}",
            dra,
            ddec
        );
    }

    /// Ground-truth cross-check against astropy (generated via
    /// `uv run --with astropy` per project convention -- see
    /// scratchpad `wcs_oracle.py` used to produce these numbers; not checked into
    /// the repo since the values are baked in here and CI needs no Python).
    /// Covers CD, SIP, SIN, a PC-matrix (no-CD) header, and CAR.
    #[test]
    fn test_astropy_oracle() {
        struct Case {
            name: &'static str,
            header: &'static [(&'static str, &'static str)],
            // (x, y, expected_ra_deg, expected_dec_deg), 0-based pixel coords
            points: &'static [(f64, f64, f64, f64)],
        }

        const CASES: &[Case] = &[
            Case {
                name: "tan_cd",
                header: &[
                    ("NAXIS1", "2048"),
                    ("NAXIS2", "2048"),
                    ("CRPIX1", "1024.0"),
                    ("CRPIX2", "1024.0"),
                    ("CRVAL1", "83.633"),
                    ("CRVAL2", "22.014"),
                    ("CD1_1", "-7.27778e-05"),
                    ("CD1_2", "0.0"),
                    ("CD2_1", "0.0"),
                    ("CD2_2", "7.27778e-05"),
                    ("CTYPE1", "RA---TAN"),
                    ("CTYPE2", "DEC--TAN"),
                ],
                points: &[
                    (0.0, 0.0, 83.71326444289475, 21.939528868238998),
                    (1023.0, 1023.0, 83.633, 22.013999999999996),
                    (2047.0, 2047.0, 83.55257259192761, 22.08850475617715),
                    (300.0, 1700.0, 83.68977604406483, 22.063260765585806),
                ],
            },
            Case {
                name: "tan_sip",
                header: &[
                    ("NAXIS1", "2048"),
                    ("NAXIS2", "2048"),
                    ("CRPIX1", "1024.0"),
                    ("CRPIX2", "1024.0"),
                    ("CRVAL1", "83.633"),
                    ("CRVAL2", "22.014"),
                    ("CD1_1", "-2.7778e-04"),
                    ("CD1_2", "0.0"),
                    ("CD2_1", "0.0"),
                    ("CD2_2", "2.7778e-04"),
                    ("CTYPE1", "RA---TAN-SIP"),
                    ("CTYPE2", "DEC--TAN-SIP"),
                    ("A_ORDER", "2"),
                    ("A_2_0", "2.5e-06"),
                    ("A_0_2", "1.2e-06"),
                    ("A_1_1", "-1.5e-06"),
                    ("B_ORDER", "2"),
                    ("B_2_0", "-1.1e-06"),
                    ("B_0_2", "2.2e-06"),
                    ("B_1_1", "0.9e-06"),
                ],
                points: &[
                    (0.0, 0.0, 83.93821282879065, 21.730135195227888),
                    (1023.0, 1023.0, 83.633, 22.013999999999996),
                    (2047.0, 2047.0, 83.32487606396182, 22.298736055023593),
                    (100.0, 1900.0, 83.90874726902706, 22.25738633082282),
                ],
            },
            Case {
                name: "sin",
                header: &[
                    ("NAXIS1", "1024"),
                    ("NAXIS2", "1024"),
                    ("CRPIX1", "512.0"),
                    ("CRPIX2", "512.0"),
                    ("CRVAL1", "10.684"),
                    ("CRVAL2", "41.269"),
                    ("CD1_1", "-0.0003"),
                    ("CD1_2", "0.0"),
                    ("CD2_1", "0.0"),
                    ("CD2_2", "0.0003"),
                    ("CTYPE1", "RA---SIN"),
                    ("CTYPE2", "DEC--SIN"),
                ],
                points: &[
                    (0.0, 0.0, 10.887481967673958, 41.115520263173906),
                    (511.0, 511.0, 10.684, 41.269),
                    (1023.0, 1023.0, 10.479159209991808, 41.42241907730277),
                    (200.0, 800.0, 10.80829686017614, 41.35563328062953),
                ],
            },
            Case {
                name: "pc_matrix",
                header: &[
                    ("NAXIS1", "1024"),
                    ("NAXIS2", "1024"),
                    ("CRPIX1", "512.0"),
                    ("CRPIX2", "512.0"),
                    ("CRVAL1", "150.0"),
                    ("CRVAL2", "30.0"),
                    ("CDELT1", "-0.0005"),
                    ("CDELT2", "0.0005"),
                    ("PC1_1", "0.98"),
                    ("PC1_2", "0.05"),
                    ("PC2_1", "-0.03"),
                    ("PC2_2", "0.99"),
                    ("CTYPE1", "RA---TAN"),
                    ("CTYPE2", "DEC--TAN"),
                ],
                points: &[
                    (0.0, 0.0, 150.30312472777297, 29.75437601817845),
                    (511.0, 511.0, 150.0, 30.0),
                    (1023.0, 1023.0, 149.69477557201301, 30.245404726187203),
                    (700.0, 300.0, 149.8992632313001, 29.89268186218835),
                ],
            },
            Case {
                name: "car",
                header: &[
                    ("NAXIS1", "720"),
                    ("NAXIS2", "360"),
                    ("CRPIX1", "360.5"),
                    ("CRPIX2", "180.5"),
                    ("CRVAL1", "0.0"),
                    ("CRVAL2", "0.0"),
                    ("CDELT1", "-0.5"),
                    ("CDELT2", "0.5"),
                    ("CTYPE1", "RA---CAR"),
                    ("CTYPE2", "DEC--CAR"),
                ],
                points: &[
                    (0.0, 0.0, 179.75, -89.75),
                    (359.0, 179.0, 0.25, -0.25),
                    (719.0, 359.0, 180.25, 89.75),
                    (100.0, 250.0, 129.75, 35.25),
                ],
            },
        ];

        const TOL_DEG: f64 = 1e-8;
        for case in CASES {
            let h = make_header(case.header);
            let wcs = WcsTransform::from_header(&h)
                .unwrap_or_else(|e| panic!("{}: from_header failed: {e}", case.name));
            for &(x, y, expected_ra, expected_dec) in case.points {
                let got = wcs.pixel_to_world(x, y);
                assert!(
                    (got.ra - expected_ra).abs() < TOL_DEG,
                    "{} @ ({x},{y}): ra got={} want={}",
                    case.name,
                    got.ra,
                    expected_ra
                );
                assert!(
                    (got.dec - expected_dec).abs() < TOL_DEG,
                    "{} @ ({x},{y}): dec got={} want={}",
                    case.name,
                    got.dec,
                    expected_dec
                );
            }
        }
    }

    #[test]
    fn pc_cdelt_sip_header_matches_the_astropy_goldens_in_both_directions() {
        let wcs = WcsTransform::from_header(&make_header(&[
            ("NAXIS1", "2048"),
            ("NAXIS2", "2048"),
            ("CRPIX1", "1024.0"),
            ("CRPIX2", "1024.0"),
            ("CRVAL1", "83.633"),
            ("CRVAL2", "22.014"),
            ("CDELT1", "-1.3889e-04"),
            ("CDELT2", "5.5556e-04"),
            ("PC1_1", "2.0"),
            ("PC1_2", "0.0"),
            ("PC2_1", "0.0"),
            ("PC2_2", "0.5"),
            ("CTYPE1", "RA---TAN-SIP"),
            ("CTYPE2", "DEC--TAN-SIP"),
            ("A_ORDER", "2"),
            ("A_2_0", "2.5e-06"),
            ("A_0_2", "1.2e-06"),
            ("A_1_1", "-1.5e-06"),
            ("B_ORDER", "2"),
            ("B_2_0", "-1.1e-06"),
            ("B_0_2", "2.2e-06"),
            ("B_1_1", "0.9e-06"),
        ]))
        .unwrap();
        assert_eq!(wcs.raw_params().4, [[-2.7778e-04, 0.0], [0.0, 2.7778e-04]]);
        assert!(wcs.sip_forward_terms().0.is_some() && wcs.sip_forward_terms().1.is_some());
        let reference = wcs.pixel_to_world(1023.0, 1023.0);
        assert!((reference.ra - 83.633).abs() < 1e-8 && (reference.dec - 22.014).abs() < 1e-8, "{reference:?}");

        for (x, y, ra, dec) in [
            (0.0, 0.0, 83.93821282879065, 21.730135195227888),
            (2047.0, 2047.0, 83.32487606396182, 22.298736055023593),
            (100.0, 1900.0, 83.90874726902706, 22.25738633082282),
        ] {
            let got = wcs.pixel_to_world(x, y);
            assert!(
                (got.ra - ra).abs() < 1e-8 && (got.dec - dec).abs() < 1e-8,
                "({x},{y}): got ({},{}) want ({ra},{dec})",
                got.ra,
                got.dec
            );
            let (px, py) = wcs.world_to_pixel(ra, dec);
            assert!(
                (px - x).abs() < ROUND_TRIP_TOL_PX && (py - y).abs() < ROUND_TRIP_TOL_PX,
                "({ra},{dec}) -> ({px},{py}) want ({x},{y})"
            );
        }
    }

    #[test]
    fn projection_cards_accept_the_fortran_d_exponent() {
        for (ctype1, ctype2, key, d_value, e_value) in [
            ("RA---TAN", "DEC--TAN", "LONPOLE", "1.7D+02", "170.0"),
            ("RA---AZP", "DEC--AZP", "PV2_1", "2.0D-01", "0.2"),
        ] {
            let transform = |value: Option<&str>| {
                let mut cards = vec![
                    ("NAXIS1", "200"),
                    ("NAXIS2", "200"),
                    ("CRPIX1", "100.5"),
                    ("CRPIX2", "100.5"),
                    ("CRVAL1", "150.0"),
                    ("CRVAL2", "30.0"),
                    ("CDELT1", "-0.05"),
                    ("CDELT2", "0.05"),
                    ("CTYPE1", ctype1),
                    ("CTYPE2", ctype2),
                ];
                cards.extend(value.map(|v| (key, v)));
                WcsTransform::from_header(&make_header(&cards)).unwrap()
            };
            let fortran = transform(Some(d_value)).pixel_to_world(10.0, 190.0);
            let decimal = transform(Some(e_value)).pixel_to_world(10.0, 190.0);
            let absent = transform(None).pixel_to_world(10.0, 190.0);
            assert!(
                (fortran.ra - decimal.ra).abs() < 1e-12 && (fortran.dec - decimal.dec).abs() < 1e-12,
                "{key} = {d_value}: {fortran:?} vs {decimal:?}"
            );
            assert!(
                (decimal.ra - absent.ra).abs() + (decimal.dec - absent.dec).abs() > 1e-6,
                "{key} must change the transform: {decimal:?} vs {absent:?}"
            );
        }
    }

    const ROUND_TRIP_TOL_PX: f64 = 1e-6;
    const ORACLE_TOL_DEG: f64 = 1e-9;

    fn tan_oracle(cd: [[f64; 2]; 2], crpix: (f64, f64), crval: (f64, f64), x: f64, y: f64) -> (f64, f64) {
        let dx = x + 1.0 - crpix.0;
        let dy = y + 1.0 - crpix.1;
        let xi = (cd[0][0] * dx + cd[0][1] * dy).to_radians();
        let eta = (cd[1][0] * dx + cd[1][1] * dy).to_radians();
        let (a0, d0) = (crval.0.to_radians(), crval.1.to_radians());
        let denom = d0.cos() - eta * d0.sin();
        let ra = a0 + xi.atan2(denom);
        let dec = (d0.sin() + eta * d0.cos()).atan2((xi * xi + denom * denom).sqrt());
        (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
    }

    fn sci(v: f64) -> String {
        format!("{v:.17e}")
    }

    fn tan_header(cards: &[(&str, String)]) -> HduHeader {
        let mut pairs: Vec<(&str, &str)> = vec![("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN")];
        pairs.extend(cards.iter().map(|(k, v)| (*k, v.as_str())));
        make_header(&pairs)
    }

    fn cd_cards(size: usize, crpix: f64, crval: (f64, f64), cd: [[f64; 2]; 2]) -> Vec<(&'static str, String)> {
        vec![
            ("NAXIS1", size.to_string()),
            ("NAXIS2", size.to_string()),
            ("CRPIX1", sci(crpix)),
            ("CRPIX2", sci(crpix)),
            ("CRVAL1", sci(crval.0)),
            ("CRVAL2", sci(crval.1)),
            ("CD1_1", sci(cd[0][0])),
            ("CD1_2", sci(cd[0][1])),
            ("CD2_1", sci(cd[1][0])),
            ("CD2_2", sci(cd[1][1])),
        ]
    }

    fn probe_points(size: usize) -> Vec<(f64, f64)> {
        let n = size as f64 - 1.0;
        vec![(0.0, 0.0), (n, 0.0), (0.0, n), (n, n), (0.3 * n, 0.8 * n), (0.5 * n, 0.5 * n)]
    }

    fn max_round_trip_error(wcs: &WcsTransform, points: &[(f64, f64)]) -> f64 {
        points.iter().fold(0.0_f64, |acc, &(x, y)| {
            let c = wcs.pixel_to_world(x, y);
            let (px, py) = wcs.world_to_pixel(c.ra, c.dec);
            acc.max((px - x).abs()).max((py - y).abs())
        })
    }

    fn deg_rot(theta_deg: f64) -> (f64, f64) {
        theta_deg.to_radians().sin_cos()
    }

    #[test]
    fn tan_oracle_agrees_with_the_astropy_goldens() {
        let tan_cd = [[-7.27778e-05, 0.0], [0.0, 7.27778e-05]];
        for (x, y, ra, dec) in [
            (0.0, 0.0, 83.71326444289475, 21.939528868238998),
            (2047.0, 2047.0, 83.55257259192761, 22.08850475617715),
            (300.0, 1700.0, 83.68977604406483, 22.063260765585806),
        ] {
            let (r, d) = tan_oracle(tan_cd, (1024.0, 1024.0), (83.633, 22.014), x, y);
            assert!((r - ra).abs() < 1e-8 && (d - dec).abs() < 1e-8, "tan_cd ({x},{y}) -> ({r},{d})");
        }
        let pc_cd = [[-0.0005 * 0.98, -0.0005 * 0.05], [0.0005 * -0.03, 0.0005 * 0.99]];
        for (x, y, ra, dec) in [
            (0.0, 0.0, 150.30312472777297, 29.75437601817845),
            (1023.0, 1023.0, 149.69477557201301, 30.245404726187203),
            (700.0, 300.0, 149.8992632313001, 29.89268186218835),
        ] {
            let (r, d) = tan_oracle(pc_cd, (512.0, 512.0), (150.0, 30.0), x, y);
            assert!((r - ra).abs() < 1e-8 && (d - dec).abs() < 1e-8, "pc ({x},{y}) -> ({r},{d})");
        }
    }

    #[test]
    fn world_to_pixel_inverts_a_mirrored_rotated_cd() {
        let s = 0.5 / 3600.0;
        let (sn, cs) = deg_rot(30.0);
        let cd = [[s * cs, -s * sn], [s * sn, s * cs]];
        assert!(cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0] > 0.0);
        let crval = (210.0, 54.0);
        let wcs = WcsTransform::from_header(&tan_header(&cd_cards(4096, 2048.5, crval, cd))).unwrap();

        let points = probe_points(4096);
        let err = max_round_trip_error(&wcs, &points);
        assert!(err < ROUND_TRIP_TOL_PX, "det>0 rotated CD round trip off by {err} px");

        for &(x, y) in &points {
            let (ra, dec) = tan_oracle(cd, (2048.5, 2048.5), crval, x, y);
            let got = wcs.pixel_to_world(x, y);
            assert!((got.ra - ra).abs() < ORACLE_TOL_DEG && (got.dec - dec).abs() < ORACLE_TOL_DEG, "({x},{y})");
            let (px, py) = wcs.world_to_pixel(ra, dec);
            assert!((px - x).abs() < ROUND_TRIP_TOL_PX && (py - y).abs() < ROUND_TRIP_TOL_PX, "({x},{y}) -> ({px},{py})");
        }
    }

    #[test]
    fn world_to_pixel_inverts_an_anisotropic_cd() {
        let s = 0.2 / 3600.0;
        let (sn, cs) = deg_rot(45.0);
        let cd = [[-s * cs, -1.01 * s * sn], [-s * sn, 1.01 * s * cs]];
        let crval = (35.5, -12.25);
        let wcs = WcsTransform::from_header(&tan_header(&cd_cards(4096, 2048.5, crval, cd))).unwrap();

        let points = probe_points(4096);
        let err = max_round_trip_error(&wcs, &points);
        assert!(err < ROUND_TRIP_TOL_PX, "anisotropic CD round trip off by {err} px");
        for &(x, y) in &points {
            let (ra, dec) = tan_oracle(cd, (2048.5, 2048.5), crval, x, y);
            let (px, py) = wcs.world_to_pixel(ra, dec);
            assert!((px - x).abs() < ROUND_TRIP_TOL_PX && (py - y).abs() < ROUND_TRIP_TOL_PX, "({x},{y}) -> ({px},{py})");
        }
    }

    #[test]
    fn world_to_pixel_inverts_a_resampled_jwst_like_pc_header() {
        let cdelt = 8.6737e-06;
        let (sn, cs) = deg_rot(20.0);
        let (sx, sy) = (1.0, 1.25);
        let pc = [[-cs * sx, sn * sy], [sn * sx, cs * sy]];
        let crval = (53.16, -27.78);
        let h = tan_header(&[
            ("NAXIS1", "2048".to_string()),
            ("NAXIS2", "2048".to_string()),
            ("CRPIX1", sci(1024.5)),
            ("CRPIX2", sci(1024.5)),
            ("CRVAL1", sci(crval.0)),
            ("CRVAL2", sci(crval.1)),
            ("CDELT1", sci(cdelt)),
            ("CDELT2", sci(cdelt)),
            ("PC1_1", sci(pc[0][0])),
            ("PC1_2", sci(pc[0][1])),
            ("PC2_1", sci(pc[1][0])),
            ("PC2_2", sci(pc[1][1])),
        ]);
        let wcs = WcsTransform::from_header(&h).unwrap();
        let cd = [[cdelt * pc[0][0], cdelt * pc[0][1]], [cdelt * pc[1][0], cdelt * pc[1][1]]];

        let points = probe_points(2048);
        let err = max_round_trip_error(&wcs, &points);
        assert!(err < ROUND_TRIP_TOL_PX, "JWST-like PC round trip off by {err} px");
        for &(x, y) in &points {
            let (ra, dec) = tan_oracle(cd, (1024.5, 1024.5), crval, x, y);
            let got = wcs.pixel_to_world(x, y);
            assert!((got.ra - ra).abs() < ORACLE_TOL_DEG && (got.dec - dec).abs() < ORACLE_TOL_DEG, "({x},{y})");
            let (px, py) = wcs.world_to_pixel(ra, dec);
            assert!((px - x).abs() < ROUND_TRIP_TOL_PX && (py - y).abs() < ROUND_TRIP_TOL_PX, "({x},{y}) -> ({px},{py})");
        }
    }

    #[test]
    fn world_to_pixel_round_trips_the_skewed_hst_cd_and_the_oracle_pc_header() {
        let hst = make_header(&[
            ("NAXIS1", "1600"),
            ("NAXIS2", "1600"),
            ("CRPIX1", "386.5"),
            ("CRPIX2", "396."),
            ("CRVAL1", "274.71149247724"),
            ("CRVAL2", "-13.816384007184"),
            ("CD1_1", "1.878013E-5"),
            ("CD1_2", "-2.031193E-5"),
            ("CD2_1", "-2.029358E-5"),
            ("CD2_2", "-1.879711E-5"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);
        let wcs = WcsTransform::from_header(&hst).unwrap();
        let err = max_round_trip_error(&wcs, &[(3.0, 3.0), (100.0, 100.0), (800.0, 800.0), (0.0, 1599.0), (1599.0, 1599.0)]);
        assert!(err < ROUND_TRIP_TOL_PX, "skewed HST CD round trip off by {err} px");

        let pc = make_header(&[
            ("NAXIS1", "1024"),
            ("NAXIS2", "1024"),
            ("CRPIX1", "512.0"),
            ("CRPIX2", "512.0"),
            ("CRVAL1", "150.0"),
            ("CRVAL2", "30.0"),
            ("CDELT1", "-0.0005"),
            ("CDELT2", "0.0005"),
            ("PC1_1", "0.98"),
            ("PC1_2", "0.05"),
            ("PC2_1", "-0.03"),
            ("PC2_2", "0.99"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
        ]);
        let wcs = WcsTransform::from_header(&pc).unwrap();
        let err = max_round_trip_error(&wcs, &probe_points(1024));
        assert!(err < ROUND_TRIP_TOL_PX, "oracle PC header round trip off by {err} px");
    }

    #[test]
    fn pc_matrix_rows_are_scaled_by_cdelt() {
        let (sn, cs) = deg_rot(30.0);
        let (cdelt1, cdelt2) = (-1e-5, 1e-5);
        let pc_header = tan_header(&[
            ("NAXIS1", "512".to_string()),
            ("NAXIS2", "512".to_string()),
            ("CRPIX1", "256.5".to_string()),
            ("CRPIX2", "256.5".to_string()),
            ("CRVAL1", "10.0".to_string()),
            ("CRVAL2", "40.0".to_string()),
            ("CDELT1", sci(cdelt1)),
            ("CDELT2", sci(cdelt2)),
            ("PC1_1", sci(cs)),
            ("PC1_2", sci(-sn)),
            ("PC2_1", sci(sn)),
            ("PC2_2", sci(cs)),
        ]);
        let expected = [[cdelt1 * cs, cdelt1 * -sn], [cdelt2 * sn, cdelt2 * cs]];
        let cd_header = tan_header(&cd_cards(512, 256.5, (10.0, 40.0), expected));

        let from_pc = WcsTransform::from_header(&pc_header).unwrap();
        let from_cd = WcsTransform::from_header(&cd_header).unwrap();
        let cd = from_pc.raw_params().4;
        for i in 0..2 {
            for j in 0..2 {
                assert!((cd[i][j] - expected[i][j]).abs() < 1e-18, "CD{}_{} = {} want {}", i + 1, j + 1, cd[i][j], expected[i][j]);
            }
        }
        assert!((cd[0][1] - 5e-6).abs() < 1e-15 && (cd[1][0] - 5e-6).abs() < 1e-15, "{cd:?}");
        let a = from_pc.pixel_to_world(10.0, 500.0);
        let b = from_cd.pixel_to_world(10.0, 500.0);
        assert!((a.ra - b.ra).abs() < 1e-12 && (a.dec - b.dec).abs() < 1e-12);

        let (sn, cs) = deg_rot(45.0);
        let skewed = tan_header(&[
            ("NAXIS1", "100".to_string()),
            ("NAXIS2", "100".to_string()),
            ("CRPIX1", "50".to_string()),
            ("CRPIX2", "50".to_string()),
            ("CRVAL1", "10.0".to_string()),
            ("CRVAL2", "40.0".to_string()),
            ("CDELT1", "-1e-4".to_string()),
            ("CDELT2", "2e-4".to_string()),
            ("PC1_1", sci(cs)),
            ("PC1_2", sci(-sn)),
            ("PC2_1", sci(sn)),
            ("PC2_2", sci(cs)),
        ]);
        let wcs = WcsTransform::from_header(&skewed).unwrap();
        let per_axis = (0.5f64 * (1e-4f64.powi(2) + 2e-4f64.powi(2))).sqrt();
        let (fov_w, fov_h) = wcs.field_of_view(100, 100);
        assert!((fov_w - 100.0 * per_axis * 60.0).abs() < 1e-9, "fov_w {fov_w}");
        assert!((fov_h - 100.0 * per_axis * 60.0).abs() < 1e-9, "fov_h {fov_h}");
        assert!((wcs.pixel_scale_arcsec() - per_axis * 3600.0).abs() < 1e-9);
    }

    #[test]
    fn crota2_headers_follow_the_paper_ii_matrix_in_both_directions() {
        for (cdelt1, cdelt2, crota2) in [(2.78e-4, 2.78e-4, 30.0), (-2.78e-4, 1.01 * 2.78e-4, 45.0), (-2.78e-4, 2.78e-4, 30.0)] {
            let h = tan_header(&[
                ("NAXIS1", "1024".to_string()),
                ("NAXIS2", "1024".to_string()),
                ("CRPIX1", "512".to_string()),
                ("CRPIX2", "512".to_string()),
                ("CRVAL1", "150.0".to_string()),
                ("CRVAL2", "30.0".to_string()),
                ("CDELT1", sci(cdelt1)),
                ("CDELT2", sci(cdelt2)),
                ("CROTA2", sci(crota2)),
            ]);
            let wcs = WcsTransform::from_header(&h).unwrap();
            let (sn, cs) = deg_rot(crota2);
            let cd = [[cdelt1 * cs, -cdelt2 * sn], [cdelt1 * sn, cdelt2 * cs]];
            let raw = wcs.raw_params().4;
            for i in 0..2 {
                for j in 0..2 {
                    assert!((raw[i][j] - cd[i][j]).abs() < 1e-18);
                }
            }
            for &(x, y) in &probe_points(1024) {
                let (ra, dec) = tan_oracle(cd, (512.0, 512.0), (150.0, 30.0), x, y);
                let got = wcs.pixel_to_world(x, y);
                assert!(
                    (got.ra - ra).abs() < ORACLE_TOL_DEG && (got.dec - dec).abs() < ORACLE_TOL_DEG,
                    "CDELT ({cdelt1},{cdelt2}) CROTA2 {crota2} at ({x},{y}): got ({},{}) want ({ra},{dec})",
                    got.ra,
                    got.dec
                );
                let (px, py) = wcs.world_to_pixel(ra, dec);
                assert!((px - x).abs() < ROUND_TRIP_TOL_PX && (py - y).abs() < ROUND_TRIP_TOL_PX);
            }
        }
    }

    fn frame_header(ctype1: &str, ctype2: &str, crval: (&str, &str), extra: &[(&str, &str)]) -> HduHeader {
        let mut pairs = vec![
            ("NAXIS1", "100"),
            ("NAXIS2", "100"),
            ("CRPIX1", "50"),
            ("CRPIX2", "50"),
            ("CRVAL1", crval.0),
            ("CRVAL2", crval.1),
            ("CDELT1", "-0.001"),
            ("CDELT2", "0.001"),
            ("CTYPE1", ctype1),
            ("CTYPE2", ctype2),
        ];
        pairs.extend_from_slice(extra);
        make_header(&pairs)
    }

    #[test]
    fn galactic_headers_convert_to_icrs_even_with_radesys_and_equinox() {
        for extra in [&[][..], &[("RADESYS", "ICRS"), ("EQUINOX", "2000.0")][..]] {
            let wcs = WcsTransform::from_header(&frame_header("GLON-TAN", "GLAT-TAN", ("0.0", "0.0"), extra)).unwrap();
            let c = wcs.pixel_to_world(49.0, 49.0);
            assert!((c.ra - 266.4049882865447).abs() < 1e-4 && (c.dec + 28.936177761791473).abs() < 1e-4, "{extra:?}: {c:?}");
            let (px, py) = wcs.world_to_pixel(266.4049882865447, -28.936177761791473);
            assert!((px - 49.0).abs() < 0.1 && (py - 49.0).abs() < 0.1, "{extra:?}: ({px},{py})");
            let err = max_round_trip_error(&wcs, &probe_points(100));
            assert!(err < ROUND_TRIP_TOL_PX, "{err}");
        }
    }

    #[test]
    fn ecliptic_headers_convert_to_icrs() {
        let wcs = WcsTransform::from_header(&frame_header("ELON-TAN", "ELAT-TAN", ("90.0", "0.0"), &[])).unwrap();
        let c = wcs.pixel_to_world(49.0, 49.0);
        let (ra, dec) = convert_to_icrs(SkyFrame::EclipticJ2000, 90.0, 0.0);
        assert!((c.ra - ra).abs() < 1e-9 && (c.dec - dec).abs() < 1e-9, "{c:?}");
        assert!((c.dec - 23.4392794).abs() < 1e-4, "{c:?}");
        let (px, py) = wcs.world_to_pixel(ra, dec);
        assert!((px - 49.0).abs() < 1e-6 && (py - 49.0).abs() < 1e-6);
    }

    #[test]
    fn unsupported_celestial_frames_are_rejected_instead_of_labelled_icrs() {
        for (ctype1, ctype2, extra, needle) in [
            ("RA---TAN", "DEC--TAN", &[("RADESYS", "FK4"), ("EQUINOX", "1950.0")][..], "FK4"),
            ("RA---TAN", "DEC--TAN", &[("EQUINOX", "1950.0")][..], "FK4"),
            ("RA---TAN", "DEC--TAN", &[("RADESYS", "FK5"), ("EQUINOX", "1975.0")][..], "1975"),
            ("RA---TAN", "DEC--TAN", &[("RADESYS", "GAPPT")][..], "GAPPT"),
            ("HPLN-TAN", "HPLT-TAN", &[][..], "HPLN"),
            ("DEC--TAN", "RA---TAN", &[][..], "DEC"),
        ] {
            let err = WcsTransform::from_header(&frame_header(ctype1, ctype2, ("83.0", "22.0"), extra))
                .expect_err(ctype1)
                .to_string();
            assert!(err.contains(needle), "{ctype1} {extra:?}: {err}");
        }
        for extra in [
            &[][..],
            &[("RADESYS", "ICRS")][..],
            &[("RADESYS", "FK5"), ("EQUINOX", "2000.0")][..],
            &[("EQUINOX", "2000")][..],
            &[("EPOCH", "2015.5")][..],
        ] {
            let wcs = WcsTransform::from_header(&frame_header("RA---TAN", "DEC--TAN", ("83.0", "22.0"), extra)).unwrap();
            let c = wcs.pixel_to_world(49.0, 49.0);
            assert!((c.ra - 83.0).abs() < 1e-9 && (c.dec - 22.0).abs() < 1e-9, "{extra:?}: {c:?}");
        }
    }

    #[test]
    fn malformed_ctype1_is_an_error_not_a_panic() {
        for ctype1 in ["RA---TA\u{FFFD}", "RA-TAN-SIP", "RA\u{FFFD}-TAN", "RA--TAN"] {
            let h = make_header(&[
                ("NAXIS1", "100"),
                ("NAXIS2", "100"),
                ("CRPIX1", "50"),
                ("CRPIX2", "50"),
                ("CRVAL1", "10.0"),
                ("CRVAL2", "20.0"),
                ("CDELT1", "-0.001"),
                ("CDELT2", "0.001"),
                ("CTYPE1", ctype1),
                ("CTYPE2", "DEC--TAN"),
            ]);
            let err = WcsTransform::from_header(&h).expect_err(ctype1).to_string();
            assert!(err.contains("CTYPE1"), "{ctype1}: {err}");
        }
    }

    #[test]
    fn a_header_without_ctype_is_not_a_celestial_wcs() {
        let mut h = make_header(&[
            ("NAXIS1", "2048"),
            ("NAXIS2", "512"),
            ("CRPIX1", "1024.5"),
            ("CRVAL1", "6563.0"),
            ("CDELT1", "0.5"),
            ("CRPIX2", "256.5"),
            ("CRVAL2", "0"),
            ("CDELT2", "1"),
        ]);
        let err = WcsTransform::from_header(&h).expect_err("a header without CTYPE is a linear axis pair");
        assert!(format!("{err:#}").contains("CTYPE1/CTYPE2 missing"), "{err:#}");
        h.set("CTYPE1", "RA---TAN".to_string());
        assert!(format!("{:#}", WcsTransform::from_header(&h).unwrap_err()).contains("CTYPE1/CTYPE2 missing"));
    }

    #[test]
    fn ctype2_must_pair_with_ctype1() {
        for (ctype1, ctype2) in [("RA---TAN", "WAVE    "), ("GLON-TAN", "DEC--TAN")] {
            let err = WcsTransform::from_header(&frame_header(ctype1, ctype2, ("83.0", "22.0"), &[]))
                .expect_err(ctype2)
                .to_string();
            assert!(err.contains("not a celestial longitude/latitude pair"), "{ctype1}/{ctype2}: {err}");
        }
        let err = WcsTransform::from_header(&frame_header("RA---TAN", "DE\u{FFFD}--TAN", ("83.0", "22.0"), &[]))
            .expect_err("non-ASCII CTYPE2")
            .to_string();
        assert!(err.starts_with("CTYPE2 '") && err.contains("ASCII"), "{err}");
    }

    #[test]
    fn ctype_projections_must_agree() {
        let err = WcsTransform::from_header(&frame_header("RA---TAN", "DEC--SIN", ("83.0", "22.0"), &[]))
            .expect_err("mixed projections")
            .to_string();
        assert!(err.contains("projection 'TAN' and CTYPE2 projection 'SIN' differ"), "{err}");
    }

    #[test]
    fn latitude_first_headers_are_refused_with_a_reason() {
        let err = WcsTransform::from_header(&frame_header("DEC--TAN", "RA---TAN", ("22.0", "83.0"), &[]))
            .expect_err("latitude first")
            .to_string();
        assert!(err.contains("latitude on axis 1"), "{err}");
    }

    #[test]
    fn sip_suffix_is_stripped_from_both_axes() {
        let wcs = WcsTransform::from_header(&sip_header()).unwrap();
        assert!(wcs.orientation(2048, 2048).sip_present);
        assert_eq!(wcs.raw_params().5, "TAN");
    }

    #[test]
    fn sip_fit_residual_cards_are_read() {
        let mut h = sip_header();
        h.set("SIPMXERR", "0.0123".to_string());
        h.set("SIPIVERR", "0.0456".to_string());
        let wcs = WcsTransform::from_header(&h).unwrap();
        assert_eq!(wcs.sip_fit_residuals(), (Some(0.0123), Some(0.0456)));
        let o = wcs.orientation(2048, 2048);
        assert_eq!(o.sip_max_err_px, Some(0.0123));
        assert_eq!(o.sip_inv_err_px, Some(0.0456));
        assert!(o.sip_present);

        let plain = WcsTransform::from_header(&sip_header()).unwrap().orientation(2048, 2048);
        assert_eq!((plain.sip_max_err_px, plain.sip_inv_err_px), (None, None));

        let mut bad = sip_header();
        bad.set("SIPMXERR", "-0.5".to_string());
        bad.set("SIPIVERR", "NaN".to_string());
        assert_eq!(WcsTransform::from_header(&bad).unwrap().sip_fit_residuals(), (None, None));
    }

    const HEAVY_F200W_I2D: &str =
        r"C:\astrokit\exampleFits\sample-data\heavyTest\jw02739-o001_t001_nircam_clear-f200w_i2d.fits";

    #[test]
    #[ignore]
    fn real_data_f200w_wcs_center_is_unchanged() {
        if !std::path::Path::new(HEAVY_F200W_I2D).exists() {
            return;
        }
        let header = crate::infra::image_source::load_plane_header(&crate::types::image_ref::ImageRef::parse(HEAVY_F200W_I2D)).unwrap();
        assert_eq!((header.get_i64("NAXIS1"), header.get_i64("NAXIS2")), (Some(14344), Some(8589)));
        assert_eq!(header.get("CTYPE1").map(str::trim), Some("RA---TAN"));
        assert_eq!(header.get("CTYPE2").map(str::trim), Some("DEC--TAN"));
        let wcs = WcsTransform::from_header(&header).unwrap();
        let (cx, cy) = pixel_center(14344, 8589);
        let c = wcs.pixel_to_world(cx, cy);
        assert!((c.ra - 274.729893924).abs() < 1e-6, "centre RA {} vs 274.729893924", c.ra);
        assert!((c.dec + 13.851839479).abs() < 1e-6, "centre Dec {} vs -13.851839479", c.dec);
        let o = wcs.orientation(14344, 8589);
        assert_eq!(o.sip_max_err_px, None);
        assert_eq!(o.sip_inv_err_px, None);
        assert!(!o.sip_present);
        assert_eq!(o.projection, "TAN");
    }

    #[test]
    fn pixel_center_and_edges_use_the_zero_based_pixel_centre_convention() {
        assert_eq!(pixel_center(100, 50), (49.5, 24.5));
        assert_eq!(pixel_center(1, 1), (0.0, 0.0));
        assert_eq!(pixel_edge_corners(4, 2), [(-0.5, -0.5), (3.5, -0.5), (3.5, 1.5), (-0.5, 1.5)]);
    }

    fn assert_close(actual: f64, expected: f64, tol: f64, what: &str) {
        assert!((actual - expected).abs() < tol, "{what}: {actual} vs {expected}");
    }

    fn assert_vec_close(actual: Option<(f64, f64)>, expected: (f64, f64), what: &str) {
        let (x, y) = actual.unwrap_or_else(|| panic!("{what} is None"));
        assert_close(x, expected.0, 1e-6, what);
        assert_close(y, expected.1, 1e-6, what);
    }

    #[test]
    fn position_angle_is_measured_east_of_north_from_zero_to_360() {
        let step_east = 0.01 / 20f64.to_radians().cos();
        assert_close(position_angle_deg(10.0, 20.0, 10.0, 21.0), 0.0, 1e-9, "north");
        assert_close(position_angle_deg(10.0, 20.0, 10.0 + step_east, 20.0), 90.0, 0.01, "east");
        assert_close(position_angle_deg(10.0, 20.0, 10.0, 19.0), 180.0, 1e-9, "south");
        assert_close(position_angle_deg(10.0, 20.0, 10.0 - step_east, 20.0), 270.0, 0.01, "west");
        assert_close(position_angle_deg(10.0, 20.0, 11.0, 21.0), 42.9531, 0.01, "astropy golden");
        assert_close(position_angle_deg(359.5, 0.0, 0.5, 0.0), 90.0, 1e-9, "east across the RA wrap");
        assert!(position_angle_deg(10.0, 20.0, 10.0, 20.0).is_finite(), "coincident points stay finite");
    }

    #[test]
    fn non_finite_inputs_give_nan_instead_of_panicking_in_both_directions() {
        use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd};
        let wcs = WcsTransform::from_header(&header_with_cd(north_up_cd())).unwrap();
        for (x, y) in [(f64::NAN, 1.0), (1.0, f64::INFINITY), (f64::MAX, f64::MAX)] {
            let c = wcs.pixel_to_world(x, y);
            assert!(c.ra.is_nan() && c.dec.is_nan(), "({x}, {y}) gave ({}, {})", c.ra, c.dec);
        }
        for (ra, dec) in [(f64::NAN, 2.0), (150.0, f64::NEG_INFINITY)] {
            let (px, py) = wcs.world_to_pixel(ra, dec);
            assert!(px.is_nan() && py.is_nan(), "({ra}, {dec}) gave ({px}, {py})");
        }
    }

    #[test]
    fn orientation_of_a_north_up_east_left_header_has_zero_rotation_and_normal_parity() {
        use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd};
        let wcs = WcsTransform::from_header(&header_with_cd(north_up_cd())).unwrap();
        let o = wcs.orientation(100, 100);
        assert_close(o.rotation_deg, 0.0, 1e-6, "rotation");
        assert!(!o.flipped);
        assert_close(o.pixel_scale_x_arcsec, 1.0, 1e-9, "scale x");
        assert_close(o.pixel_scale_y_arcsec, 1.0, 1e-9, "scale y");
        assert_eq!(o.projection, "TAN");
        assert!(!o.sip_present);
        assert_vec_close(o.north_vec, (0.0, 1.0), "north");
        assert_vec_close(o.east_vec, (-1.0, 0.0), "east");
    }

    #[test]
    fn orientation_of_a_rotated_header_rotates_the_cardinal_vectors() {
        use crate::core::imaging::region::test_support::{header_with_cd, rotated_cd};
        let wcs = WcsTransform::from_header(&header_with_cd(rotated_cd(30.0))).unwrap();
        let o = wcs.orientation(100, 100);
        assert_close(o.rotation_deg, 30.0, 1e-6, "rotation");
        assert!(!o.flipped);
        let (sn, cs) = 30f64.to_radians().sin_cos();
        assert_vec_close(o.north_vec, (-sn, cs), "north");
        assert_vec_close(o.east_vec, (-cs, -sn), "east");
    }

    #[test]
    fn orientation_of_a_positive_determinant_header_reports_flipped_parity_with_east_right() {
        use crate::core::imaging::region::test_support::header_with_cd;
        let s = 1.0 / 3600.0;
        let wcs = WcsTransform::from_header(&header_with_cd([[s, 0.0], [0.0, s]])).unwrap();
        let o = wcs.orientation(100, 100);
        assert!(o.flipped);
        assert_close(o.rotation_deg, 0.0, 1e-6, "rotation");
        assert_vec_close(o.north_vec, (0.0, 1.0), "north");
        assert_vec_close(o.east_vec, (1.0, 0.0), "east");
    }

    #[test]
    fn orientation_without_image_dimensions_falls_back_to_the_reference_pixel() {
        use crate::core::imaging::region::test_support::{header_with_cd, north_up_cd};
        let wcs = WcsTransform::from_header(&header_with_cd(north_up_cd())).unwrap();
        let o = wcs.orientation(0, 0);
        assert_vec_close(o.north_vec, (0.0, 1.0), "north");
        assert_vec_close(o.east_vec, (-1.0, 0.0), "east");
    }

    fn centred_tan_orientation(axes: (&str, &str), crval: (f64, f64), cd: [[f64; 2]; 2], naxis: usize) -> WcsOrientation {
        let n = naxis.to_string();
        let crpix = (naxis as f64 / 2.0 + 0.5).to_string();
        let (crval1, crval2) = (crval.0.to_string(), crval.1.to_string());
        let cd_cards: Vec<String> = [cd[0][0], cd[0][1], cd[1][0], cd[1][1]].iter().map(|v| format!("{v:.12e}")).collect();
        let h = make_header(&[
            ("NAXIS1", &n),
            ("NAXIS2", &n),
            ("CRPIX1", &crpix),
            ("CRPIX2", &crpix),
            ("CRVAL1", &crval1),
            ("CRVAL2", &crval2),
            ("CD1_1", &cd_cards[0]),
            ("CD1_2", &cd_cards[1]),
            ("CD2_1", &cd_cards[2]),
            ("CD2_2", &cd_cards[3]),
            ("CTYPE1", axes.0),
            ("CTYPE2", axes.1),
        ]);
        WcsTransform::from_header(&h).unwrap().orientation(naxis, naxis)
    }

    const EQUATORIAL_TAN: (&str, &str) = ("RA---TAN", "DEC--TAN");

    #[test]
    fn orientation_near_a_celestial_pole_points_east_along_the_local_tangent_for_both_parities() {
        let s = 1.0 / 3600.0;
        for dec in [89.9, 89.99, 89.999, 89.9995, -89.999] {
            for east_x in [-1.0, 1.0] {
                let o = centred_tan_orientation(EQUATORIAL_TAN, (150.0, dec), [[east_x * s, 0.0], [0.0, s]], 100);
                let what = format!("dec {dec}, CD1_1 sign {east_x}");
                assert_eq!(o.flipped, east_x > 0.0, "{what}");
                assert_vec_close(o.north_vec, (0.0, 1.0), &format!("north at {what}"));
                assert_vec_close(o.east_vec, (east_x, 0.0), &format!("east at {what}"));
            }
        }
    }

    #[test]
    fn orientation_of_wide_or_rotated_polar_fields_keeps_east_a_quarter_turn_from_north() {
        let wide = 10.0 / 3600.0;
        let o = centred_tan_orientation(EQUATORIAL_TAN, (150.0, 89.9), [[-wide, 0.0], [0.0, wide]], 2000);
        assert_vec_close(o.north_vec, (0.0, 1.0), "wide field north");
        assert_vec_close(o.east_vec, (-1.0, 0.0), "wide field east");

        let s = 1.0 / 3600.0;
        let (sn, cs) = 30f64.to_radians().sin_cos();
        let o = centred_tan_orientation(EQUATORIAL_TAN, (150.0, 89.99), [[-cs * s, -sn * s], [-sn * s, cs * s]], 100);
        assert_vec_close(o.north_vec, (-sn, cs), "rotated north");
        assert_vec_close(o.east_vec, (-cs, -sn), "rotated east");
    }

    #[test]
    fn orientation_of_a_galactic_frame_centred_near_the_equatorial_pole_keeps_icrs_east_perpendicular_to_icrs_north() {
        let s = 1.0 / 3600.0;
        let (l, b) = crate::core::astrometry::frames::icrs_to_galactic(150.0, 89.999);
        let o = centred_tan_orientation(("GLON-TAN", "GLAT-TAN"), (l, b), [[-s, 0.0], [0.0, s]], 100);
        let (nx, ny) = o.north_vec.expect("north");
        assert_close(nx.hypot(ny), 1.0, 1e-9, "north length");
        assert!(ny.abs() < 0.999, "ICRS north must be rotated against the galactic grid, got ({nx}, {ny})");
        assert_vec_close(o.east_vec, (-ny, nx), "east is north turned a quarter anticlockwise for normal parity");
    }
}

#[cfg(test)]
mod gwcs_tests {
    use std::sync::Arc;

    use super::*;
    use crate::core::astrometry::gwcs::test_support::{assert_close, assert_close_slice, fixture_pipeline, no_arrays, parse_real_syntax};
    use crate::core::astrometry::gwcs::{find_wcs_node, parse_gwcs, GwcsOrigin, GwcsSource};

    fn source(name: &str) -> Arc<GwcsSource> {
        Arc::new(GwcsSource {
            pipeline: fixture_pipeline(name),
            origin: GwcsOrigin::AsdfTree { key: "wcs".into() },
            wcsinfo_sip_max_err_px: None,
            wcsinfo_sip_inv_err_px: None,
        })
    }

    fn gwcs(name: &str) -> WcsTransform {
        WcsTransform::from_gwcs(source(name), &HduHeader::empty()).unwrap_or_else(|e| panic!("{name}: {e:#}"))
    }

    fn world_of(w: &WcsTransform, x: f64, y: f64) -> (f64, f64) {
        let c = w.pixel_to_world(x, y);
        (c.ra, c.dec)
    }

    fn header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in pairs {
            h.set(k, v.to_string());
        }
        h
    }

    const NRCA1_TAN_CARDS: &[(&str, &str)] = &[
        ("NAXIS1", "2048"),
        ("NAXIS2", "2048"),
        ("WCSAXES", "2"),
        ("RADESYS", "ICRS"),
        ("CRPIX1", "1024.5"),
        ("CRPIX2", "1024.5"),
        ("CRVAL1", "274.73480470116"),
        ("CRVAL2", "-13.86726301314"),
        ("CD1_1", "2.8415731816019e-07"),
        ("CD1_2", "8.6958723180969e-06"),
        ("CD2_1", "8.6435595374278e-06"),
        ("CD2_2", "-2.7788638413143e-07"),
    ];

    const NRCA1_SIP_CARDS: &[(&str, &str)] = &[
        ("CTYPE1", "RA---TAN-SIP"),
        ("CTYPE2", "DEC--TAN-SIP"),
        ("A_ORDER", "4"),
        ("A_0_2", "-1.1012423989127e-06"),
        ("A_0_3", "3.12630323322178e-11"),
        ("A_0_4", "-1.5779928405363e-14"),
        ("A_1_1", "-6.7550069826291e-06"),
        ("A_1_2", "4.06501013986234e-10"),
        ("A_1_3", "-2.4307858945383e-14"),
        ("A_2_0", "-8.9728303385234e-10"),
        ("A_2_1", "5.43919193588454e-11"),
        ("A_2_2", "-4.2683692001701e-14"),
        ("A_3_0", "2.95623303533868e-10"),
        ("A_3_1", "-1.037383801982e-14"),
        ("A_4_0", "-1.0006315789163e-14"),
        ("B_ORDER", "4"),
        ("B_0_2", "-4.6421696448153e-06"),
        ("B_0_3", "3.64570432075601e-10"),
        ("B_0_4", "-6.7467741902436e-14"),
        ("B_1_1", "1.1498127157042e-06"),
        ("B_1_2", "3.37399386167831e-11"),
        ("B_1_3", "2.41009443467561e-14"),
        ("B_2_0", "2.06972967564988e-06"),
        ("B_2_1", "3.40321949670535e-10"),
        ("B_2_2", "-6.3721976852566e-15"),
        ("B_3_0", "6.94269936499333e-12"),
        ("B_3_1", "-2.281984492617e-14"),
        ("B_4_0", "1.28023924015329e-14"),
        ("AP_ORDER", "4"),
        ("AP_0_2", "1.10128510854206e-06"),
        ("AP_0_3", "-1.240052641494e-11"),
        ("AP_0_4", "1.41149943466432e-14"),
        ("AP_1_1", "6.75507672034551e-06"),
        ("AP_1_2", "-3.3007652224429e-10"),
        ("AP_1_3", "1.32343787938799e-14"),
        ("AP_2_0", "9.68928669480131e-10"),
        ("AP_2_1", "-6.5480908452541e-11"),
        ("AP_2_2", "3.97571091366746e-14"),
        ("AP_3_0", "-3.0902076121121e-10"),
        ("AP_3_1", "1.29622940105603e-15"),
        ("AP_4_0", "1.001971521865e-14"),
        ("BP_ORDER", "4"),
        ("BP_0_2", "4.64237379659221e-06"),
        ("BP_0_3", "-3.1980656053782e-10"),
        ("BP_0_4", "5.91073727904483e-14"),
        ("BP_1_1", "-1.1497598054995e-06"),
        ("BP_1_2", "-6.2553902536975e-11"),
        ("BP_1_3", "-2.4197896127573e-14"),
        ("BP_2_0", "-2.0698085793808e-06"),
        ("BP_2_1", "-3.8605177770494e-10"),
        ("BP_2_2", "2.12234252800323e-16"),
        ("BP_3_0", "-4.3114987895992e-12"),
        ("BP_3_1", "2.43332959885544e-14"),
        ("BP_4_0", "-1.0710061740737e-14"),
    ];

    pub(crate) fn nrca1_sip_header() -> HduHeader {
        let mut cards = NRCA1_TAN_CARDS.to_vec();
        cards.extend_from_slice(NRCA1_SIP_CARDS);
        header(&cards)
    }

    fn nrca1_tan_header() -> HduHeader {
        let mut cards = NRCA1_TAN_CARDS.to_vec();
        cards.extend_from_slice(&[("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN")]);
        header(&cards)
    }

    const ROMAN_LIKE_CRVAL: (f64, f64) = (72.5, -30.7);
    const ROMAN_LIKE_NAXIS: usize = 4096;

    fn roman_like_body(cd: [[f64; 2]; 2]) -> String {
        let offset = -((ROMAN_LIKE_NAXIS as f64 - 1.0) / 2.0);
        format!(
            "wcs: !<tag:stsci.edu:gwcs/wcs-1.4.0>
  name: ''
  steps:
  - !<tag:stsci.edu:gwcs/step-1.3.0>
    frame: !<tag:stsci.edu:gwcs/frame2d-1.2.0>
      axes_names: [x, y]
      axes_order: [0, 1]
      axis_physical_types: ['custom:x', 'custom:y']
      name: detector
      unit: [!unit/unit-1.0.0 pixel, !unit/unit-1.0.0 pixel]
    transform: !transform/compose-1.4.0
      forward:
      - !transform/compose-1.4.0
        forward:
        - !transform/compose-1.4.0
          forward:
          - !transform/concatenate-1.4.0
            forward:
            - !transform/shift-1.4.0 {{inputs: [x], offset: {offset}, outputs: [y]}}
            - !transform/shift-1.4.0 {{inputs: [x], offset: {offset}, outputs: [y]}}
            inputs: [x0, x1]
            outputs: [y0, y1]
          - !transform/affine-1.4.0
            inputs: [x, y]
            matrix: !core/ndarray-1.1.0
              data: [[{}, {}], [{}, {}]]
              datatype: float64
              shape: [2, 2]
            outputs: [x, y]
            translation: !core/ndarray-1.1.0
              data: [0.0, 0.0]
              datatype: float64
              shape: [2]
          inputs: [x0, x1]
          outputs: [x, y]
        - !transform/gnomonic-1.4.0 {{direction: pix2sky, inputs: [x, y], outputs: [phi, theta]}}
        inputs: [x0, x1]
        outputs: [phi, theta]
      - !transform/rotate3d-1.5.0 {{direction: native2celestial, inputs: [phi_N, theta_N], outputs: [alpha_C, delta_C], phi: {}, psi: 180.0, theta: {}}}
      inputs: [x0, x1]
      outputs: [alpha_C, delta_C]
  - !<tag:stsci.edu:gwcs/step-1.3.0>
    frame: !<tag:stsci.edu:gwcs/celestial_frame-1.2.0>
      axes_names: [lon, lat]
      axes_order: [0, 1]
      axis_physical_types: [pos.eq.ra, pos.eq.dec]
      name: world
      reference_frame: !<tag:astropy.org:astropy/coordinates/frames/icrs-1.1.0>
        frame_attributes: {{}}
      unit: [!unit/unit-1.0.0 deg, !unit/unit-1.0.0 deg]
    transform: null
",
            cd[0][0], cd[0][1], cd[1][0], cd[1][1], ROMAN_LIKE_CRVAL.0, ROMAN_LIKE_CRVAL.1
        )
    }

    fn roman_like_pair(cd: [[f64; 2]; 2]) -> (WcsTransform, WcsTransform) {
        let tree = parse_real_syntax(&roman_like_body(cd));
        let (key, node) = find_wcs_node(&tree).expect("wcs node");
        let pipeline = parse_gwcs(node, &no_arrays, &key.0).unwrap_or_else(|e| panic!("{e:#}"));
        let src = Arc::new(GwcsSource {
            pipeline,
            origin: GwcsOrigin::AsdfTree { key: "roman.meta.wcs".into() },
            wcsinfo_sip_max_err_px: None,
            wcsinfo_sip_inv_err_px: None,
        });
        let naxis = ROMAN_LIKE_NAXIS.to_string();
        let dims = header(&[("NAXIS1", &naxis), ("NAXIS2", &naxis)]);
        let gwcs = WcsTransform::from_gwcs(src, &dims).unwrap_or_else(|e| panic!("{e:#}"));
        let mut fits = header(&[("NAXIS1", &naxis), ("NAXIS2", &naxis), ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN")]);
        let crpix = (ROMAN_LIKE_NAXIS as f64 + 1.0) / 2.0;
        for (key, value) in [
            ("CRPIX1", crpix),
            ("CRPIX2", crpix),
            ("CRVAL1", ROMAN_LIKE_CRVAL.0),
            ("CRVAL2", ROMAN_LIKE_CRVAL.1),
            ("CD1_1", cd[0][0]),
            ("CD1_2", cd[0][1]),
            ("CD2_1", cd[1][0]),
            ("CD2_2", cd[1][1]),
        ] {
            fits.set_f64(key, value);
        }
        (gwcs, WcsTransform::from_header(&fits).unwrap())
    }

    #[test]
    fn gwcs_surrogate_cd_has_the_header_orientation() {
        let header = nrca1_sip_header();
        let gwcs = WcsTransform::from_gwcs(source("wcs_jwst_nircam_cal300.asdf"), &header).unwrap().orientation(2048, 2048);
        let sip = WcsTransform::from_header(&header).unwrap().orientation(2048, 2048);
        assert!(!gwcs.flipped && !sip.flipped, "nrca1 is east-left: {gwcs:?}");
        assert_close(sip.rotation_deg, -91.83, 0.01, "header rotation atan2(-CD1_2, CD2_2)");
        assert_close(gwcs.rotation_deg, sip.rotation_deg, 0.5, "surrogate CD rotation vs the header CD");
        let (ge, he) = (gwcs.east_vec.unwrap(), sip.east_vec.unwrap());
        assert_close_slice(&[ge.0, ge.1], &[he.0, he.1], 0.01, "east vector of the surrogate vs the header (nrca1 has east up)");
        let (gn, hn) = (gwcs.north_vec.unwrap(), sip.north_vec.unwrap());
        assert_close_slice(&[gn.0, gn.1], &[hn.0, hn.1], 0.01, "north vector of the surrogate vs the header");

        let scale = 0.10931 / 3600.0;
        let (sin, cos) = 0.1f64.to_radians().sin_cos();
        let cd = [[-scale * cos, -scale * sin], [-scale * sin, scale * cos]];
        let (roman, fits) = roman_like_pair(cd);
        let n = ROMAN_LIKE_NAXIS;
        let (og, oh) = (roman.orientation(n, n), fits.orientation(n, n));
        assert!(!og.flipped && !oh.flipped, "Roman-like chain is east-left: {og:?}");
        assert_close(oh.rotation_deg, 0.1, 1e-9, "header rotation of the Roman-like CD");
        assert_close(og.rotation_deg, 0.1, 1e-6, "surrogate rotation of the Roman-like chain");
        assert_close(og.pixel_scale_x_arcsec, 0.10931, 1e-6, "Roman-like scale");
        assert!(og.east_vec.unwrap().0 < 0.0, "E left: {og:?}");
        let surrogate = roman.raw_params().4;
        for (got, exp) in surrogate.iter().flatten().zip(cd.iter().flatten()) {
            assert_close(*got, *exp, scale * 1e-6, "surrogate CD element");
        }
        for (x, y) in [(0.0, 0.0), (4095.0, 4095.0), (300.0, 3900.0), (2047.5, 2047.5)] {
            let (a, b) = (roman.pixel_to_world(x, y), fits.pixel_to_world(x, y));
            assert_close_slice(&[a.ra, a.dec], &[b.ra, b.dec], 1e-9, &format!("Roman-like chain vs TAN header at ({x}, {y})"));
        }
    }

    #[test]
    fn gwcs_backed_transform_matches_the_pipeline() {
        let nircam = gwcs("wcs_jwst_nircam_cal300.asdf");
        let centre = nircam.pixel_to_world(1023.5, 1023.5);
        assert_close_slice(&[centre.ra, centre.dec], &[274.7348047011612, -13.867263013140171], 1e-10, "NIRCam bbox centre");
        let (ra, dec) = world_of(&nircam, 0.0, 0.0);
        assert_close_slice(&[ra, dec], &[274.7253135310639, -13.875903855344635], 1e-10, "NIRCam (0, 0)");
        let (x, y) = nircam.world_to_pixel(ra, dec);
        assert_close_slice(&[x, y], &[0.0, 0.0], 1e-6, "NIRCam exact inverse of the (0, 0) world (the analytic start is 4.4e-4 px off)");

        let miri = gwcs("wcs_jwst_miri_cal300.asdf");
        let (ra, dec) = world_of(&miri, 0.0, 0.0);
        assert_close_slice(&[ra, dec], &[274.7271082813119, -13.841811218259549], 1e-10, "MIRI (0, 0)");
        let (x, y) = miri.world_to_pixel(ra, dec);
        assert_close_slice(&[x, y], &[0.0, 0.0], 1e-6, "MIRI exact inverse of the (0, 0) world (the analytic start is 0.11 px off)");

        let outside = nircam.pixel_to_world(-1.0, 0.0);
        assert!(outside.ra.is_finite() && outside.dec.is_finite(), "pixel_to_world extrapolates past the bbox: {outside:?}");
        let bounded = nircam.pixel_to_world_bounded(-1.0, 0.0);
        assert!(bounded.ra.is_nan() && bounded.dec.is_nan(), "pixel_to_world_bounded applies the bbox: {bounded:?}");
        let inside = nircam.pixel_to_world_bounded(-0.5, 2047.5);
        assert!(inside.ra.is_finite(), "the bbox edges are inside: {inside:?}");

        let (ra, dec) = world_of(&nircam, -3.0, 10.0);
        let (x, y) = nircam.world_to_pixel(ra, dec);
        assert_close_slice(&[x, y], &[-3.0, 10.0], 1e-6, "a start within 8 px of the box runs Newton and is not blanked");
        assert!(nircam.world_to_pixel(f64::NAN, dec).0.is_nan());
    }

    #[test]
    fn a_far_sky_point_is_nan_not_a_ghost() {
        let fgs = gwcs("wcs_jwst_fgs_cal300.asdf");
        let pipeline = fgs.gwcs_pipeline().expect("gWCS backend");
        let start = pipeline.backward_analytic(79.43014558211632, -69.59900426111466).unwrap();
        assert!(pipeline.in_bbox(start[0], start[1]), "the folded analytic start lies inside the box: {start:?}");
        let (x, y) = fgs.world_to_pixel(79.43014558211632, -69.59900426111466);
        let margin = GWCS_NEWTON_SKIP_MARGIN_PX;
        let ghost = x.is_finite() && y.is_finite() && x > -0.5 - margin && x < 2047.5 + margin && y > -0.5 - margin && y < 2047.5 + margin;
        assert!(!ghost, "a sky point 0.44 deg off the field must not land inside the image: ({x}, {y})");

        let (ra, dec) = world_of(&fgs, 1024.0, 2048.3);
        let (x, y) = fgs.world_to_pixel(ra, dec);
        assert_close_slice(&[x, y], &[1024.0, 2048.3], 1e-6, "1 px outside the top edge round-trips through Newton");

        let (ra, dec) = world_of(&fgs, 1024.0, 2070.0);
        let far_start = pipeline.backward_analytic(ra, dec).unwrap();
        assert!(far_start[1] > 2047.5 + margin, "{far_start:?}");
        let (x, y) = fgs.world_to_pixel(ra, dec);
        assert_eq!([x, y], far_start, "a start more than 8 px outside the box is returned without Newton");
    }

    #[test]
    fn gwcs_surrogate_cd_gives_the_plate_scale() {
        let nircam = gwcs("wcs_jwst_nircam_cal300.asdf");
        let scale = nircam.pixel_scale_arcsec();
        assert!((scale - 0.0312).abs() / 0.0312 < 0.005, "NIRCam SW scale {scale} arcsec/px");
        let (crpix1, crpix2, crval1, crval2, _, projection) = nircam.raw_params();
        assert_eq!((crpix1, crpix2), (1024.5, 1024.5), "surrogate CRPIX is the 1-based bbox centre");
        assert_close_slice(&[crval1, crval2], &[274.7348047011612, -13.867263013140171], 1e-10, "surrogate CRVAL");
        assert_eq!(projection, "gWCS");
        let o = nircam.orientation(2048, 2048);
        assert_eq!(o.projection, "gWCS");
        assert_eq!(o.wcs_kind, WcsKind::Gwcs);
        assert_eq!(nircam.kind(), WcsKind::Gwcs);
        assert!(!o.sip_present);
        let info = o.gwcs.as_ref().expect("gwcs info");
        assert_eq!(info.n_steps, 3);
        assert_eq!(info.frames, ["detector", "v2v3", "v2v3vacorr", "world"]);
        assert_eq!(info.source, "wcs");
        assert_eq!((info.vs_header_sip_max_mas, info.vs_header_sip_max_px), (None, None), "an empty header has no SIP to compare");
        assert!(o.north_vec.is_some() && o.east_vec.is_some(), "{o:?}");
        let (fov_w, fov_h) = nircam.field_of_view(2048, 2048);
        assert!((fov_w - 1.065).abs() < 0.02 && (fov_h - 1.065).abs() < 0.02, "fov {fov_w} x {fov_h} arcmin");
        assert_eq!(nircam.gwcs_refusal(), None);
    }

    #[test]
    fn gwcs_vs_header_sip_is_measured_on_a_9x9_grid() {
        let wcs = WcsTransform::from_gwcs(source("wcs_jwst_nircam_cal300.asdf"), &nrca1_sip_header()).unwrap();
        let info = wcs.gwcs_info().expect("gwcs info");
        let mas = info.vs_header_sip_max_mas.expect("the header carries a SIP to compare");
        assert_close(mas, 0.2661699147769568, 0.01, "max separation on the 9x9 grid (gwcs 1.0.3 vs astropy SIP)");
        let px = info.vs_header_sip_max_px.expect("px");
        assert_close(px, 0.0085, 0.0005, "the same in pixels at 31.2 mas/px");
        assert_eq!(wcs.sip_fit_residuals(), (None, None), "the real header has no SIPMXERR card");

        let tan = WcsTransform::from_gwcs(source("wcs_jwst_nircam_cal300.asdf"), &nrca1_tan_header()).unwrap();
        assert_eq!(tan.gwcs_info().unwrap().vs_header_sip_max_mas, None, "a header without SIP terms is not compared");
    }

    #[test]
    fn wcsinfo_residuals_fill_the_sip_fields() {
        let src = Arc::new(GwcsSource {
            pipeline: fixture_pipeline("wcs_jwst_nircam_cal300.asdf"),
            origin: GwcsOrigin::FitsAsdfHdu { hdu_index: 8 },
            wcsinfo_sip_max_err_px: Some(0.0087),
            wcsinfo_sip_inv_err_px: Some(0.0088),
        });
        let wcs = WcsTransform::from_gwcs(src, &HduHeader::empty()).unwrap();
        assert_eq!(wcs.sip_fit_residuals(), (Some(0.0087), Some(0.0088)));
        assert_eq!(wcs.gwcs_info().unwrap().source, "ASDF HDU 8");
        let o = wcs.orientation(2048, 2048);
        assert_eq!((o.sip_max_err_px, o.sip_inv_err_px), (Some(0.0087), Some(0.0088)));
    }

    #[test]
    fn a_refusal_is_kept_on_a_header_fallback() {
        let tan = header(&[
            ("NAXIS1", "64"),
            ("NAXIS2", "64"),
            ("CTYPE1", "RA---TAN"),
            ("CTYPE2", "DEC--TAN"),
            ("CRPIX1", "32.5"),
            ("CRPIX2", "32.5"),
            ("CRVAL1", "180.0"),
            ("CRVAL2", "45.0"),
            ("CDELT1", "-0.001"),
            ("CDELT2", "0.001"),
        ]);
        let refusal = "gWCS transform 'tabular' at steps[0].transform is not supported by the evaluator";
        let wcs = WcsTransform::from_header(&tan).unwrap().with_gwcs_refusal(refusal.to_string());
        assert_eq!(wcs.kind(), WcsKind::Header);
        assert_eq!(wcs.gwcs_refusal(), Some(refusal));
        assert!(wcs.gwcs_info().is_none());
        assert!(wcs.gwcs_pipeline().is_none());
        let o = wcs.orientation(64, 64);
        assert_eq!(o.wcs_kind, WcsKind::Header);
        assert!(o.gwcs.is_none());
        assert_eq!(WcsKind::Header.name(), "header");
        assert_eq!(WcsKind::Gwcs.name(), "gwcs");
        assert_eq!(serde_json::to_value(WcsKind::Gwcs).unwrap(), serde_json::json!("gwcs"));
    }
}
