use anyhow::Result;
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::astrometry::wcs::read_effective_cd;
use crate::core::imaging::debayer::BayerPattern;
use crate::core::imaging::push_history;
use crate::types::header::HduHeader;

type Mat = [[f64; 2]; 2];
type Vec2 = [f64; 2];

const STALE_ORIENTATION_CARDS: [&str; 5] = ["ORIENTAT", "PA_APER", "PA_V3", "VPARITY", "ROLL_REF"];
const ALTERNATE_AXIS_PREFIXES: [&str; 6] = ["CRPIX", "CRVAL", "CDELT", "CTYPE", "CUNIT", "CROTA"];
const ALTERNATE_MATRIX_PREFIXES: [&str; 2] = ["PC", "CD"];
const ALTERNATE_SCALAR_KEYS: [&str; 5] = ["LONPOLE", "LATPOLE", "WCSNAME", "RADESYS", "EQUINOX"];
const LINEAR_FORM_CARDS: [&str; 8] = ["PC1_1", "PC1_2", "PC2_1", "PC2_2", "CDELT1", "CDELT2", "CROTA1", "CROTA2"];
const CD_CARDS: [&str; 4] = ["CD1_1", "CD1_2", "CD2_1", "CD2_2"];
const PHYSICAL_CARDS: [&str; 6] = ["LTV1", "LTV2", "LTM1_1", "LTM1_2", "LTM2_1", "LTM2_2"];
const CFA_PATTERN_CARDS: [&str; 2] = ["BAYERPAT", "COLORTYP"];
const CFA_OFFSET_CARDS: [&str; 2] = ["XBAYROFF", "YBAYROFF"];
const CFA_UNRECOGNISED_HISTORY: &str = "geometry: CFA cards removed (pattern not recognised)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryOp {
    Rot90Cw,
    Rot180,
    Rot270Cw,
    FlipH,
    FlipV,
}

impl GeometryOp {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim() {
            "rot90" => Some(Self::Rot90Cw),
            "rot180" => Some(Self::Rot180),
            "rot270" => Some(Self::Rot270Cw),
            "flip_h" => Some(Self::FlipH),
            "flip_v" => Some(Self::FlipV),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rot90Cw => "rot90",
            Self::Rot180 => "rot180",
            Self::Rot270Cw => "rot270",
            Self::FlipH => "flip_h",
            Self::FlipV => "flip_v",
        }
    }

    pub fn transposes(self) -> bool {
        matches!(self, Self::Rot90Cw | Self::Rot270Cw)
    }

    pub fn output_dims(self, dims: (usize, usize)) -> (usize, usize) {
        let (rows, cols) = dims;
        if self.transposes() { (cols, rows) } else { (rows, cols) }
    }

    fn matrix(self) -> Mat {
        match self {
            Self::Rot90Cw => [[0.0, -1.0], [1.0, 0.0]],
            Self::Rot180 => [[-1.0, 0.0], [0.0, -1.0]],
            Self::Rot270Cw => [[0.0, 1.0], [-1.0, 0.0]],
            Self::FlipH => [[-1.0, 0.0], [0.0, 1.0]],
            Self::FlipV => [[1.0, 0.0], [0.0, -1.0]],
        }
    }
}

fn mat_vec(m: &Mat, v: Vec2) -> Vec2 {
    [m[0][0] * v[0] + m[0][1] * v[1], m[1][0] * v[0] + m[1][1] * v[1]]
}

fn mat_mul(a: &Mat, b: &Mat) -> Mat {
    [
        [a[0][0] * b[0][0] + a[0][1] * b[1][0], a[0][0] * b[0][1] + a[0][1] * b[1][1]],
        [a[1][0] * b[0][0] + a[1][1] * b[1][0], a[1][0] * b[0][1] + a[1][1] * b[1][1]],
    ]
}

fn transpose(m: &Mat) -> Mat {
    [[m[0][0], m[1][0]], [m[0][1], m[1][1]]]
}

pub fn pixel_map(op: GeometryOp, dims: (usize, usize)) -> (Mat, Vec2) {
    let (rows, cols) = dims;
    let (h, w) = (rows as f64, cols as f64);
    let m = op.matrix();
    let t0 = match op {
        GeometryOp::Rot90Cw => [h - 1.0, 0.0],
        GeometryOp::Rot180 => [w - 1.0, h - 1.0],
        GeometryOp::Rot270Cw => [0.0, w - 1.0],
        GeometryOp::FlipH => [w - 1.0, 0.0],
        GeometryOp::FlipV => [0.0, h - 1.0],
    };
    let m1 = mat_vec(&m, [1.0, 1.0]);
    (m, [t0[0] + 1.0 - m1[0], t0[1] + 1.0 - m1[1]])
}

pub fn apply_geometry(arr: &Array2<f32>, op: GeometryOp) -> Array2<f32> {
    let (rows, cols) = arr.dim();
    let (out_rows, out_cols) = op.output_dims((rows, cols));
    if arr.is_empty() {
        return Array2::zeros((out_rows, out_cols));
    }
    let mut data = vec![0f32; out_rows * out_cols];
    data.par_chunks_mut(out_cols).enumerate().for_each(|(r, row)| {
        for (c, v) in row.iter_mut().enumerate() {
            let (sy, sx) = source_index(op, rows, cols, r, c);
            *v = arr[[sy, sx]];
        }
    });
    Array2::from_shape_vec((out_rows, out_cols), data).expect("output buffer holds rows * cols values")
}

