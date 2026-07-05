// WCS engine migration to the CDS wcs-rs crate (full FITS projection coverage, wrapper-side SIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use serde_json::{Map, Number, Value};
use wcs::{ImgXY, LonLat, WCSParams};

use crate::types::header::HduHeader;

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

    pub fn terms(&self) -> &[(i32, i32, f64)] {
        &self.terms
    }
}

/// Legacy projection-code enum, retained for API stability.
///
/// The engine (wcs-rs) actually supports ~20 projections; `WcsTransform::raw_params()`
/// returns the projection as a `&str` so codes beyond these four (e.g. "AIT", "ZEA")
/// flow through correctly. This enum is kept only so existing callers matching on it
/// keep compiling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Projection {
    Tan,
    Sin,
    Arc,
    Car,
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

/// Funnels all pixel<->sky WCS math for the app through a single type. The linear
/// (CD/PC/CDELT) transform and celestial projection/deprojection are delegated to
/// the `wcs` crate (wcs-rs + mapproj); SIP distortion is applied by this wrapper
/// itself (see `SipPoly`'s doc comment for why).
#[derive(Debug)]
pub struct WcsTransform {
    /// Always constructed with any `-SIP` suffix stripped from CTYPE1/2, so its
    /// internal (buggy) SIP path never engages -- see `SipPoly`.
    engine: wcs::WCS,
    crpix1: f64,
    crpix2: f64,
    crval1: f64,
    crval2: f64,
    /// Effective CD matrix (from CD, or PC*CDELT, or CDELT+CROTA2 — same priority
    /// and defaults wcs-rs itself uses) kept only for `pixel_scale_arcsec`/
    /// `field_of_view`/`raw_params`; the engine does the actual coordinate math.
    cd: [[f64; 2]; 2],
    /// CTYPE1 projection code (e.g. "TAN", "AIT"), parsed independently of wcs-rs's
    /// stricter internal parsing so it's always populated for display/serialization.
    projection: String,
    sip_a: Option<SipPoly>,
    sip_b: Option<SipPoly>,
    sip_ap: Option<SipPoly>,
    sip_bp: Option<SipPoly>,
}

/// Returns true if `key` (already uppercased) is a FITS keyword forwarded to
/// `wcs::WCSParams`. This is a whitelist, not a pass-through of the whole header:
/// ASDF ingestion (`infra/asdf_bridge.rs`) injects arbitrary uppercased metadata
/// cards into the header, and `serde_json::from_value::<WCSParams>` hard-fails the
/// moment a string-valued card collides with a numeric field. Restricting to known
/// WCSParams keys keeps the bridge total.
fn is_wcs_param_key(key: &str) -> bool {
    matches!(
        key,
        "NAXIS"
            | "NAXIS1"
            | "NAXIS2"
            | "NAXIS3"
            | "NAXIS4"
            | "ZNAXIS1"
            | "ZNAXIS2"
            | "ZNAXIS3"
            | "ZNAXIS4"
            | "CRPIX1"
            | "CRPIX2"
            | "CRPIX3"
            | "CRVAL1"
            | "CRVAL2"
            | "CRVAL3"
            | "CDELT1"
            | "CDELT2"
            | "CDELT3"
            | "CROTA1"
            | "CROTA2"
            | "CROTA3"
            | "CTYPE1"
            | "CTYPE2"
            | "CTYPE3"
            | "EPOCH"
            | "EQUINOX"
            | "RADESYS"
            | "LONPOLE"
            | "LATPOLE"
            | "A_ORDER"
            | "B_ORDER"
            | "AP_ORDER"
            | "BP_ORDER"
    ) || is_matrix_key(key, "CD")
        || is_matrix_key(key, "PC")
        || key.starts_with("PV1_")
        || key.starts_with("PV2_")
        || is_sip_coeff_key(key, "AP_")
        || is_sip_coeff_key(key, "BP_")
        || is_sip_coeff_key(key, "A_")
        || is_sip_coeff_key(key, "B_")
}

