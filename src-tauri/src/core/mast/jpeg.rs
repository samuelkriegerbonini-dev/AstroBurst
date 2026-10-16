use super::types::{MastQuickWcs, WcsOrientation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
}

impl ImageKind {
    pub const fn extension(self) -> &'static str {
        match self {
            ImageKind::Jpeg => "jpg",
            ImageKind::Png => "png",
        }
    }
}

pub const SNIFF_LEN: usize = 8;

const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];
const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

pub fn sniff(bytes: &[u8]) -> Option<ImageKind> {
    if bytes.starts_with(&JPEG_MAGIC) {
        Some(ImageKind::Jpeg)
    } else if bytes.starts_with(&PNG_MAGIC) {
        Some(ImageKind::Png)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuickWcs {
    pub ctype: String,
    pub crpix: [f64; 2],
    pub crval: [f64; 2],
    pub cd: [[f64; 2]; 2],
    pub from_cd: bool,
}

const MARKER_SOS: u8 = 0xDA;
const MARKER_EOI: u8 = 0xD9;
const MARKER_COM: u8 = 0xFE;
const CARD_WIDTH: usize = 80;

fn is_frame_marker(marker: u8) -> bool {
    (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

fn is_standalone(marker: u8) -> bool {
    marker == 0x01 || (0xD0..=0xD7).contains(&marker)
}

fn jpeg_segments<'a>(bytes: &'a [u8], mut visit: impl FnMut(u8, &'a [u8]) -> bool) {
    if !bytes.starts_with(&JPEG_MAGIC[..2]) {
        return;
    }
    let mut i = 2;
    while i < bytes.len() {
        if bytes[i] != 0xFF {
            return;
        }
        while i < bytes.len() && bytes[i] == 0xFF {
            i += 1;
        }
        let Some(&marker) = bytes.get(i) else { return };
        i += 1;
        if is_standalone(marker) {
            continue;
        }
        if marker == MARKER_SOS || marker == MARKER_EOI {
            return;
        }
        let Some(length) = bytes.get(i..i + 2).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]]))) else { return };
        if length < 2 || i + length > bytes.len() {
            return;
        }
        if !visit(marker, &bytes[i + 2..i + length]) {
            return;
        }
        i += length;
    }
}

fn channels_for(components: u8) -> u8 {
    if components >= 3 {
        3
    } else {
        1
    }
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32, u8)> {
    if bytes.len() < 26 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    let channels = match bytes[25] {
        0 | 4 => 1,
        2 | 3 | 6 => 3,
        _ => return None,
    };
    (width > 0 && height > 0).then_some((width, height, channels))
}

pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32, u8)> {
    match sniff(bytes)? {
        ImageKind::Png => png_dimensions(bytes),
        ImageKind::Jpeg => {
            let mut found = None;
            jpeg_segments(bytes, |marker, payload| {
                if !is_frame_marker(marker) {
                    return true;
                }
                if payload.len() >= 6 {
                    let height = u32::from(u16::from_be_bytes([payload[1], payload[2]]));
                    let width = u32::from(u16::from_be_bytes([payload[3], payload[4]]));
                    if width > 0 && height > 0 {
                        found = Some((width, height, channels_for(payload[5])));
                    }
                }
                false
            });
            found
        }
    }
}

pub fn comment(bytes: &[u8]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    jpeg_segments(bytes, |marker, payload| {
        if marker == MARKER_COM {
            parts.push(String::from_utf8_lossy(payload).into_owned());
        }
        true
    });
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn card_lines(text: &str) -> Vec<&str> {
    if text.contains('\n') {
        return text.lines().collect();
    }
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut cut = rest.len().min(CARD_WIDTH);
        while !rest.is_char_boundary(cut) {
            cut -= 1;
        }
        let (line, tail) = rest.split_at(cut);
        lines.push(line);
        rest = tail;
    }
    lines
}

fn card_value(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(quoted) = raw.strip_prefix('\'') {
        return quoted.split('\'').next().unwrap_or_default().trim_end().to_owned();
    }
    raw.split('/').next().unwrap_or_default().trim().to_owned()
}