fn source_index(op: GeometryOp, rows: usize, cols: usize, r: usize, c: usize) -> (usize, usize) {
    match op {
        GeometryOp::Rot90Cw => (rows - 1 - c, r),
        GeometryOp::Rot180 => (rows - 1 - r, cols - 1 - c),
        GeometryOp::Rot270Cw => (c, cols - 1 - r),
        GeometryOp::FlipH => (r, cols - 1 - c),
        GeometryOp::FlipV => (rows - 1 - r, c),
    }
}

fn remapped_cfa(pattern: BayerPattern, op: GeometryOp, dims: (usize, usize)) -> Option<BayerPattern> {
    let sites = pattern.name().as_bytes();
    let (rows, cols) = (dims.0 % 2 + 2, dims.1 % 2 + 2);
    let mut out = [0u8; 4];
    for r in 0..2 {
        for c in 0..2 {
            let (sy, sx) = source_index(op, rows, cols, r, c);
            out[r * 2 + c] = sites[(sy % 2) * 2 + sx % 2];
        }
    }
    BayerPattern::parse(std::str::from_utf8(&out).ok()?)
}

fn remap_cfa(header: &mut HduHeader, source: &HduHeader, op: GeometryOp, dims: (usize, usize)) -> Option<String> {
    let raw = CFA_PATTERN_CARDS.iter().find_map(|k| source.get(k))?;
    let base = BayerPattern::parse(raw);
    let effective = BayerPattern::detect(source);
    let remapped = effective.and_then(|p| remapped_cfa(p, op, dims));
    for key in CFA_PATTERN_CARDS.iter().chain(CFA_OFFSET_CARDS.iter()) {
        header.remove(key);
    }
    let (Some(base), Some(effective), Some(new)) = (base, effective, remapped) else {
        return Some(CFA_UNRECOGNISED_HISTORY.to_string());
    };
    header.remove("ROWORDER");
    header.set("BAYERPAT", new.name().to_string());
    Some(if base == effective {
        format!("geometry: CFA pattern {} -> {}", base.name(), new.name())
    } else {
        format!("geometry: CFA pattern {} (effective {}) -> {}", base.name(), effective.name(), new.name())
    })
}

pub fn has_linear_wcs(header: &HduHeader) -> bool {
    header.get_f64("CRPIX1").is_some() && header.get_f64("CRPIX2").is_some() && read_effective_cd(header).is_ok()
}

fn exact_keys(header: &HduHeader, matches: impl Fn(&str) -> bool) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for key in header.cards.iter().map(|(k, _)| k).chain(header.index.keys()) {
        if matches(key.trim()) && !keys.contains(key) {
            keys.push(key.clone());
        }
    }
    keys
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn is_alternate_wcs_key(key: &str) -> bool {
    let Some(stem) = key.strip_suffix(|c: char| c.is_ascii_uppercase()) else {
        return false;
    };
    if ALTERNATE_SCALAR_KEYS.contains(&stem) {
        return true;
    }
    if ALTERNATE_AXIS_PREFIXES.iter().any(|p| stem.strip_prefix(p).is_some_and(all_digits)) {
        return true;
    }
    ALTERNATE_MATRIX_PREFIXES.iter().any(|p| {
        stem.strip_prefix(p)
            .and_then(|rest| rest.split_once('_'))
            .is_some_and(|(i, j)| all_digits(i) && all_digits(j))
    })
}

fn remove_stale_orientation(header: &mut HduHeader) -> Vec<String> {
    let keys = exact_keys(header, |k| STALE_ORIENTATION_CARDS.contains(&k) || is_alternate_wcs_key(k));
    for key in &keys {
        header.remove(key);
    }
    keys.into_iter().map(|k| k.trim().to_string()).collect()
}

type Poly = Vec<(i32, i32, f64)>;

fn sip_terms(header: &HduHeader, prefix: &str) -> (Poly, Vec<String>) {
    let lead = format!("{prefix}_");
    let mut terms = Vec::new();
    let mut keys = Vec::new();
    for key in exact_keys(header, |k| k.starts_with(&lead)) {
        let indices = key.trim().strip_prefix(&lead).and_then(|rest| rest.split_once('_'));
        let Some((p, q)) = indices.filter(|(p, q)| all_digits(p) && all_digits(q)) else {
            continue;
        };
        if let (Ok(p), Ok(q), Some(c)) = (p.parse::<i32>(), q.parse::<i32>(), header.get_f64(&key)) {
            terms.push((p, q, c));
            keys.push(key);
        }
    }
    (terms, keys)
}

fn compose_with_transpose(poly: &Poly, m: &Mat) -> Poly {
    let mt = transpose(m);
    poly.iter()
        .map(|&(p, q, c)| {
            if mt[0][1] == 0.0 {
                (p, q, c * mt[0][0].powi(p) * mt[1][1].powi(q))
            } else {
                (q, p, c * mt[0][1].powi(p) * mt[1][0].powi(q))
            }
        })
        .collect()
}