/// Matches e.g. `CD1_1`..`CD3_3` / `PC1_1`..`PC3_3`: prefix + digit + '_' + digit.
fn is_matrix_key(key: &str, prefix: &str) -> bool {
    let bytes = key.as_bytes();
    key.len() == prefix.len() + 3
        && key.starts_with(prefix)
        && bytes[prefix.len()].is_ascii_digit()
        && bytes[prefix.len() + 1] == b'_'
        && bytes[prefix.len() + 2].is_ascii_digit()
}

/// Matches e.g. `A_2_0`, `AP_1_3`: prefix + digit + '_' + digit (excludes `*_ORDER`,
/// matched separately above; note this must be checked against the `AP_`/`BP_`
/// prefixes before `A_`/`B_` since e.g. "AP_1_0" also starts with "A"... but not "A_").
fn is_sip_coeff_key(key: &str, prefix: &str) -> bool {
    let rest = match key.strip_prefix(prefix) {
        Some(r) => r,
        None => return false,
    };
    let mut parts = rest.split('_');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(p), Some(q), None) => p.parse::<u32>().is_ok() && q.parse::<u32>().is_ok(),
        _ => false,
    }
}

/// Coerces a raw FITS card string into the JSON representation `WCSParams` needs
/// for `key`, and inserts it into `map`. Non-numeric values for numeric-only keys
/// are dropped rather than forwarded as strings (a stray unparsable value should
/// fall back to `WCSParams`' own `Option::None` default, not fail deserialization).
fn insert_wcs_card(map: &mut Map<String, Value>, key: &str, raw: &str) {
    let trimmed = raw.trim().trim_matches('\'').trim();

    let is_string_key = matches!(key, "CTYPE1" | "CTYPE2" | "CTYPE3" | "RADESYS");
    let is_int_key = matches!(
        key,
        "NAXIS" | "NAXIS1" | "NAXIS2" | "NAXIS3" | "NAXIS4"
            | "ZNAXIS1" | "ZNAXIS2" | "ZNAXIS3" | "ZNAXIS4"
            | "A_ORDER" | "B_ORDER" | "AP_ORDER" | "BP_ORDER"
    );

    if is_string_key {
        map.insert(key.to_string(), Value::String(trimmed.to_string()));
        return;
    }

    if is_int_key {
        if let Ok(i) = trimmed.parse::<i64>() {
            map.insert(key.to_string(), Value::Number(i.into()));
        } else if let Ok(f) = trimmed.parse::<f64>() {
            map.insert(key.to_string(), Value::Number((f.round() as i64).into()));
        }
        return;
    }

    if let Ok(f) = trimmed.parse::<f64>() {
        if let Some(n) = Number::from_f64(f) {
            map.insert(key.to_string(), Value::Number(n));
        }
    }
}

/// Parses the CTYPE1 projection code the same lenient way the old hand-rolled
/// code did (last '-'-separated segment, with any "-SIP" suffix stripped first),
/// but returns the raw string rather than mapping onto a 4-variant enum — so
/// `raw_params()` correctly reports any of wcs-rs's ~20 supported codes.
fn projection_code(ctype1: &str) -> String {
    let base = ctype1.trim().trim_end_matches("-SIP");
    base.rsplit('-').next().unwrap_or("TAN").to_string()
}