fn parse_cards(text: &str) -> std::collections::HashMap<String, String> {
    card_lines(text)
        .into_iter()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            (!key.is_empty() && !key.contains(' ')).then(|| (key.to_ascii_uppercase(), card_value(value)))
        })
        .collect()
}

fn number(cards: &std::collections::HashMap<String, String>, key: &str) -> Option<f64> {
    let text = cards.get(key)?.replace(['D', 'd'], "E");
    text.trim().parse::<f64>().ok().filter(|value| value.is_finite())
}

pub fn quick_wcs(bytes: &[u8]) -> Option<QuickWcs> {
    let cards = parse_cards(&comment(bytes)?);
    let crpix = [number(&cards, "CRPIX1")?, number(&cards, "CRPIX2")?];
    let crval = [number(&cards, "CRVAL1")?, number(&cards, "CRVAL2")?];
    let ctype = cards.get("CTYPE1").cloned().unwrap_or_default();
    let cd_keys = ["CD1_1", "CD1_2", "CD2_1", "CD2_2"];
    if cd_keys.iter().any(|key| number(&cards, key).is_some()) {
        let [a, b, c, d] = cd_keys.map(|key| number(&cards, key).unwrap_or(0.0));
        return Some(QuickWcs { ctype, crpix, crval, cd: [[a, b], [c, d]], from_cd: true });
    }
    let cdelt1 = number(&cards, "CDELT1")?;
    let cdelt2 = number(&cards, "CDELT2")?;
    let (sin, cos) = number(&cards, "CROTA2").unwrap_or(0.0).to_radians().sin_cos();
    Some(QuickWcs {
        ctype,
        crpix,
        crval,
        cd: [[cdelt1 * cos, -cdelt2 * sin], [cdelt1 * sin, cdelt2 * cos]],
        from_cd: false,
    })
}

pub fn scale_wcs(wcs: QuickWcs, factor: f64) -> QuickWcs {
    QuickWcs {
        crpix: wcs.crpix.map(|value| value / factor),
        cd: wcs.cd.map(|row| row.map(|value| value * factor)),
        ..wcs
    }
}

fn normalize_ra(ra: f64) -> f64 {
    let wrapped = ra.rem_euclid(360.0);
    if wrapped >= 360.0 {
        0.0
    } else {
        wrapped
    }
}

impl QuickWcs {
    pub fn pixel_to_world(&self, x: f64, y: f64) -> (f64, f64) {
        let dx = x - self.crpix[0];
        let dy = y - self.crpix[1];
        let xi = self.cd[0][0] * dx + self.cd[0][1] * dy;
        let eta = self.cd[1][0] * dx + self.cd[1][1] * dy;
        if !self.ctype.to_ascii_uppercase().contains("-TAN") {
            let cos_dec = self.crval[1].to_radians().cos();
            let ra = if cos_dec.abs() > 1e-12 { self.crval[0] + xi / cos_dec } else { self.crval[0] };
            return (normalize_ra(ra), self.crval[1] + eta);
        }
        let (xi, eta) = (xi.to_radians(), eta.to_radians());
        let (sin_dec0, cos_dec0) = self.crval[1].to_radians().sin_cos();
        let denominator = cos_dec0 - eta * sin_dec0;
        let ra = self.crval[0].to_radians() + xi.atan2(denominator);
        let dec = (eta * cos_dec0 + sin_dec0).atan2(xi.hypot(denominator));
        (normalize_ra(ra.to_degrees()), dec.to_degrees())
    }