fn scaled(poly: Poly, factor: f64) -> Poly {
    poly.into_iter()
        .map(|(p, q, c)| (p, q, c * factor))
        .filter(|(_, _, c)| *c != 0.0)
        .collect()
}

fn swap_cards(header: &mut HduHeader, source: &HduHeader, left: &str, right: &str) {
    let l = source.get(left).map(str::to_string);
    let r = source.get(right).map(str::to_string);
    header.remove(left);
    header.remove(right);
    if let Some(v) = r {
        header.set(left, v);
    }
    if let Some(v) = l {
        header.set(right, v);
    }
}

fn remap_sip_pair(header: &mut HduHeader, source: &HduHeader, a_prefix: &str, b_prefix: &str, m: &Mat) {
    let (a, a_keys) = sip_terms(source, a_prefix);
    let (b, b_keys) = sip_terms(source, b_prefix);
    if a.is_empty() && b.is_empty() {
        return;
    }
    for key in a_keys.iter().chain(b_keys.iter()) {
        header.remove(key);
    }
    let a_composed = compose_with_transpose(&a, m);
    let b_composed = compose_with_transpose(&b, m);
    let transposed = m[0][1] != 0.0;
    let (a_new, b_new) = if transposed {
        (scaled(b_composed, m[0][1]), scaled(a_composed, m[1][0]))
    } else {
        (scaled(a_composed, m[0][0]), scaled(b_composed, m[1][1]))
    };
    for (prefix, poly) in [(a_prefix, a_new), (b_prefix, b_new)] {
        for (p, q, c) in poly {
            header.set_f64(&format!("{prefix}_{p}_{q}"), c);
        }
    }
    if transposed {
        swap_cards(header, source, &format!("{a_prefix}_ORDER"), &format!("{b_prefix}_ORDER"));
    }
}

fn remap_sip(header: &mut HduHeader, source: &HduHeader, m: &Mat) {
    remap_sip_pair(header, source, "A", "B", m);
    remap_sip_pair(header, source, "AP", "BP", m);
    if m[0][1] != 0.0 {
        swap_cards(header, source, "A_DMAX", "B_DMAX");
    }
}

fn remap_physical(header: &mut HduHeader, source: &HduHeader, m: &Mat, t_f: Vec2) {
    if !PHYSICAL_CARDS.iter().any(|k| source.get_f64(k).is_some()) {
        return;
    }
    let ltv = [source.get_f64("LTV1").unwrap_or(0.0), source.get_f64("LTV2").unwrap_or(0.0)];
    let ltm = [
        [source.get_f64("LTM1_1").unwrap_or(1.0), source.get_f64("LTM1_2").unwrap_or(0.0)],
        [source.get_f64("LTM2_1").unwrap_or(0.0), source.get_f64("LTM2_2").unwrap_or(1.0)],
    ];
    let ltm_new = mat_mul(m, &ltm);
    let ltv_new = mat_vec(m, ltv);
    header.set_f64("LTV1", ltv_new[0] + t_f[0]);
    header.set_f64("LTV2", ltv_new[1] + t_f[1]);
    header.set_f64("LTM1_1", ltm_new[0][0]);
    header.set_f64("LTM1_2", ltm_new[0][1]);
    header.set_f64("LTM2_1", ltm_new[1][0]);
    header.set_f64("LTM2_2", ltm_new[1][1]);
}

