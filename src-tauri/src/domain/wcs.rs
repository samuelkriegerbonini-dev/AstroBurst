use anyhow::{bail, Context, Result};

use crate::model::HduHeader;

#[derive(Debug, Clone)]
pub struct WcsTransform {
    crpix1: f64,
    crpix2: f64,
    crval1: f64,
    crval2: f64,
    cd: [[f64; 2]; 2],
    projection: Projection,
}

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
        let s = ((ra_h - h as f64) * 3600.0 - m as f64 * 60.0);

        let dec_sign = if self.dec >= 0.0 { "+" } else { "-" };
        let dec_abs = self.dec.abs();
        let d = dec_abs.floor() as u32;
        let dm = ((dec_abs - d as f64) * 60.0).floor() as u32;
        let ds = ((dec_abs - d as f64) * 3600.0 - dm as f64 * 60.0);

        write!(
            f,
            "{:02}h{:02}m{:05.2}s {}{}°{:02}'{:05.2}\"",
            h, m, s, dec_sign, d, dm, ds
        )
    }
}

impl WcsTransform {
    pub fn from_header(header: &HduHeader) -> Result<Self> {
        let crpix1 = header
            .get_f64("CRPIX1")
            .context("Missing CRPIX1")?;
        let crpix2 = header
            .get_f64("CRPIX2")
            .context("Missing CRPIX2")?;
        let crval1 = header
            .get_f64("CRVAL1")
            .context("Missing CRVAL1")?;
        let crval2 = header
            .get_f64("CRVAL2")
            .context("Missing CRVAL2")?;

        let cd = Self::read_cd_matrix(header)?;
        let projection = Self::detect_projection(header);

        Ok(WcsTransform {
            crpix1,
            crpix2,
            crval1,
            crval2,
            cd,
            projection,
        })
    }

    fn read_cd_matrix(header: &HduHeader) -> Result<[[f64; 2]; 2]> {
        if let (Some(cd11), Some(cd12), Some(cd21), Some(cd22)) = (
            header.get_f64("CD1_1"),
            header.get_f64("CD1_2"),
            header.get_f64("CD2_1"),
            header.get_f64("CD2_2"),
        ) {
            return Ok([[cd11, cd12], [cd21, cd22]]);
        }

        let cdelt1 = header
            .get_f64("CDELT1")
            .context("Missing CD matrix and CDELT1")?;
        let cdelt2 = header
            .get_f64("CDELT2")
            .context("Missing CD matrix and CDELT2")?;
        let crota2 = header.get_f64("CROTA2").unwrap_or(0.0);

        let theta = crota2.to_radians();
        let cos_t = theta.cos();
        let sin_t = theta.sin();

        Ok([
            [cdelt1 * cos_t, -cdelt2 * sin_t],
            [cdelt1 * sin_t, cdelt2 * cos_t],
        ])
    }

    fn detect_projection(header: &HduHeader) -> Projection {
        let ctype1 = header.get("CTYPE1").unwrap_or("");
        let suffix = if ctype1.len() >= 8 {
            &ctype1[5..8]
        } else if ctype1.len() > 4 {
            &ctype1[ctype1.len() - 3..]
        } else {
            "TAN"
        };

        match suffix {
            "TAN" => Projection::Tan,
            "SIN" => Projection::Sin,
            "ARC" => Projection::Arc,
            "CAR" => Projection::Car,
            _ => Projection::Tan,
        }
    }

    pub fn pixel_to_world(&self, x: f64, y: f64) -> CelestialCoord {
        let dx = x - self.crpix1 + 1.0;
        let dy = y - self.crpix2 + 1.0;

        let xi = self.cd[0][0] * dx + self.cd[0][1] * dy;
        let eta = self.cd[1][0] * dx + self.cd[1][1] * dy;

        self.deproject(xi, eta)
    }

    pub fn world_to_pixel(&self, ra: f64, dec: f64) -> (f64, f64) {
        let (xi, eta) = self.project(ra, dec);

        let det = self.cd[0][0] * self.cd[1][1] - self.cd[0][1] * self.cd[1][0];
        if det.abs() < 1e-30 {
            return (f64::NAN, f64::NAN);
        }

        let inv_det = 1.0 / det;
        let dx = inv_det * (self.cd[1][1] * xi - self.cd[0][1] * eta);
        let dy = inv_det * (-self.cd[1][0] * xi + self.cd[0][0] * eta);

        (dx + self.crpix1 - 1.0, dy + self.crpix2 - 1.0)
    }