    pub fn summary(&self, width: u32, height: u32) -> Option<MastQuickWcs> {
        let det = self.cd[0][0] * self.cd[1][1] - self.cd[0][1] * self.cd[1][0];
        if !det.is_finite() || det == 0.0 || width == 0 || height == 0 {
            return None;
        }
        let scale_deg = det.abs().sqrt();
        let (centre_ra_deg, centre_dec_deg) = self.pixel_to_world((f64::from(width) + 1.0) / 2.0, (f64::from(height) + 1.0) / 2.0);
        if !centre_ra_deg.is_finite() || !centre_dec_deg.is_finite() {
            return None;
        }
        Some(MastQuickWcs {
            centre_ra_deg,
            centre_dec_deg,
            pixel_scale_arcsec: scale_deg * 3600.0,
            field_arcmin: [f64::from(width) * scale_deg * 60.0, f64::from(height) * scale_deg * 60.0],
            orientation: if self.from_cd { WcsOrientation::CdMatrix } else { WcsOrientation::Unreliable },
            ctype: self.ctype.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mast").join(name)).unwrap()
    }

    fn close(actual: f64, expected: f64, tolerance: f64) -> bool {
        (actual - expected).abs() <= tolerance
    }

    fn png_header(width: u32, height: u32, colour_type: u8) -> Vec<u8> {
        let mut bytes = PNG_MAGIC.to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, colour_type, 0, 0, 0]);
        bytes
    }