pub fn geometry_header(source: &HduHeader, op: GeometryOp, dims: (usize, usize)) -> Result<HduHeader> {
    let (rows, cols) = dims;
    let (out_rows, out_cols) = op.output_dims(dims);
    let (m, t_f) = pixel_map(op, dims);
    let mut header = source.clone();
    let mut history = vec![format!("geometry: {} ({cols}x{rows} -> {out_cols}x{out_rows})", op.name())];

    if has_linear_wcs(source) {
        let cd = read_effective_cd(source)?;
        let crpix = mat_vec(&m, [source.get_f64("CRPIX1").unwrap_or(0.0), source.get_f64("CRPIX2").unwrap_or(0.0)]);
        header.set_f64("CRPIX1", crpix[0] + t_f[0]);
        header.set_f64("CRPIX2", crpix[1] + t_f[1]);
        let linear_form = exact_keys(source, |k| LINEAR_FORM_CARDS.contains(&k));
        for key in &linear_form {
            header.remove(key);
        }
        let cd_new = mat_mul(&cd, &transpose(&m));
        for (key, value) in CD_CARDS.iter().zip([cd_new[0][0], cd_new[0][1], cd_new[1][0], cd_new[1][1]]) {
            header.set_f64(key, value);
        }
        let cd_was_present = CD_CARDS.iter().any(|k| source.get_f64(k).is_some());
        if !linear_form.is_empty() && !cd_was_present {
            history.push("geometry: PC/CDELT converted to CD".to_string());
        }
        remap_sip(&mut header, source, &m);
    }
    remap_physical(&mut header, source, &m, t_f);
    if let Some(note) = remap_cfa(&mut header, source, op, dims) {
        history.push(note);
    }
    let removed = remove_stale_orientation(&mut header);
    if !removed.is_empty() {
        history.push(format!("geometry: removed stale orientation cards {}", removed.join(", ")));
    }
    header.set("NAXIS1", out_cols.to_string());
    header.set("NAXIS2", out_rows.to_string());
    for line in history {
        push_history(&mut header, &line);
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::astrometry::wcs::WcsTransform;

    const ALL_OPS: [GeometryOp; 5] =
        [GeometryOp::Rot90Cw, GeometryOp::Rot180, GeometryOp::Rot270Cw, GeometryOp::FlipH, GeometryOp::FlipV];

    fn zero_based_offset(op: GeometryOp, dims: (usize, usize)) -> (Mat, Vec2) {
        let (m, t_f) = pixel_map(op, dims);
        let m1 = mat_vec(&m, [1.0, 1.0]);
        (m, [t_f[0] - 1.0 + m1[0], t_f[1] - 1.0 + m1[1]])
    }

    fn mapped(op: GeometryOp, dims: (usize, usize), x: f64, y: f64) -> (f64, f64) {
        let (m, t0) = zero_based_offset(op, dims);
        let p = mat_vec(&m, [x, y]);
        (p[0] + t0[0], p[1] + t0[1])
    }

    fn text<'a>(header: &'a HduHeader, key: &str) -> Option<&'a str> {
        header.get(key).map(|v| v.trim().trim_matches('\'').trim())
    }

    fn history(header: &HduHeader) -> Vec<String> {
        header
            .cards
            .iter()
            .filter(|(k, _)| k.trim() == "HISTORY")
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn f444w_sci_header() -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in [
            ("WCSAXES", "2"),
            ("CTYPE1", "'RA---TAN'"),
            ("CTYPE2", "'DEC--TAN'"),
            ("CUNIT1", "'deg'"),
            ("CUNIT2", "'deg'"),
            ("RADESYS", "'ICRS'"),
            ("NAXIS1", "7065"),
            ("NAXIS2", "4177"),
            ("BUNIT", "'MJy/sr'"),
        ] {
            h.set(k, v.to_string());
        }
        h.set_f64("CRPIX1", 3545.3994741278593);
        h.set_f64("CRPIX2", 2092.707324684303);
        h.set_f64("CRVAL1", 274.7299961326976);
        h.set_f64("CRVAL2", -13.851596271958135);
        h.set_f64("CDELT1", 1.74739632912783e-5);
        h.set_f64("CDELT2", 1.74739632912783e-5);
        h.set_f64("PC1_1", 0.04044713241775692);
        h.set_f64("PC1_2", 0.99918167991571);
        h.set_f64("PC2_1", 0.99918167991571);
        h.set_f64("PC2_2", -0.04044713241775692);
        h.set_f64("PIXAR_SR", 9.30116980868672e-14);
        h.set_f64("PA_APER", 92.31808232655477);
        h.set_f64("PA_V3", 92.34742767851586);
        h
    }

    fn tan_cd_header(naxis: usize) -> HduHeader {
        let mut h = HduHeader::empty();
        h.set("CTYPE1", "RA---TAN".to_string());
        h.set("CTYPE2", "DEC--TAN".to_string());
        h.set("NAXIS1", naxis.to_string());
        h.set("NAXIS2", naxis.to_string());
        h.set_f64("CRVAL1", 150.0);
        h.set_f64("CRVAL2", 2.0);
        h.set_f64("CRPIX1", 100.0);
        h.set_f64("CRPIX2", 100.0);
        h.set_f64("CD1_1", -1e-4);
        h.set_f64("CD2_2", 1e-4);
        h.set_f64("PIXAR_SR", 4e-14);
        h.set("EXTNAME", "SCI".to_string());
        h
    }

    fn sip_header() -> HduHeader {
        let mut h = tan_cd_header(200);
        h.set("CTYPE1", "RA---TAN-SIP".to_string());
        h.set("CTYPE2", "DEC--TAN-SIP".to_string());
        h.set("A_ORDER", "2".to_string());
        h.set("B_ORDER", "2".to_string());
        h.set_f64("A_2_0", 2e-5);
        h.set_f64("A_1_1", -1e-5);
        h.set_f64("B_0_2", 3e-5);
        h.set_f64("A_DMAX", 1.5);
        h.set("AP_ORDER", "2".to_string());
        h.set("BP_ORDER", "2".to_string());
        h.set_f64("AP_2_0", -2e-5);
        h.set_f64("AP_1_1", 1e-5);
        h.set_f64("BP_0_2", -3e-5);
        h
    }

    fn assert_sky_preserved(before: &HduHeader, op: GeometryOp, points: &[(f64, f64)]) -> HduHeader {
        let rows = before.get_i64("NAXIS2").unwrap() as usize;
        let cols = before.get_i64("NAXIS1").unwrap() as usize;
        let after = geometry_header(before, op, (rows, cols)).unwrap();
        let wcs_before = WcsTransform::from_header(before).unwrap();
        let wcs_after = WcsTransform::from_header(&after).unwrap();
        for &(x, y) in points {
            let (xm, ym) = mapped(op, (rows, cols), x, y);
            let a = wcs_before.pixel_to_world(x, y);
            let b = wcs_after.pixel_to_world(xm, ym);
            assert!(
                (a.ra - b.ra).abs() < 1e-9 && (a.dec - b.dec).abs() < 1e-9,
                "{} at ({x}, {y}) -> ({xm}, {ym}): before {} {} after {} {}",
                op.name(),
                a.ra,
                a.dec,
                b.ra,
                b.dec
            );
        }
        after
    }

    #[test]
    fn rot90_and_flips_move_pixels_as_specified() {
        let a = Array2::from_shape_vec((2, 3), vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let expected: [(GeometryOp, (usize, usize), Vec<f32>); 5] = [
            (GeometryOp::Rot90Cw, (3, 2), vec![4.0, 1.0, 5.0, 2.0, 6.0, 3.0]),
            (GeometryOp::Rot180, (2, 3), vec![6.0, 5.0, 4.0, 3.0, 2.0, 1.0]),
            (GeometryOp::Rot270Cw, (3, 2), vec![3.0, 6.0, 2.0, 5.0, 1.0, 4.0]),
            (GeometryOp::FlipH, (2, 3), vec![3.0, 2.0, 1.0, 6.0, 5.0, 4.0]),
            (GeometryOp::FlipV, (2, 3), vec![4.0, 5.0, 6.0, 1.0, 2.0, 3.0]),
        ];
        for (op, dims, values) in expected {
            let out = apply_geometry(&a, op);
            assert_eq!(out.dim(), dims, "{}", op.name());
            assert_eq!(out.as_slice().unwrap(), values.as_slice(), "{}", op.name());
            assert_eq!(op.output_dims(a.dim()), dims);
            for ((y, x), v) in a.indexed_iter() {
                let (xm, ym) = mapped(op, a.dim(), x as f64, y as f64);
                assert_eq!(out[[ym as usize, xm as usize]], *v, "{} pixel ({x}, {y})", op.name());
            }
        }
        assert_eq!(GeometryOp::from_name("rot90"), Some(GeometryOp::Rot90Cw));
        assert_eq!(GeometryOp::from_name("flip_v"), Some(GeometryOp::FlipV));
        assert_eq!(GeometryOp::from_name("rotate"), None);
    }

    #[test]
    fn geometry_header_keeps_sky_positions_for_the_jwst_f444w_wcs() {
        let before = f444w_sci_header();
        let (w, h) = (7065.0, 4177.0);
        let points = [(0.0, 0.0), (w - 1.0, 0.0), (0.0, h - 1.0), (w - 1.0, h - 1.0), (1234.5, 777.25)];
        for op in ALL_OPS {
            let after = assert_sky_preserved(&before, op, &points);
            let (rows, cols) = op.output_dims((4177, 7065));
            assert_eq!(after.get_i64("NAXIS1"), Some(cols as i64), "{}", op.name());
            assert_eq!(after.get_i64("NAXIS2"), Some(rows as i64), "{}", op.name());
            assert!(has_linear_wcs(&after));
        }
        let rot90 = geometry_header(&before, GeometryOp::Rot90Cw, (4177, 7065)).unwrap();
        assert!((rot90.get_f64("CRPIX1").unwrap() - 2085.2927).abs() < 1e-4, "{:?}", rot90.get("CRPIX1"));
        assert!((rot90.get_f64("CRPIX2").unwrap() - 3545.3995).abs() < 1e-4, "{:?}", rot90.get("CRPIX2"));
        assert!((rot90.get_f64("CD1_1").unwrap() + 1.745966e-5).abs() < 1e-10);
        assert!((rot90.get_f64("CD1_2").unwrap() - 7.067717e-7).abs() < 1e-11);
        assert!((rot90.get_f64("CD2_1").unwrap() - 7.067717e-7).abs() < 1e-11);
        assert!((rot90.get_f64("CD2_2").unwrap() - 1.745966e-5).abs() < 1e-10);
        assert!((rot90.get_f64("PIXAR_SR").unwrap() - 9.30116980868672e-14).abs() < 1e-28);
        assert_eq!(text(&rot90, "CTYPE1"), Some("RA---TAN"));
        assert!(history(&rot90).iter().any(|h| h.starts_with("geometry: rot90")), "{:?}", history(&rot90));
        assert!(history(&rot90).iter().all(|h| h.is_ascii()));
        assert!(!has_linear_wcs(&HduHeader::empty()));
    }

    #[test]
    fn pc_cdelt_headers_are_written_as_cd_after_a_rotation() {
        let before = f444w_sci_header();
        let after = geometry_header(&before, GeometryOp::Rot90Cw, (4177, 7065)).unwrap();
        for key in LINEAR_FORM_CARDS {
            assert!(after.get(key).is_none(), "{key} survived");
            assert!(!after.cards.iter().any(|(k, _)| k.trim() == key), "{key} card survived");
        }
        for key in CD_CARDS {
            assert!(after.get_f64(key).is_some(), "{key} missing");
        }
        assert!(history(&after).iter().any(|h| h == "geometry: PC/CDELT converted to CD"), "{:?}", history(&after));

        let cd_source = tan_cd_header(64);
        let kept = geometry_header(&cd_source, GeometryOp::Rot90Cw, (64, 64)).unwrap();
        assert!(history(&kept).iter().all(|h| !h.contains("converted to CD")), "{:?}", history(&kept));

        let mut cd_with_redundant_cdelt = tan_cd_header(64);
        cd_with_redundant_cdelt.set_f64("CDELT1", -1e-4);
        cd_with_redundant_cdelt.set_f64("CDELT2", 1e-4);
        let trimmed = geometry_header(&cd_with_redundant_cdelt, GeometryOp::Rot90Cw, (64, 64)).unwrap();
        assert!(trimmed.get("CDELT1").is_none() && trimmed.get("CDELT2").is_none(), "{:?}", trimmed.cards);
        for key in CD_CARDS {
            assert_eq!(trimmed.get_f64(key), kept.get_f64(key), "{key}");
        }
        assert!(history(&trimmed).iter().all(|h| !h.contains("converted to CD")), "{:?}", history(&trimmed));
    }

    #[test]
    fn stale_orientation_cards_are_removed() {
        let mut before = tan_cd_header(64);
        before.set_f64("ORIENTAT", -131.9115);
        before.set_f64("PA_APER", 92.318);
        before.set_f64("PA_V3", 92.347);
        before.set("VPARITY", "-1".to_string());
        before.set_f64("CRPIX1A", 100.0);
        before.set_f64("CD1_1A", -1e-4);
        before.set("CTYPE1A", "RA---TAN".to_string());
        before.set("RADESYS", "ICRS".to_string());
        before.set_f64("ROLL_REF", 92.1);
        before.set_f64("CRPIX2A", 100.0);
        before.set_f64("CRVAL1A", 150.0);
        before.set_f64("CRVAL2A", 2.0);
        before.set("CTYPE2A", "DEC--TAN".to_string());
        before.set("CUNIT1A", "deg".to_string());
        before.set_f64("CD2_2A", 1e-4);
        before.set("WCSNAMEA", "ALT".to_string());
        before.set("RADESYSA", "ICRS".to_string());
        let stale = [
            "ORIENTAT", "PA_APER", "PA_V3", "VPARITY", "ROLL_REF", "CRPIX1A", "CRPIX2A", "CRVAL1A", "CRVAL2A", "CTYPE1A",
            "CTYPE2A", "CUNIT1A", "CD1_1A", "CD2_2A", "WCSNAMEA", "RADESYSA",
        ];
        for op in ALL_OPS {
            let after = geometry_header(&before, op, (64, 64)).unwrap();
            for key in stale {
                assert!(after.get(key).is_none(), "{} kept {key}", op.name());
                assert!(!after.cards.iter().any(|(k, _)| k.trim() == key), "{} kept the {key} card", op.name());
            }
            assert_eq!(text(&after, "RADESYS"), Some("ICRS"), "{}", op.name());
            assert_eq!(text(&after, "CTYPE1"), Some("RA---TAN"));
            let notes = history(&after);
            assert!(notes.iter().any(|h| h.contains("removed stale orientation cards")), "{} wrote no removal note: {notes:?}", op.name());
            assert!(notes.iter().all(|h| h.len() <= 72), "{}: {notes:?}", op.name());
            let note = notes.join(" ");
            for key in stale {
                assert!(note.contains(key), "{}: {key} missing from {note}", op.name());
            }
            let (m, t_f) = pixel_map(op, (64, 64));
            let expected = mat_vec(&m, [100.0, 100.0]);
            assert!((after.get_f64("CRPIX1").unwrap() - (expected[0] + t_f[0])).abs() < 1e-9, "{}", op.name());
            assert!((after.get_f64("CRPIX2").unwrap() - (expected[1] + t_f[1])).abs() < 1e-9, "{}", op.name());
            assert!(after.get_f64("CD1_1").is_some());
            if op == GeometryOp::FlipH {
                assert!((after.get_f64("CD1_1").unwrap() - 1e-4).abs() < 1e-18);
            }
        }
        let clean = geometry_header(&tan_cd_header(64), GeometryOp::FlipV, (64, 64)).unwrap();
        assert!(history(&clean).iter().all(|h| !h.contains("removed stale")), "{:?}", history(&clean));
    }

    #[test]
    fn sip_polynomials_are_remapped_exactly() {
        let before = sip_header();
        let points = [(0.0, 0.0), (199.0, 0.0), (0.0, 199.0), (199.0, 199.0), (37.5, 120.25), (150.0, 20.0)];
        let wcs_before = WcsTransform::from_header(&before).unwrap();
        for op in [GeometryOp::FlipH, GeometryOp::Rot90Cw, GeometryOp::Rot180] {
            let after = assert_sky_preserved(&before, op, &points);
            let wcs_after = WcsTransform::from_header(&after).unwrap();
            for &(x, y) in &points {
                let sky = wcs_before.pixel_to_world(x, y);
                let (bx, by) = wcs_before.world_to_pixel(sky.ra, sky.dec);
                let (xm, ym) = mapped(op, (200, 200), bx, by);
                let (xi, yi) = wcs_after.world_to_pixel(sky.ra, sky.dec);
                assert!((xi - xm).abs() < 1e-6 && (yi - ym).abs() < 1e-6, "{} inverse ({x}, {y}) -> ({xi}, {yi}) expected ({xm}, {ym})", op.name());
                assert!((bx - x).abs() < 0.01 && (by - y).abs() < 0.01, "fixture AP/BP drifted: ({bx}, {by}) for ({x}, {y})");
            }
            assert_eq!(text(&after, "CTYPE1"), Some("RA---TAN-SIP"), "{}", op.name());
            assert_eq!(text(&after, "CTYPE2"), Some("DEC--TAN-SIP"), "{}", op.name());
            assert_eq!(after.get_f64("A_ORDER"), Some(2.0));
            assert_eq!(after.get_f64("B_ORDER"), Some(2.0));
            assert_eq!(after.get_f64("AP_ORDER"), Some(2.0));
        }
        let rot90 = geometry_header(&before, GeometryOp::Rot90Cw, (200, 200)).unwrap();
        assert_eq!(rot90.get_f64("B_DMAX"), Some(1.5), "A_DMAX did not move to B_DMAX");
        assert!(rot90.get("A_DMAX").is_none());
        assert!((rot90.get_f64("A_2_0").unwrap() + 3e-5).abs() < 1e-20, "{:?}", rot90.get("A_2_0"));
        assert!((rot90.get_f64("B_0_2").unwrap() - 2e-5).abs() < 1e-20, "{:?}", rot90.get("B_0_2"));
        assert!((rot90.get_f64("B_1_1").unwrap() - 1e-5).abs() < 1e-20, "{:?}", rot90.get("B_1_1"));
        assert!(rot90.get("A_0_2").is_none());
        assert!(rot90.get("A_1_1").is_none());
        let flip = geometry_header(&before, GeometryOp::FlipH, (200, 200)).unwrap();
        assert!((flip.get_f64("A_2_0").unwrap() + 2e-5).abs() < 1e-20, "{:?}", flip.get("A_2_0"));
        assert!((flip.get_f64("A_1_1").unwrap() + 1e-5).abs() < 1e-20, "{:?}", flip.get("A_1_1"));
        assert!((flip.get_f64("B_0_2").unwrap() - 3e-5).abs() < 1e-20, "{:?}", flip.get("B_0_2"));
    }

    #[test]
    fn sip_orders_and_dmax_swap_on_transpose() {
        let mut before = sip_header();
        before.set("A_ORDER", "3".to_string());
        before.set_f64("A_3_0", 1e-9);
        before.set_f64("B_DMAX", 0.7);
        for op in [GeometryOp::Rot90Cw, GeometryOp::Rot270Cw] {
            let after = geometry_header(&before, op, (200, 200)).unwrap();
            assert_eq!(after.get_f64("A_ORDER"), Some(2.0), "{}", op.name());
            assert_eq!(after.get_f64("B_ORDER"), Some(3.0), "{}", op.name());
            assert_eq!(after.get_f64("A_DMAX"), Some(0.7), "{}", op.name());
            assert_eq!(after.get_f64("B_DMAX"), Some(1.5), "{}", op.name());
            assert!(after.get_f64("B_0_3").is_some(), "{} lost the cubic term", op.name());
        }
        let flip = geometry_header(&before, GeometryOp::FlipV, (200, 200)).unwrap();
        assert_eq!(flip.get_f64("A_ORDER"), Some(3.0));
        assert_eq!(flip.get_f64("A_DMAX"), Some(1.5));
    }

    #[test]
    fn ltv_ltm_follow_the_transform() {
        let mut before = tan_cd_header(64);
        before.set("NAXIS1", "10".to_string());
        before.set("NAXIS2", "8".to_string());
        before.set_f64("LTV1", -3.0);
        before.set_f64("LTV2", -2.0);
        before.set_f64("LTM1_1", 1.0);
        before.set_f64("LTM2_2", 1.0);
        let after = geometry_header(&before, GeometryOp::Rot180, (8, 10)).unwrap();
        let (m, t_f) = pixel_map(GeometryOp::Rot180, (8, 10));
        assert_eq!(t_f, [11.0, 9.0]);
        let expected = mat_vec(&m, [-3.0, -2.0]);
        assert!((after.get_f64("LTV1").unwrap() - (expected[0] + t_f[0])).abs() < 1e-12, "{:?}", after.get("LTV1"));
        assert!((after.get_f64("LTV2").unwrap() - (expected[1] + t_f[1])).abs() < 1e-12, "{:?}", after.get("LTV2"));
        assert_eq!(after.get_f64("LTV1"), Some(14.0));
        assert_eq!(after.get_f64("LTV2"), Some(11.0));
        assert_eq!(after.get_f64("LTM1_1"), Some(-1.0));
        assert_eq!(after.get_f64("LTM2_2"), Some(-1.0));
        assert_eq!(after.get_f64("LTM1_2"), Some(0.0));
        assert_eq!(after.get_f64("LTM2_1"), Some(0.0));

        let rot90 = geometry_header(&before, GeometryOp::Rot90Cw, (8, 10)).unwrap();
        assert_eq!(rot90.get_f64("LTM1_1"), Some(0.0));
        assert_eq!(rot90.get_f64("LTM1_2"), Some(-1.0));
        assert_eq!(rot90.get_f64("LTM2_1"), Some(1.0));
        assert_eq!(rot90.get_f64("LTM2_2"), Some(0.0));

        let plain = geometry_header(&tan_cd_header(64), GeometryOp::Rot180, (64, 64)).unwrap();
        assert!(plain.get("LTV1").is_none(), "LTV cards invented for a header without them");
    }

    fn cfa_colour(pattern: BayerPattern, y: usize, x: usize) -> f32 {
        let site = pattern.name().as_bytes()[(y % 2) * 2 + x % 2];
        b"RGB".iter().position(|&b| b == site).unwrap() as f32
    }

    fn cfa_mosaic(pattern: BayerPattern, dims: (usize, usize)) -> Array2<f32> {
        Array2::from_shape_fn(dims, |(y, x)| cfa_colour(pattern, y, x))
    }

    fn assert_cfa_follows(before: &HduHeader, dims: (usize, usize)) {
        let effective = BayerPattern::detect(before).expect("the fixture carries a Bayer pattern");
        let mosaic = cfa_mosaic(effective, dims);
        for op in ALL_OPS {
            let out = apply_geometry(&mosaic, op);
            let after = geometry_header(before, op, dims).unwrap();
            let detected = BayerPattern::detect(&after)
                .unwrap_or_else(|| panic!("{} {dims:?}: no Bayer pattern in {:?}", op.name(), after.cards));
            for ((y, x), v) in out.indexed_iter() {
                assert_eq!(
                    *v,
                    cfa_colour(detected, y, x),
                    "{} {dims:?} at ({x}, {y}): the header says {}",
                    op.name(),
                    detected.name()
                );
            }
            for key in ["XBAYROFF", "YBAYROFF", "COLORTYP", "ROWORDER"] {
                assert!(after.get(key).is_none(), "{} {dims:?} kept {key}", op.name());
            }
            let notes = history(&after);
            assert!(notes.iter().any(|h| h.starts_with("geometry: CFA pattern ")), "{} {dims:?}: {notes:?}", op.name());
        }
    }

    #[test]
    fn cfa_pattern_follows_the_transform() {
        let mut before = tan_cd_header(64);
        before.set("BAYERPAT", "'RGGB'".to_string());
        for dims in [(4, 6), (3, 5), (4, 5), (3, 6)] {
            assert_cfa_follows(&before, dims);
        }
        let flip = geometry_header(&before, GeometryOp::FlipH, (4, 6)).unwrap();
        assert_eq!(text(&flip, "BAYERPAT"), Some("GRBG"));
        assert!(history(&flip).iter().any(|h| h == "geometry: CFA pattern RGGB -> GRBG"), "{:?}", history(&flip));
        let flip_odd = geometry_header(&before, GeometryOp::FlipH, (4, 5)).unwrap();
        assert_eq!(text(&flip_odd, "BAYERPAT"), Some("RGGB"));
        let plain = geometry_header(&tan_cd_header(64), GeometryOp::Rot90Cw, (64, 64)).unwrap();
        assert!(plain.get("BAYERPAT").is_none());
        assert!(history(&plain).iter().all(|h| !h.contains("CFA")), "{:?}", history(&plain));
    }

    #[test]
    fn cfa_offsets_and_row_order_fold_into_the_new_bayerpat() {
        let mut before = HduHeader::empty();
        before.set("BAYERPAT", "'RGGB'".to_string());
        before.set("XBAYROFF", "1".to_string());
        before.set("ROWORDER", "'BOTTOM-UP'".to_string());
        before.set("NAXIS1", "6".to_string());
        before.set("NAXIS2", "4".to_string());
        assert_eq!(BayerPattern::detect(&before), Some(BayerPattern::Bggr));
        assert_cfa_follows(&before, (4, 6));
        let rot = geometry_header(&before, GeometryOp::Rot180, (4, 6)).unwrap();
        assert_eq!(text(&rot, "BAYERPAT"), Some("RGGB"));
        assert!(
            history(&rot).iter().any(|h| h == "geometry: CFA pattern RGGB (effective BGGR) -> RGGB"),
            "{:?}",
            history(&rot)
        );

        let mut unknown = HduHeader::empty();
        unknown.set("COLORTYP", "'MONO'".to_string());
        unknown.set("ROWORDER", "'BOTTOM-UP'".to_string());
        let after = geometry_header(&unknown, GeometryOp::Rot90Cw, (4, 6)).unwrap();
        assert!(after.get("COLORTYP").is_none() && after.get("BAYERPAT").is_none(), "{:?}", after.cards);
        assert_eq!(text(&after, "ROWORDER"), Some("BOTTOM-UP"));
        assert!(
            history(&after).iter().any(|h| h.contains("CFA") && h.contains("not recognised")),
            "{:?}",
            history(&after)
        );
    }
}