    fn deproject(&self, xi_deg: f64, eta_deg: f64) -> CelestialCoord {
        let xi = xi_deg.to_radians();
        let eta = eta_deg.to_radians();
        let ra0 = self.crval1.to_radians();
        let dec0 = self.crval2.to_radians();

        let (ra, dec) = match self.projection {
            Projection::Tan => {
                let denom = dec0.cos() - eta * dec0.sin();
                let ra = ra0 + xi.atan2(denom);
                let dec = (dec0.sin() + eta * dec0.cos())
                    .atan2((xi * xi + denom * denom).sqrt());
                (ra, dec)
            }
            Projection::Sin => {
                let cos_c = (1.0 - xi * xi - eta * eta).max(0.0).sqrt();
                let dec = (cos_c * dec0.sin() + eta * dec0.cos()).asin();
                let ra = ra0 + (xi).atan2(cos_c * dec0.cos() - eta * dec0.sin());
                (ra, dec)
            }
            Projection::Arc => {
                let rho = (xi * xi + eta * eta).sqrt();
                if rho < 1e-15 {
                    (ra0, dec0)
                } else {
                    let c = rho;
                    let dec = (c.cos() * dec0.sin() + (eta / rho) * c.sin() * dec0.cos()).asin();
                    let ra = ra0
                        + (xi * c.sin())
                            .atan2(rho * dec0.cos() * c.cos() - eta * dec0.sin() * c.sin());
                    (ra, dec)
                }
            }
            Projection::Car => {
                let ra = ra0 + xi / dec0.cos();
                let dec = dec0 + eta;
                (ra, dec)
            }
        };

        let mut ra_deg = ra.to_degrees();
        if ra_deg < 0.0 {
            ra_deg += 360.0;
        }
        if ra_deg >= 360.0 {
            ra_deg -= 360.0;
        }

        CelestialCoord {
            ra: ra_deg,
            dec: dec.to_degrees(),
        }
    }

    fn project(&self, ra: f64, dec: f64) -> (f64, f64) {
        let ra_r = ra.to_radians();
        let dec_r = dec.to_radians();
        let ra0 = self.crval1.to_radians();
        let dec0 = self.crval2.to_radians();

        let delta_ra = ra_r - ra0;

        match self.projection {
            Projection::Tan => {
                let denom =
                    dec_r.sin() * dec0.sin() + dec_r.cos() * dec0.cos() * delta_ra.cos();
                if denom.abs() < 1e-15 {
                    return (f64::NAN, f64::NAN);
                }
                let xi = (dec_r.cos() * delta_ra.sin()) / denom;
                let eta =
                    (dec_r.sin() * dec0.cos() - dec_r.cos() * dec0.sin() * delta_ra.cos()) / denom;
                (xi.to_degrees(), eta.to_degrees())
            }
            Projection::Sin => {
                let xi = dec_r.cos() * delta_ra.sin();
                let eta =
                    dec_r.sin() * dec0.cos() - dec_r.cos() * dec0.sin() * delta_ra.cos();
                (xi.to_degrees(), eta.to_degrees())
            }
            Projection::Arc => {
                let cos_c =
                    dec_r.sin() * dec0.sin() + dec_r.cos() * dec0.cos() * delta_ra.cos();
                let c = cos_c.clamp(-1.0, 1.0).acos();
                if c.abs() < 1e-15 {
                    return (0.0, 0.0);
                }
                let k = c / c.sin();
                let xi = k * dec_r.cos() * delta_ra.sin();
                let eta = k
                    * (dec_r.sin() * dec0.cos() - dec_r.cos() * dec0.sin() * delta_ra.cos());
                (xi.to_degrees(), eta.to_degrees())
            }
            Projection::Car => {
                let xi = delta_ra * dec0.cos();
                let eta = dec_r - dec0;
                (xi.to_degrees(), eta.to_degrees())
            }
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
        coords
            .iter()
            .map(|&(x, y)| self.pixel_to_world(x, y))
            .collect()
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
        let h = make_header(&[
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
        assert!((coord.ra - 180.0).abs() < 1e-6);
        assert!((coord.dec - 45.0).abs() < 1e-6);
    }

    #[test]
    fn test_roundtrip_tan() {
        let h = make_header(&[
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

        let coord = wcs.pixel_to_world(150.0, 200.0);
        let (px, py) = wcs.world_to_pixel(coord.ra, coord.dec);
        assert!((px - 150.0).abs() < 1e-3);
        assert!((py - 200.0).abs() < 1e-3);
    }

    #[test]
    fn test_crpix_center() {
        let h = make_header(&[
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
        let c = CelestialCoord {
            ra: 83.633,
            dec: 22.014,
        };
        let s = format!("{}", c);
        assert!(s.contains("h"));
        assert!(s.contains("°"));
    }
}