    #[test]
    fn sniff_jpeg_png_and_reject_html() {
        assert_eq!(sniff(&fixture("t_clear-f187n.bin")), Some(ImageKind::Jpeg));
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]), Some(ImageKind::Png));
        assert_eq!(sniff(&fixture("t_bad.bin")), None);
        assert_eq!(sniff(b"<html><body>404</body></html>"), None);
        assert_eq!(sniff(&[0xFF, 0xD8]), None);
        assert_eq!(sniff(&[]), None);
    }

    #[test]
    fn jpeg_dimensions_from_sof() {
        assert_eq!(dimensions(&fixture("t_clear-f187n.bin")), Some((128, 76, 1)));
        assert_eq!(dimensions(&fixture("thumb_hst.bin")), Some((117, 128, 1)));
        assert_eq!(dimensions(&fixture("t_cal_1.bin")), Some((128, 128, 1)));
        assert_eq!(dimensions(&fixture("prev_jw_head_sos.bin")), Some((7065, 4177, 1)));
        assert_eq!(dimensions(&fixture("prev_hst_head_sos.bin")), Some((4253, 4659, 1)));
        assert_eq!(dimensions(&fixture("hpL_hst_15238_20_wfc3_ir_total_idnm20_drz_color_sos.bin")), Some((2494, 2908, 3)));
        assert_eq!(dimensions(&png_header(640, 480, 2)), Some((640, 480, 3)));
        assert_eq!(dimensions(&png_header(10, 20, 0)), Some((10, 20, 1)));
        assert_eq!(dimensions(&png_header(10, 20, 6)), Some((10, 20, 3)));
        assert_eq!(dimensions(&png_header(0, 20, 0)), None);
        assert_eq!(dimensions(&fixture("t_bad.bin")), None);
        let head = fixture("t_clear-f187n.bin");
        assert_eq!(dimensions(&head[..330]), None);
        assert_eq!(dimensions(&[0xFF, 0xD8, 0xFF, 0xC0, 0x00]), None);
    }

    #[test]
    fn comment_reads_fitscut_cards_and_fixed_width_cards() {
        let text = comment(&fixture("t_clear-f187n.bin")).unwrap();
        assert!(text.starts_with("CTYPE1  = 'RA---TAN'"), "{text}");
        assert!(text.contains("CROTA2  = 0"));
        assert_eq!(comment(&fixture("t_bad.bin")), None);
        let cards = [
            "CTYPE1  = 'RA---TAN'           / projection",
            "CTYPE2  = 'DEC--TAN'",
            "CRPIX1  =                 10.5",
            "CRPIX2  =                 20.5",
            "CRVAL1  =                 15.0",
            "CRVAL2  =                -30.0",
            "CD1_1   =             -1.0D-04",
            "CD2_2   =              1.0D-04",
        ]
        .iter()
        .map(|card| format!("{card:<80}"))
        .collect::<String>();
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xFE];
        jpeg.extend_from_slice(&((cards.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(cards.as_bytes());
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        let wcs = quick_wcs(&jpeg).unwrap();
        assert_eq!(wcs.ctype, "RA---TAN");
        assert_eq!(wcs.crpix, [10.5, 20.5]);
        assert_eq!(wcs.crval, [15.0, -30.0]);
        assert_eq!(wcs.cd, [[-1.0e-4, 0.0], [0.0, 1.0e-4]]);
        assert!(wcs.from_cd);
    }

    #[test]
    fn quick_wcs_cdelt_crota_is_unreliable() {
        let bytes = fixture("t_clear-f187n.bin");
        let wcs = quick_wcs(&bytes).unwrap();
        assert!(!wcs.from_cd);
        assert_eq!(wcs.ctype, "RA---TAN");
        assert_eq!(wcs.crpix, [64.1398465441007, 38.5197857328984]);
        assert_eq!(wcs.crval, [274.729906841095, -13.8517218281262]);
        let (width, height, _) = dimensions(&bytes).unwrap();
        assert_eq!(width.max(height), 128);
        let summary = wcs.summary(width, height).unwrap();
        assert_eq!(summary.orientation, WcsOrientation::Unreliable);
        assert_eq!(summary.ctype, "RA---TAN");
        assert!(close(summary.pixel_scale_arcsec, 0.000978983020639757 * 3600.0, 1e-9), "{summary:?}");
        assert!(close(summary.field_arcmin[0], 128.0 * 0.000978983020639757 * 60.0, 1e-9), "{summary:?}");
        assert!(close(summary.field_arcmin[1], 76.0 * 0.000978983020639757 * 60.0, 1e-9), "{summary:?}");
        assert!(close(summary.centre_ra_deg, 274.7299, 0.005), "{summary:?}");
        assert!(close(summary.centre_dec_deg, -13.8517, 0.005), "{summary:?}");
        let cards = "CTYPE1  = 'RA---TAN'\nCRPIX1  = 1\nCRPIX2  = 1\nCRVAL1  = 10\nCRVAL2  = 20\nCDELT1  = -0.0001\nCDELT2  = 0.0001\nCROTA2  = 90\n";
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xFE];
        jpeg.extend_from_slice(&((cards.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(cards.as_bytes());
        let rotated = quick_wcs(&jpeg).unwrap();
        assert!(!rotated.from_cd);
        assert!(close(rotated.cd[0][0], 0.0, 1e-15) && close(rotated.cd[1][1], 0.0, 1e-15), "{rotated:?}");
        assert!(close(rotated.cd[0][1], -1.0e-4, 1e-15) && close(rotated.cd[1][0], -1.0e-4, 1e-15), "{rotated:?}");
        assert_eq!(rotated.summary(10, 10).unwrap().orientation, WcsOrientation::Unreliable);
    }

    #[test]
    fn quick_wcs_cd_matrix_is_reliable() {
        let bytes = fixture("p_jw02739001002_02105_00003_nrca1_cal_sos.bin");
        let wcs = quick_wcs(&bytes).unwrap();
        assert!(wcs.from_cd);
        assert_eq!(wcs.ctype, "RA---TAN-SIP");
        assert_eq!(wcs.cd, [[2.8524168572147e-07, 8.6958363118598e-06], [8.6435227513201e-06, -2.7897731555856e-07]]);
        let summary = wcs.summary(2048, 2048).unwrap();
        assert_eq!(summary.orientation, WcsOrientation::CdMatrix);
        let det: f64 = 2.8524168572147e-07 * -2.7897731555856e-07 - 8.6958363118598e-06 * 8.6435227513201e-06;
        let scale = det.abs().sqrt() * 3600.0;
        assert!(close(summary.pixel_scale_arcsec, scale, 1e-12), "{summary:?}");
        assert!(close(summary.pixel_scale_arcsec, 0.0312, 0.0005), "{summary:?}");
        assert!(close(summary.field_arcmin[0], 2048.0 * scale / 60.0, 1e-9));
        assert!(close(summary.centre_ra_deg, 274.70227719546, 1e-9), "{summary:?}");
        assert!(close(summary.centre_dec_deg, -13.872310766872, 1e-9), "{summary:?}");
        let hst = quick_wcs(&fixture("prev_hst_head_sos.bin")).unwrap().summary(4253, 4659).unwrap();
        assert_eq!(hst.orientation, WcsOrientation::CdMatrix);
        assert!(close(hst.pixel_scale_arcsec, 0.040, 0.001), "{hst:?}");
        assert!(close(hst.field_arcmin[0], 2.8, 0.05) && close(hst.field_arcmin[1], 3.1, 0.05), "{hst:?}");
        assert!(quick_wcs(&fixture("t_bad.bin")).is_none());
        let degenerate = QuickWcs { cd: [[0.0, 0.0], [0.0, 0.0]], ..wcs };
        assert!(degenerate.summary(10, 10).is_none());
    }

    #[test]
    fn quick_wcs_centre_uses_the_tan_projection() {
        let wcs = QuickWcs {
            ctype: "RA---TAN".into(),
            crpix: [1.0, 1.0],
            crval: [0.0, 0.0],
            cd: [[-1.0 / 3600.0, 0.0], [0.0, 1.0 / 3600.0]],
            from_cd: true,
        };
        assert_eq!(wcs.pixel_to_world(1.0, 1.0), (0.0, 0.0));
        let summary = wcs.summary(3601, 3601).unwrap();
        assert!(close(summary.centre_ra_deg, 359.5, 1e-3), "{summary:?}");
        assert!(close(summary.centre_dec_deg, 0.5, 1e-3), "{summary:?}");
        let standard = 0.5f64.to_radians();
        let expected_ra = 360.0 - standard.atan().to_degrees();
        let expected_dec = (standard / (1.0 + standard * standard).sqrt()).atan().to_degrees();
        assert!(close(summary.centre_ra_deg, expected_ra, 1e-9), "{summary:?} {expected_ra}");
        assert!(close(summary.centre_dec_deg, expected_dec, 1e-9), "{summary:?} {expected_dec}");
        assert!(close(summary.pixel_scale_arcsec, 1.0, 1e-12));
        let polar = QuickWcs { crval: [120.0, 89.9], ..wcs.clone() };
        let (ra, dec) = polar.pixel_to_world(1.0, 1.0);
        assert!(close(ra, 120.0, 1e-9) && close(dec, 89.9, 1e-9));
        let linear = QuickWcs { ctype: "RA---CAR".into(), crval: [10.0, 0.0], ..wcs };
        let (ra, dec) = linear.pixel_to_world(3601.0, 1.0);
        assert!(close(ra, 9.0, 1e-9) && close(dec, 0.0, 1e-9), "{ra} {dec}");
    }

    #[test]
    fn scale_wcs_divides_crpix_and_multiplies_cd() {
        let bytes = fixture("prev_jw_head_sos.bin");
        let original = quick_wcs(&bytes).unwrap();
        let factor = 7065.0 / 2048.0;
        let scaled = scale_wcs(original.clone(), factor);
        assert_eq!(scaled.crpix, [original.crpix[0] / factor, original.crpix[1] / factor]);
        for row in 0..2 {
            for col in 0..2 {
                assert_eq!(scaled.cd[row][col], original.cd[row][col] * factor);
            }
        }
        assert_eq!(scaled.crval, original.crval);
        assert_eq!(scaled.from_cd, original.from_cd);
        assert_eq!(scaled.ctype, original.ctype);
        let before = original.summary(7065, 4177).unwrap();
        let after = scaled.summary(2048, 1211).unwrap();
        assert!(close(after.pixel_scale_arcsec, before.pixel_scale_arcsec * factor, 1e-9));
        assert!(close(after.field_arcmin[0], before.field_arcmin[0], 1e-9), "{before:?} {after:?}");
        assert!(close(after.field_arcmin[1], before.field_arcmin[1], 0.01), "{before:?} {after:?}");
        assert!(close(after.centre_ra_deg, before.centre_ra_deg, 1e-3));
        assert!(close(after.centre_dec_deg, before.centre_dec_deg, 1e-3));
        assert_eq!(after.orientation, WcsOrientation::Unreliable);
    }
}