/// Synthesizes the effective CD matrix used for `pixel_scale_arcsec`/`field_of_view`,
/// mirroring wcs-rs's own construction priority and defaults exactly (CD > PC+CDELT
/// > CDELT+CROTA2) so these numbers stay consistent with what the engine transforms.
fn read_effective_cd(header: &HduHeader) -> Result<[[f64; 2]; 2]> {
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
        let pc11 = pc11.unwrap_or(1.0);
        let pc12 = pc12.unwrap_or(0.0);
        let pc21 = pc21.unwrap_or(0.0);
        let pc22 = pc22.unwrap_or(1.0);
        return Ok([
            [pc11 * cdelt1, pc12 * cdelt2],
            [pc21 * cdelt1, pc22 * cdelt2],
        ]);
    }

    let cdelt1 = header
        .get_f64("CDELT1")
        .context("Missing CD matrix, PC matrix, and CDELT1")?;
    let cdelt2 = header
        .get_f64("CDELT2")
        .context("Missing CD matrix, PC matrix, and CDELT2")?;
    let crota2 = header.get_f64("CROTA2").unwrap_or(0.0);
    let theta = crota2.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();

    Ok([
        [cdelt1 * cos_t, -cdelt2 * sin_t],
        [cdelt1 * sin_t, cdelt2 * cos_t],
    ])
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
        let det = cd[0][0] * cd[1][1] - cd[0][1] * cd[1][0];
        if !det.is_finite() || det.abs() < 1e-30 {
            bail!("Singular or non-finite CD matrix");
        }

        // wcs-rs slices CTYPE1[5..=7] internally to find the projection code
        // without bounds-checking; guard the minimum FITS-standard length here so
        // a malformed header returns an error instead of panicking (this crate's
        // release profile uses `panic = "abort"`, which would kill the whole app).
        let ctype1 = header.get("CTYPE1").unwrap_or("RA---TAN").trim();
        if ctype1.len() < 8 {
            bail!(
                "CTYPE1 '{}' is shorter than the minimum 8-character FITS WCS keyword",
                ctype1
            );
        }
        let projection = projection_code(ctype1);

        let naxis1 = header.get_i64("NAXIS1").context("Missing NAXIS1")?;
        let naxis2 = header.get_i64("NAXIS2").context("Missing NAXIS2")?;

        let mut map: Map<String, Value> = Map::new();
        for (key, val) in &header.cards {
            let ku = key.to_uppercase();
            if is_wcs_param_key(&ku) {
                insert_wcs_card(&mut map, &ku, val);
            }
        }
        // WcsTransform only ever does 2D celestial pix<->sky (matching today's
        // behavior of ignoring any 3rd axis), so force NAXIS=2 regardless of the
        // file's real dimensionality (e.g. a data cube's NAXIS=3).
        map.insert("NAXIS".into(), Value::Number(2.into()));
        map.insert("NAXIS1".into(), Value::Number(naxis1.into()));
        map.insert("NAXIS2".into(), Value::Number(naxis2.into()));
        // Always strip any "-SIP" suffix before handing CTYPE to the engine: SIP
        // is applied by this wrapper, never by wcs-rs (see `SipPoly`'s doc comment
        // for why). This also means A_/B_/AP_/BP_ keys forwarded into `map` above
        // are harmless no-ops as far as the engine is concerned.
        map.insert(
            "CTYPE1".into(),
            Value::String(ctype1.trim_end_matches("-SIP").to_string()),
        );
        if let Some(ctype2) = header.get("CTYPE2") {
            map.insert(
                "CTYPE2".into(),
                Value::String(ctype2.trim().trim_end_matches("-SIP").to_string()),
            );
        }

        let params: WCSParams = serde_json::from_value(Value::Object(map))
            .context("Building WCSParams from header")?;
        let engine =
            wcs::WCS::new(&params).map_err(|e| anyhow::anyhow!("wcs::WCS::new failed: {e}"))?;

        Ok(WcsTransform {
            engine,
            crpix1,
            crpix2,
            crval1,
            crval2,
            cd,
            projection,
            sip_a: SipPoly::parse(header, "A"),
            sip_b: SipPoly::parse(header, "B"),
            sip_ap: SipPoly::parse(header, "AP"),
            sip_bp: SipPoly::parse(header, "BP"),
        })
    }

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

    pub fn sip_forward_terms(&self) -> (Option<&SipPoly>, Option<&SipPoly>) {
        (self.sip_a.as_ref(), self.sip_b.as_ref())
    }

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

    /// Pixel-to-sky. `x`/`y` are 0-based image-array pixel coordinates; wcs-rs uses
    /// the FITS 1-based convention (`ImgXY == CRPIX` maps to `CRVAL`), hence `+1.0`.
    /// SIP (if any) is applied here, in image space, before handing the corrected
    /// position to the engine -- the engine's own CTYPE never carries a `-SIP`
    /// suffix (see `SipPoly`'s doc comment), so its `dx = xy.x - crpix1` recovers
    /// our already-corrected `u` unchanged when we pass `ImgXY::new(u + crpix1, ...)`.
    pub fn pixel_to_world(&self, x: f64, y: f64) -> CelestialCoord {
        let dx = x - self.crpix1 + 1.0;
        let dy = y - self.crpix2 + 1.0;
        let (u, v) = self.sip_forward(dx, dy);

        match self
            .engine
            .unproj(&ImgXY::new(u + self.crpix1, v + self.crpix2))
        {
            Some(ll) => CelestialCoord {
                ra: ll.lon().to_degrees().rem_euclid(360.0),
                dec: ll.lat().to_degrees(),
            },
            None => CelestialCoord {
                ra: f64::NAN,
                dec: f64::NAN,
            },
        }
    }

    /// Sky-to-pixel. Returns 0-based image-array pixel coordinates (see
    /// `pixel_to_world`). Inverts SIP (if any) on the engine's purely-linear result.
    pub fn world_to_pixel(&self, ra: f64, dec: f64) -> (f64, f64) {
        let ll = LonLat::new(ra.to_radians(), dec.to_radians());
        match self.engine.proj(&ll) {
            Some(xy) => {
                let u_lin = xy.x() - self.crpix1;
                let v_lin = xy.y() - self.crpix2;
                let (dx, dy) = self.sip_inverse(u_lin, v_lin);
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
        HduHeader { cards, index }
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
        for (ctype, expected) in [
            ("RA---TAN", "TAN"),
            ("RA---SIN", "SIN"),
            ("RA---ARC", "ARC"),
            ("RA---CAR", "CAR"),
            ("GLON-TAN", "TAN"),
            ("RA---TAN-SIP", "TAN"),
            ("RA---SIN-SIP", "SIN"),
            ("RA---AIT", "AIT"),
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
                ("CTYPE2", "DEC--TAN"),
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
        // The old hand-rolled `read_cd_matrix` only ever read CD1_1.. or fell back
        // to CDELT+CROTA2 -- it silently ignored the PC matrix entirely (a real bug
        // for ASDF/Roman-derived headers, which emit PC + CDELT, never CD). wcs-rs
        // honors PC; assert a non-identity PC matrix actually changes the result
        // versus the CDELT-only (PC-less) fallback, locking the fix in.
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

    /// End-to-end check against the repo's real test fixture (a JWST-style header:
    /// PC matrix + CDELT, no CD; order-3 SIP with a real fitted AP/BP inverse) --
    /// exactly the combination most at risk in this migration (PC handling, SIP
    /// correctness). `#[ignore]`d since it depends on a filesystem path outside
    /// this crate rather than an inline fixture like the tests above; run
    /// explicitly with `cargo test -- --ignored`.
    #[test]
    #[ignore = "depends on the repo-relative test_data/test.fits fixture; run explicitly"]
    fn test_real_fixture_end_to_end() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test_data/test.fits");
        let file = std::fs::File::open(path).expect("open test.fits");
        let result = crate::infra::fits::reader::extract_image_mmap(&file).expect("extract");
        let wcs = WcsTransform::from_header(&result.header).expect("from_header");

        let naxis1 = result.header.get_i64("NAXIS1").unwrap();
        let naxis2 = result.header.get_i64("NAXIS2").unwrap();
        eprintln!("naxis: {naxis1} x {naxis2}");

        for &(x, y) in &[
            (0.0, 0.0),
            (naxis1 as f64 / 2.0, naxis2 as f64 / 2.0),
            (naxis1 as f64 - 1.0, naxis2 as f64 - 1.0),
        ] {
            let coord = wcs.pixel_to_world(x, y);
            let (px, py) = wcs.world_to_pixel(coord.ra, coord.dec);
            eprintln!(
                "px=({x},{y}) -> ra={:.8} dec={:.8} -> roundtrip=({:.4},{:.4}) [{}]",
                coord.ra,
                coord.dec,
                px,
                py,
                coord.ra.is_finite() && coord.dec.is_finite()
            );
            assert!(coord.ra.is_finite() && coord.dec.is_finite());
            // Real AP/BP coefficients are only an approximate fitted inverse (not
            // exact like the synthetic test headers above), so this is looser than
            // the synthetic-header tolerances -- just confirming it's in the right
            // ballpark, not analytically exact.
            assert!((px - x).abs() < 2.0, "px roundtrip off at ({x},{y}): got {px}");
            assert!((py - y).abs() < 2.0, "py roundtrip off at ({x},{y}): got {py}");
        }

        eprintln!("pixel_scale_arcsec = {}", wcs.pixel_scale_arcsec());
        eprintln!("raw_params = {:?}", wcs.raw_params());
    }
}
