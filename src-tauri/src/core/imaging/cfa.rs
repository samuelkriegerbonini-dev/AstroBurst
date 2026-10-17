use ndarray::{s, Array2, ArrayView2, Zip};

use crate::core::imaging::debayer::{is_bottom_up, BayerPattern};
use crate::core::imaging::resample::compute_wcs_updates;
use crate::core::metadata::header_aliases::{
    first_i64, BINNING_KEYS, PIXEL_SIZE_UM_KEYS, Y_BINNING_KEYS, Y_PIXEL_SIZE_UM_KEYS,
};
use crate::types::header::HduHeader;

pub(crate) mod extract;

pub use extract::{
    extract_cfa, extract_green, extract_ha, extract_ha_oiii, extraction_pattern, split_cfa,
    CfaExtract, CfaExtracted, CfaPlane,
};

const BAYER_PERIOD: usize = 2;
const XTRANS_PERIOD: usize = 6;
const BAYER_CELL: usize = 2;
const XTRANS_CELL: usize = 3;
const XTRANS_CELL_SITES: [usize; 3] = [2, 5, 2];
const RED: u8 = 0;
const GREEN: u8 = 1;
const BLUE: u8 = 2;
const PATTERN_CARDS: [&str; 2] = ["BAYERPAT", "COLORTYP"];
const CFA_CARDS: [&str; 4] = ["BAYERPAT", "COLORTYP", "XBAYROFF", "YBAYROFF"];

type XTransLayout = [[u8; XTRANS_PERIOD]; XTRANS_PERIOD];

fn site_code(letter: u8) -> Option<u8> {
    match letter.to_ascii_uppercase() {
        b'R' => Some(RED),
        b'G' => Some(GREEN),
        b'B' => Some(BLUE),
        _ => None,
    }
}

fn site_letter(code: u8) -> char {
    match code {
        RED => 'R',
        GREEN => 'G',
        _ => 'B',
    }
}

fn channel_index(code: u8) -> usize {
    match code {
        RED => 0,
        GREEN => 1,
        _ => 2,
    }
}

fn wrap(offset: i64, period: usize) -> i64 {
    offset.rem_euclid(period as i64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaKind {
    Bayer,
    XTrans,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XTransPattern(XTransLayout);

impl XTransPattern {
    pub fn parse(text: &str) -> Option<Self> {
        let letters = text.trim().trim_matches('\'').trim().as_bytes();
        if letters.len() != XTRANS_PERIOD * XTRANS_PERIOD {
            return None;
        }
        let mut layout = [[0u8; XTRANS_PERIOD]; XTRANS_PERIOD];
        for (row, chunk) in layout.iter_mut().zip(letters.chunks_exact(XTRANS_PERIOD)) {
            for (site, &letter) in row.iter_mut().zip(chunk) {
                *site = site_code(letter)?;
            }
        }
        let pattern = Self(layout);
        pattern.balanced_origin().map(|_| pattern)
    }

    pub fn name(&self) -> String {
        self.0
            .iter()
            .flatten()
            .map(|&code| site_letter(code))
            .collect()
    }

    pub fn colour_at(&self, y: usize, x: usize) -> u8 {
        self.0[y % XTRANS_PERIOD][x % XTRANS_PERIOD]
    }

    pub fn with_offsets(&self, xoff: i64, yoff: i64) -> Self {
        let dy = wrap(yoff, XTRANS_PERIOD) as usize;
        let dx = wrap(xoff, XTRANS_PERIOD) as usize;
        Self(std::array::from_fn(|y| {
            std::array::from_fn(|x| self.colour_at(y + dy, x + dx))
        }))
    }

    pub fn flipped_rows(&self, height: usize) -> Self {
        let last_row = (height % XTRANS_PERIOD + XTRANS_PERIOD - 1) % XTRANS_PERIOD;
        Self(std::array::from_fn(|y| {
            self.0[(last_row + XTRANS_PERIOD - y) % XTRANS_PERIOD]
        }))
    }

    pub fn aligned_origin(&self) -> (usize, usize) {
        self.balanced_origin().unwrap_or((0, 0))
    }

    fn balanced_origin(&self) -> Option<(usize, usize)> {
        (0..XTRANS_CELL)
            .flat_map(|dy| (0..XTRANS_CELL).map(move |dx| (dy, dx)))
            .find(|&(dy, dx)| {
                [
                    (dy, dx),
                    (dy + XTRANS_CELL, dx),
                    (dy, dx + XTRANS_CELL),
                    (dy + XTRANS_CELL, dx + XTRANS_CELL),
                ]
                .iter()
                .all(|&(top, left)| self.cell_sites(top, left) == XTRANS_CELL_SITES)
            })
    }

    fn cell_sites(&self, top: usize, left: usize) -> [usize; 3] {
        let mut counts = [0usize; 3];
        for y in top..top + XTRANS_CELL {
            for x in left..left + XTRANS_CELL {
                counts[channel_index(self.colour_at(y, x))] += 1;
            }
        }
        counts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaPattern {
    Bayer(BayerPattern),
    XTrans(XTransPattern),
}

impl CfaPattern {
    pub fn parse(text: &str) -> Option<Self> {
        BayerPattern::parse(text)
            .map(Self::Bayer)
            .or_else(|| XTransPattern::parse(text).map(Self::XTrans))
    }

    pub fn name(&self) -> String {
        match self {
            Self::Bayer(pattern) => pattern.name().to_string(),
            Self::XTrans(pattern) => pattern.name(),
        }
    }

    pub fn period(&self) -> usize {
        match self {
            Self::Bayer(_) => BAYER_PERIOD,
            Self::XTrans(_) => XTRANS_PERIOD,
        }
    }

    pub fn colour_at(&self, y: usize, x: usize) -> u8 {
        match self {
            Self::Bayer(pattern) => pattern.colour_at(y, x),
            Self::XTrans(pattern) => pattern.colour_at(y, x),
        }
    }

    pub fn with_offsets(&self, xoff: i64, yoff: i64) -> Self {
        match self {
            Self::Bayer(pattern) => Self::Bayer(
                pattern.with_offsets(wrap(xoff, BAYER_PERIOD), wrap(yoff, BAYER_PERIOD)),
            ),
            Self::XTrans(pattern) => Self::XTrans(pattern.with_offsets(xoff, yoff)),
        }
    }

    pub fn flipped_rows(&self, height: usize) -> Self {
        match self {
            Self::Bayer(pattern) => {
                let last_row = (height % BAYER_PERIOD + BAYER_PERIOD - 1) % BAYER_PERIOD;
                Self::Bayer(pattern.with_offsets(0, last_row as i64))
            }
            Self::XTrans(pattern) => Self::XTrans(pattern.flipped_rows(height)),
        }
    }

    pub fn declared(header: &HduHeader) -> Option<Self> {
        PATTERN_CARDS
            .iter()
            .find_map(|key| header.get(key))
            .and_then(Self::parse)
    }

    pub fn detect(header: &HduHeader) -> Option<Self> {
        let base = Self::declared(header)?;
        let offset = |key: &str| {
            wrap(
                header.get_f64(key).unwrap_or(0.0).round() as i64,
                base.period(),
            )
        };
        let shifted = base.with_offsets(offset("XBAYROFF"), offset("YBAYROFF"));
        if !is_bottom_up(header) {
            return Some(shifted);
        }
        let rows = header
            .get_i64("ZNAXIS2")
            .or_else(|| header.get_i64("NAXIS2"))
            .filter(|&rows| rows > 0)?;
        Some(shifted.flipped_rows(usize::try_from(rows).ok()?))
    }

    pub fn is_xtrans(&self) -> bool {
        matches!(self, Self::XTrans(_))
    }

    pub fn kind(&self) -> CfaKind {
        match self {
            Self::Bayer(_) => CfaKind::Bayer,
            Self::XTrans(_) => CfaKind::XTrans,
        }
    }

    pub fn aligned_origin(&self) -> (usize, usize) {
        match self {
            Self::Bayer(_) => (0, 0),
            Self::XTrans(pattern) => pattern.aligned_origin(),
        }
    }

    pub fn bin_factor(&self) -> usize {
        match self {
            Self::Bayer(_) => BAYER_CELL,
            Self::XTrans(_) => XTRANS_CELL,
        }
    }
}

pub fn remove_cfa_cards(header: &mut HduHeader) {
    for key in CFA_CARDS {
        header.remove(key);
    }
}

fn cell_grid(dims: (usize, usize), origin: (usize, usize), cell: usize) -> (usize, usize) {
    (
        dims.0.saturating_sub(origin.0) / cell,
        dims.1.saturating_sub(origin.1) / cell,
    )
}

fn cell_corner(origin: (usize, usize), cell: usize, (cy, cx): (usize, usize)) -> (usize, usize) {
    (origin.0 + cy * cell, origin.1 + cx * cell)
}

fn cell_view(img: &Array2<f32>, (top, left): (usize, usize), cell: usize) -> ArrayView2<'_, f32> {
    img.slice(s![top..top + cell, left..left + cell])
}

fn cell_mean(samples: ArrayView2<'_, f32>) -> f32 {
    let mut sum = 0.0f64;
    for &value in samples {
        if !value.is_finite() {
            return f32::NAN;
        }
        sum += value as f64;
    }
    (sum / samples.len() as f64) as f32
}

pub fn luminance_bin(img: &Array2<f32>, pattern: &CfaPattern) -> Array2<f32> {
    let origin = pattern.aligned_origin();
    let cell = pattern.bin_factor();
    let mut binned = Array2::<f32>::zeros(cell_grid(img.dim(), origin, cell));
    Zip::indexed(&mut binned).par_for_each(|index, value| {
        *value = cell_mean(cell_view(img, cell_corner(origin, cell, index), cell));
    });
    binned
}

fn site_means(img: &Array2<f32>, pattern: &XTransPattern, corner: (usize, usize)) -> [f32; 3] {
    let mut sums = [0.0f64; 3];
    let mut counts = [0u32; 3];
    for ((dy, dx), &value) in cell_view(img, corner, XTRANS_CELL).indexed_iter() {
        if value.is_finite() {
            let channel = channel_index(pattern.colour_at(corner.0 + dy, corner.1 + dx));
            sums[channel] += value as f64;
            counts[channel] += 1;
        }
    }
    std::array::from_fn(|channel| {
        if counts[channel] > 0 {
            (sums[channel] / counts[channel] as f64) as f32
        } else {
            f32::NAN
        }
    })
}

pub fn debayer_xtrans_superpixel(
    img: &Array2<f32>,
    pattern: &XTransPattern,
) -> (Array2<f32>, Array2<f32>, Array2<f32>) {
    let origin = pattern.aligned_origin();
    let dims = cell_grid(img.dim(), origin, XTRANS_CELL);
    let mut red = Array2::<f32>::zeros(dims);
    let mut green = Array2::<f32>::zeros(dims);
    let mut blue = Array2::<f32>::zeros(dims);
    Zip::indexed(&mut red)
        .and(&mut green)
        .and(&mut blue)
        .par_for_each(|index, r, g, b| {
            let [mean_r, mean_g, mean_b] =
                site_means(img, pattern, cell_corner(origin, XTRANS_CELL, index));
            *r = mean_r;
            *g = mean_g;
            *b = mean_b;
        });
    (red, green, blue)
}

fn set_integer(header: &mut HduHeader, key: &str, value: impl ToString) {
    header.set(key, value.to_string());
    if let Some(keys) = header.string_keys.as_mut() {
        keys.remove(key);
    }
}

fn scale_binning(
    header: &mut HduHeader,
    source: &HduHeader,
    keys: &[&'static str],
    original: usize,
    target: usize,
) {
    if !original.is_multiple_of(target) {
        return;
    }
    let Ok(factor) = i64::try_from(original / target) else {
        return;
    };
    let declared = first_i64(source, keys).map_or(1, |(binning, _)| binning);
    for (rank, &key) in keys.iter().enumerate() {
        let binning = match first_i64(source, &[key]) {
            Some((binning, _)) => binning,
            None if rank == 0 => declared,
            None => continue,
        };
        match binning.checked_mul(factor) {
            Some(scaled) => set_integer(header, key, scaled),
            None => header.remove(key),
        }
    }
}

fn scale_pixel_size(
    header: &mut HduHeader,
    source: &HduHeader,
    keys: &[&'static str],
    original: usize,
    target: usize,
) {
    if !original.is_multiple_of(target) {
        return;
    }
    let factor = (original / target) as f64;
    for &key in keys {
        if let Some(pitch) = source.get_f64(key).filter(|v| v.is_finite() && *v > 0.0) {
            header.set_f64(key, pitch * factor);
        }
    }
}

pub fn scaled_header(
    source: &HduHeader,
    original_dims: (usize, usize),
    target_dims: (usize, usize),
) -> HduHeader {
    let mut header = source.clone();
    remove_cfa_cards(&mut header);
    let (original_rows, original_cols) = original_dims;
    let (target_rows, target_cols) = target_dims;
    if [original_rows, original_cols, target_rows, target_cols].contains(&0) {
        return header;
    }
    for (key, value) in compute_wcs_updates(source, original_dims, target_dims) {
        if !key.starts_with("NAXIS") {
            header.set_f64(&key, value);
        }
    }
    set_integer(&mut header, "NAXIS1", target_cols);
    set_integer(&mut header, "NAXIS2", target_rows);
    scale_binning(
        &mut header,
        source,
        &BINNING_KEYS,
        original_cols,
        target_cols,
    );
    scale_binning(
        &mut header,
        source,
        &Y_BINNING_KEYS,
        original_rows,
        target_rows,
    );
    scale_pixel_size(
        &mut header,
        source,
        &PIXEL_SIZE_UM_KEYS,
        original_cols,
        target_cols,
    );
    scale_pixel_size(
        &mut header,
        source,
        &Y_PIXEL_SIZE_UM_KEYS,
        original_rows,
        target_rows,
    );
    header
}

#[cfg(test)]
pub(crate) mod test_fixtures {
    use ndarray::Array2;

    pub(crate) const XTRANS_LAYOUT: &str = "GBGGRGRGRBGBGBGGRGGRGGBGBGBRGRGRGGBG";

    pub(crate) fn site_code(letter: u8) -> u8 {
        match letter.to_ascii_uppercase() {
            b'R' => 0,
            b'G' => 1,
            _ => 2,
        }
    }

    pub(crate) fn xtrans_sensor(rows: usize, cols: usize) -> Array2<u8> {
        let layout = XTRANS_LAYOUT.as_bytes();
        Array2::from_shape_fn((rows, cols), |(y, x)| {
            site_code(layout[(y % 6) * 6 + x % 6])
        })
    }

    pub(crate) fn xtrans_flipped(height: usize, width: usize) -> Array2<u8> {
        let sensor = xtrans_sensor(height, width);
        Array2::from_shape_fn((height, width), |(y, x)| sensor[[height - 1 - y, x]])
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::test_fixtures::{site_code, xtrans_flipped, XTRANS_LAYOUT};
    use super::*;

    const BAYERS: [BayerPattern; 4] = [
        BayerPattern::Rggb,
        BayerPattern::Bggr,
        BayerPattern::Grbg,
        BayerPattern::Gbrg,
    ];
    const RED_LEVEL: f32 = 8.0;
    const GREEN_LEVEL: f32 = 5.0;
    const BLUE_LEVEL: f32 = 2.0;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, (*value).to_string());
        }
        h
    }

    fn published() -> XTransPattern {
        XTransPattern::parse(XTRANS_LAYOUT).expect("the published X-Trans layout parses")
    }

    fn all_xtrans_phases() -> Vec<XTransPattern> {
        let base = published();
        (0..6)
            .flat_map(|dy| (0..6).map(move |dx| base.with_offsets(dx, dy)))
            .collect()
    }

    fn site_level(colour: u8) -> f32 {
        match colour {
            0 => RED_LEVEL,
            1 => GREEN_LEVEL,
            _ => BLUE_LEVEL,
        }
    }

    fn constant_mosaic(pattern: &CfaPattern, rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| site_level(pattern.colour_at(y, x)))
    }

    fn block_counts(pattern: &XTransPattern, top: usize, left: usize) -> [usize; 3] {
        let mut counts = [0usize; 3];
        for y in top..top + 3 {
            for x in left..left + 3 {
                counts[pattern.colour_at(y, x) as usize] += 1;
            }
        }
        counts
    }

    fn grid_is_balanced(pattern: &XTransPattern, (oy, ox): (usize, usize)) -> bool {
        [(oy, ox), (oy + 3, ox), (oy, ox + 3), (oy + 3, ox + 3)]
            .iter()
            .all(|&(top, left)| block_counts(pattern, top, left) == [2, 5, 2])
    }

    fn origin_shifted_phase() -> XTransPattern {
        all_xtrans_phases()
            .into_iter()
            .find(|p| p.aligned_origin() != (0, 0))
            .expect("some phase needs a non-zero origin")
    }

    fn within_one_ulp(a: f32, b: f32) -> bool {
        a == b || (a.to_bits() as i64 - b.to_bits() as i64).abs() <= 1
    }

    #[test]
    fn parse_accepts_the_four_bayer_strings_and_the_published_xtrans_layout() {
        assert_eq!(
            CfaPattern::parse("RGGB"),
            Some(CfaPattern::Bayer(BayerPattern::Rggb))
        );
        assert_eq!(
            CfaPattern::parse("'bggr '"),
            Some(CfaPattern::Bayer(BayerPattern::Bggr))
        );
        for bayer in BAYERS {
            let pattern = CfaPattern::parse(bayer.name()).expect("bayer parses");
            assert_eq!(pattern, CfaPattern::Bayer(bayer));
            assert_eq!(pattern.name(), bayer.name());
            assert_eq!(CfaPattern::parse(&pattern.name()), Some(pattern));
            assert_eq!(
                (pattern.period(), pattern.bin_factor()),
                (2, 2),
                "{bayer:?}"
            );
            assert_eq!(pattern.kind(), CfaKind::Bayer);
            assert!(!pattern.is_xtrans());
            assert_eq!(pattern.aligned_origin(), (0, 0));
        }

        let xtrans = CfaPattern::parse(XTRANS_LAYOUT).expect("the published layout parses");
        assert_eq!(xtrans, CfaPattern::XTrans(published()));
        assert_eq!(xtrans.name(), XTRANS_LAYOUT);
        assert_eq!(xtrans.name().len(), 36);
        assert_eq!(CfaPattern::parse(&xtrans.name()), Some(xtrans));
        assert_eq!((xtrans.period(), xtrans.bin_factor()), (6, 3));
        assert_eq!(xtrans.kind(), CfaKind::XTrans);
        assert!(xtrans.is_xtrans());
        let quoted_lowercase = format!("'{} '", XTRANS_LAYOUT.to_lowercase());
        assert_eq!(CfaPattern::parse(&quoted_lowercase), Some(xtrans));

        let bayer_tiled = "RGRGRGGBGBGB".repeat(3);
        let foreign_letter = format!("{}X", &XTRANS_LAYOUT[..35]);
        let non_ascii = format!("{}é", &XTRANS_LAYOUT[..34]);
        for rejected in [
            "",
            "XTRANS",
            "MONO",
            "RGB",
            &XTRANS_LAYOUT[..35],
            &foreign_letter,
            &"G".repeat(36),
            &bayer_tiled,
            &non_ascii,
        ] {
            assert_eq!(CfaPattern::parse(rejected), None, "{rejected:?}");
        }
    }

    #[test]
    fn colour_at_matches_the_bayer_cell_for_all_four_patterns() {
        for bayer in BAYERS {
            let pattern = CfaPattern::Bayer(bayer);
            let sites = bayer.name().as_bytes();
            for y in 0..6 {
                for x in 0..6 {
                    let expected = site_code(sites[(y % 2) * 2 + x % 2]);
                    assert_eq!(bayer.colour_at(y, x), expected, "{bayer:?} at ({y}, {x})");
                    assert_eq!(pattern.colour_at(y, x), expected, "{bayer:?} at ({y}, {x})");
                }
            }
        }
    }

    #[test]
    fn xtrans_colour_at_follows_the_published_rows() {
        let rows = ["GBGGRG", "RGRBGB", "GBGGRG", "GRGGBG", "BGBRGR", "GRGGBG"];
        assert_eq!(rows.concat(), XTRANS_LAYOUT);
        let pattern = CfaPattern::XTrans(published());
        for y in 0..12 {
            for x in 0..12 {
                let expected = site_code(rows[y % 6].as_bytes()[x % 6]);
                assert_eq!(pattern.colour_at(y, x), expected, "({y}, {x})");
            }
        }
    }

    #[test]
    fn xtrans_offsets_wrap_mod_6() {
        let base = CfaPattern::XTrans(published());
        assert_eq!(base.with_offsets(0, 0), base);
        assert_eq!(base.with_offsets(6, -6), base);
        assert_eq!(base.with_offsets(-1, 0), base.with_offsets(5, 0));
        assert_eq!(base.with_offsets(13, 8), base.with_offsets(1, 2));
        assert_eq!(
            base.with_offsets(i64::MAX, i64::MIN),
            base.with_offsets(i64::MAX.rem_euclid(6), i64::MIN.rem_euclid(6))
        );
        let shifted = base.with_offsets(1, 2);
        assert_ne!(shifted, base);
        for y in 0..12 {
            for x in 0..12 {
                assert_eq!(
                    shifted.colour_at(y, x),
                    base.colour_at(y + 2, x + 1),
                    "({y}, {x})"
                );
            }
        }
        let quoted = format!("'{XTRANS_LAYOUT}'");
        let detected = CfaPattern::detect(&header(&[
            ("BAYERPAT", &quoted),
            ("XBAYROFF", "7"),
            ("YBAYROFF", "-4"),
        ]));
        assert_eq!(detected, Some(base.with_offsets(1, 2)));
        let huge = header(&[
            ("BAYERPAT", &quoted),
            ("XBAYROFF", "1e300"),
            ("YBAYROFF", "-1e300"),
        ]);
        assert!(CfaPattern::detect(&huge).is_some());
    }

    #[test]
    fn detect_bottom_up_is_a_reflection() {
        let base = CfaPattern::XTrans(published());
        let quoted = format!("'{XTRANS_LAYOUT}'");
        let mut shift_rule_failures = 0;
        for height in [6usize, 7, 9, 10, 4160] {
            let stored = xtrans_flipped(height, 12);
            let rows = height.to_string();
            let detected = CfaPattern::detect(&header(&[
                ("BAYERPAT", &quoted),
                ("ROWORDER", "'BOTTOM-UP'"),
                ("NAXIS2", &rows),
            ]))
            .expect("bottom-up X-Trans with a height is detected");
            for ((y, x), &truth) in stored.indexed_iter() {
                assert_eq!(
                    detected.colour_at(y, x),
                    truth,
                    "H={height} at stored ({y}, {x})"
                );
            }
            let shift_rule = base.with_offsets(0, (height as i64 - 1).rem_euclid(6));
            if stored
                .indexed_iter()
                .any(|((y, x), &truth)| shift_rule.colour_at(y, x) != truth)
            {
                shift_rule_failures += 1;
            }
        }
        assert!(
            shift_rule_failures > 0,
            "the fixture must tell a reflection from a shift"
        );

        let height = 4160usize;
        let detected = CfaPattern::detect(&header(&[
            ("COLORTYP", &quoted),
            ("XBAYROFF", "1"),
            ("YBAYROFF", "2"),
            ("ROWORDER", "BOTTOM-UP"),
            ("ZNAXIS2", "4160"),
            ("NAXIS2", "17"),
        ]))
        .expect("ZNAXIS2 gives the height");
        for y in 0..24 {
            for x in 0..12 {
                let sensor_row = height - 1 - y;
                assert_eq!(
                    detected.colour_at(y, x),
                    base.colour_at(sensor_row + 2, x + 1),
                    "offsets then reflection at ({y}, {x})"
                );
            }
        }

        let top_down = header(&[
            ("BAYERPAT", &quoted),
            ("ROWORDER", "TOP-DOWN"),
            ("NAXIS2", "7"),
        ]);
        assert_eq!(CfaPattern::detect(&top_down), Some(base));
        let no_height = header(&[("BAYERPAT", &quoted), ("ROWORDER", "BOTTOM-UP")]);
        assert_eq!(CfaPattern::detect(&no_height), None);
        let zero_height = header(&[
            ("BAYERPAT", &quoted),
            ("ROWORDER", "BOTTOM-UP"),
            ("NAXIS2", "0"),
        ]);
        assert_eq!(CfaPattern::detect(&zero_height), None);
        assert_eq!(CfaPattern::detect(&header(&[("OBJECT", "M42")])), None);
    }

    #[test]
    fn cfa_detect_equals_bayer_detect_on_every_bayer_header() {
        let offsets = [
            None,
            Some("0"),
            Some("1"),
            Some("3"),
            Some("-1"),
            Some("1e300"),
        ];
        let row_orders = [None, Some("'TOP-DOWN'"), Some("'BOTTOM-UP'")];
        let heights = [None, Some("3519"), Some("3520")];
        let mut compared = 0;
        for bayer in BAYERS {
            for key in ["BAYERPAT", "COLORTYP"] {
                for xoff in offsets {
                    for yoff in offsets {
                        for row_order in row_orders {
                            for rows in heights {
                                let mut cards = vec![(key, bayer.name())];
                                cards.extend(xoff.map(|v| ("XBAYROFF", v)));
                                cards.extend(yoff.map(|v| ("YBAYROFF", v)));
                                cards.extend(row_order.map(|v| ("ROWORDER", v)));
                                cards.extend(rows.map(|v| ("NAXIS2", v)));
                                let h = header(&cards);
                                assert_eq!(
                                    CfaPattern::detect(&h),
                                    BayerPattern::detect(&h).map(CfaPattern::Bayer),
                                    "{cards:?}"
                                );
                                compared += 1;
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(compared, 4 * 2 * 6 * 6 * 3 * 3);
        let rggb_bottom_up_even = header(&[
            ("BAYERPAT", "'RGGB'"),
            ("ROWORDER", "'BOTTOM-UP'"),
            ("NAXIS2", "3520"),
        ]);
        assert_eq!(
            CfaPattern::detect(&rggb_bottom_up_even),
            Some(CfaPattern::Bayer(BayerPattern::Gbrg))
        );
    }

    #[test]
    fn declared_reads_the_card_without_offsets_or_row_order() {
        let quoted = format!("'{XTRANS_LAYOUT}'");
        let xtrans = CfaPattern::XTrans(published());
        let rggb = CfaPattern::Bayer(BayerPattern::Rggb);
        let cases: [(Vec<(&str, &str)>, Option<CfaPattern>); 7] = [
            (
                vec![("BAYERPAT", "'RGGB'"), ("ROWORDER", "'BOTTOM-UP'")],
                Some(rggb),
            ),
            (
                vec![
                    ("BAYERPAT", "'RGGB'"),
                    ("ROWORDER", "'BOTTOM-UP'"),
                    ("NAXIS2", "3520"),
                    ("XBAYROFF", "1"),
                ],
                Some(rggb),
            ),
            (
                vec![("BAYERPAT", &quoted), ("ROWORDER", "'BOTTOM-UP'")],
                Some(xtrans),
            ),
            (
                vec![
                    ("COLORTYP", &quoted),
                    ("ROWORDER", "'BOTTOM-UP'"),
                    ("NAXIS2", "4160"),
                    ("YBAYROFF", "2"),
                ],
                Some(xtrans),
            ),
            (vec![("BAYERPAT", "'MONO'"), ("COLORTYP", "'RGGB'")], None),
            (vec![("COLORTYP", "'RGB'")], None),
            (vec![("OBJECT", "'M42'")], None),
        ];
        for (cards, expected) in cases {
            assert_eq!(CfaPattern::declared(&header(&cards)), expected, "{cards:?}");
        }
    }

    #[test]
    fn aligned_origin_is_balanced_for_all_36_phases() {
        let mut origins = HashSet::new();
        let mut unbalanced_at_zero = 0;
        for phase in all_xtrans_phases() {
            for pattern in [phase, phase.flipped_rows(7), phase.flipped_rows(4160)] {
                let origin = pattern.aligned_origin();
                assert!(origin.0 < 3 && origin.1 < 3, "{origin:?}");
                assert!(
                    grid_is_balanced(&pattern, origin),
                    "{} at {origin:?}",
                    pattern.name()
                );
                let first = (0..3)
                    .flat_map(|dy| (0..3).map(move |dx| (dy, dx)))
                    .find(|&o| grid_is_balanced(&pattern, o));
                assert_eq!(
                    Some(origin),
                    first,
                    "rows-first rule for {}",
                    pattern.name()
                );
                assert_eq!(CfaPattern::XTrans(pattern).aligned_origin(), origin);
                origins.insert(origin);
                if block_counts(&pattern, 0, 0) != [2, 5, 2] {
                    unbalanced_at_zero += 1;
                }
            }
        }
        assert!(origins.len() > 1, "{origins:?}");
        assert!(
            unbalanced_at_zero > 0,
            "a fixed (0, 0) grid mixes colours on some phase"
        );
        assert_eq!(published().aligned_origin(), (0, 0));
    }

    #[test]
    fn luminance_bin_of_the_constant_fixture_is_five_for_bayer_and_xtrans() {
        for bayer in BAYERS {
            let pattern = CfaPattern::Bayer(bayer);
            let binned = luminance_bin(&constant_mosaic(&pattern, 12, 12), &pattern);
            assert_eq!(binned.dim(), (6, 6), "{bayer:?}");
            assert!(binned.iter().all(|&v| v == 5.0), "{bayer:?}: {binned:?}");
        }

        let shifted = origin_shifted_phase();
        let (oy, ox) = shifted.aligned_origin();
        let pattern = CfaPattern::XTrans(shifted);
        let binned = luminance_bin(&constant_mosaic(&pattern, 18, 18), &pattern);
        assert_eq!(binned.dim(), ((18 - oy) / 3, (18 - ox) / 3));
        assert!(binned.iter().all(|&v| v == 5.0), "{binned:?}");

        for phase in all_xtrans_phases() {
            let pattern = CfaPattern::XTrans(phase);
            let binned = luminance_bin(&constant_mosaic(&pattern, 18, 18), &pattern);
            assert!(!binned.is_empty());
            assert!(binned.iter().all(|&v| v == 5.0), "{}", phase.name());
        }
    }

    #[test]
    fn luminance_bin_gives_the_x_t3_frame_three_by_three_cells() {
        let frame = Array2::<f32>::zeros((4160, 6240));
        let pattern = CfaPattern::XTrans(published());
        assert_eq!(luminance_bin(&frame, &pattern).dim(), (1386, 2080));
        let bottom_up = CfaPattern::XTrans(published().flipped_rows(4160));
        let (oy, ox) = bottom_up.aligned_origin();
        let dims = luminance_bin(&frame, &bottom_up).dim();
        assert_eq!(dims, ((4160 - oy) / 3, (6240 - ox) / 3));
        assert_eq!((oy, ox), (0, 1));
        assert_eq!(dims, (1386, 2079));
    }

    #[test]
    fn luminance_bin_of_a_gradient_is_the_hand_mean_within_one_ulp() {
        let gradient = Array2::from_shape_fn((23, 29), |(y, x)| ((y * 37 + x * 11) % 1009) as f32);
        let patterns = [
            CfaPattern::Bayer(BayerPattern::Grbg),
            CfaPattern::XTrans(published()),
            CfaPattern::XTrans(origin_shifted_phase()),
        ];
        for pattern in patterns {
            let (oy, ox) = pattern.aligned_origin();
            let cell = pattern.bin_factor();
            let binned = luminance_bin(&gradient, &pattern);
            assert_eq!(binned.dim(), ((23 - oy) / cell, (29 - ox) / cell));
            for ((cy, cx), &value) in binned.indexed_iter() {
                let (top, left) = (oy + cy * cell, ox + cx * cell);
                let mut sum = 0.0f64;
                for y in top..top + cell {
                    for x in left..left + cell {
                        sum += gradient[[y, x]] as f64;
                    }
                }
                let hand = (sum / (cell * cell) as f64) as f32;
                assert!(
                    within_one_ulp(value, hand),
                    "{} cell ({cy}, {cx}): {value} vs {hand}",
                    pattern.name()
                );
            }
        }
    }

    #[test]
    fn a_non_finite_sample_makes_its_cell_nan() {
        let bayer = CfaPattern::Bayer(BayerPattern::Rggb);
        let mut img = constant_mosaic(&bayer, 8, 8);
        img[[3, 2]] = f32::NAN;
        img[[6, 7]] = f32::INFINITY;
        let binned = luminance_bin(&img, &bayer);
        for ((cy, cx), &v) in binned.indexed_iter() {
            if (cy, cx) == (1, 1) || (cy, cx) == (3, 3) {
                assert!(v.is_nan(), "cell ({cy}, {cx}) = {v}");
            } else {
                assert_eq!(v, 5.0, "cell ({cy}, {cx})");
            }
        }

        let shifted = origin_shifted_phase();
        let (oy, ox) = shifted.aligned_origin();
        let xtrans = CfaPattern::XTrans(shifted);
        let mut img = constant_mosaic(&xtrans, 18, 18);
        img[[oy + 4, ox + 5]] = f32::NAN;
        let binned = luminance_bin(&img, &xtrans);
        for ((cy, cx), &v) in binned.indexed_iter() {
            if (cy, cx) == (1, 1) {
                assert!(v.is_nan(), "cell ({cy}, {cx}) = {v}");
            } else {
                assert_eq!(v, 5.0, "cell ({cy}, {cx})");
            }
        }
    }

    #[test]
    fn luminance_bin_trims_the_remainder_rows_and_columns() {
        let bayer = CfaPattern::Bayer(BayerPattern::Gbrg);
        let binned = luminance_bin(&constant_mosaic(&bayer, 13, 11), &bayer);
        assert_eq!(binned.dim(), (6, 5));
        assert!(binned.iter().all(|&v| v == 5.0));

        let xtrans = CfaPattern::XTrans(published());
        let binned = luminance_bin(&constant_mosaic(&xtrans, 20, 19), &xtrans);
        assert_eq!(binned.dim(), (6, 6));
        assert!(binned.iter().all(|&v| v == 5.0));

        let shifted = origin_shifted_phase();
        let (oy, ox) = shifted.aligned_origin();
        let xtrans = CfaPattern::XTrans(shifted);
        let binned = luminance_bin(&constant_mosaic(&xtrans, 20, 19), &xtrans);
        assert_eq!(binned.dim(), ((20 - oy) / 3, (19 - ox) / 3));
        assert!(binned.iter().all(|&v| v == 5.0));

        assert_eq!(luminance_bin(&Array2::zeros((1, 1)), &bayer).dim(), (0, 0));
        assert_eq!(luminance_bin(&Array2::zeros((2, 2)), &xtrans).dim(), (0, 0));
        assert!(luminance_bin(&Array2::zeros((0, 7)), &xtrans).is_empty());
    }

    #[test]
    fn xtrans_superpixel_returns_the_site_constants_per_aligned_cell() {
        for phase in all_xtrans_phases() {
            let pattern = CfaPattern::XTrans(phase);
            let img = constant_mosaic(&pattern, 19, 20);
            let (r, g, b) = debayer_xtrans_superpixel(&img, &phase);
            let expected_dims = luminance_bin(&img, &pattern).dim();
            assert!(expected_dims.0 > 0 && expected_dims.1 > 0);
            for (plane, level) in [(&r, RED_LEVEL), (&g, GREEN_LEVEL), (&b, BLUE_LEVEL)] {
                assert_eq!(plane.dim(), expected_dims, "{}", phase.name());
                assert!(
                    plane.iter().all(|&v| v == level),
                    "{}: {plane:?}",
                    phase.name()
                );
            }
        }

        let phase = origin_shifted_phase();
        let (oy, ox) = phase.aligned_origin();
        let pattern = CfaPattern::XTrans(phase);
        let mut img = constant_mosaic(&pattern, 18, 18);
        let cell_sites: Vec<(usize, usize, u8)> = (oy..oy + 3)
            .flat_map(|y| (ox..ox + 3).map(move |x| (y, x)))
            .map(|(y, x)| (y, x, pattern.colour_at(y, x)))
            .collect();
        let first_green = cell_sites.iter().find(|s| s.2 == 1).expect("a green site");
        img[[first_green.0, first_green.1]] = f32::NAN;
        for &(y, x, colour) in &cell_sites {
            if colour == 0 {
                img[[y, x]] = f32::NAN;
            }
        }
        let (r, g, b) = debayer_xtrans_superpixel(&img, &phase);
        assert!(r[[0, 0]].is_nan());
        assert_eq!(g[[0, 0]], GREEN_LEVEL);
        assert_eq!(b[[0, 0]], BLUE_LEVEL);
        assert_eq!(r[[1, 1]], RED_LEVEL);
    }

    fn wcs_header() -> HduHeader {
        header(&[
            ("NAXIS1", "16"),
            ("NAXIS2", "16"),
            ("BAYERPAT", "'RGGB'"),
            ("COLORTYP", "'RGGB'"),
            ("XBAYROFF", "1"),
            ("YBAYROFF", "0"),
            ("CRPIX1", "8.5"),
            ("CRPIX2", "8.5"),
            ("CD1_1", "1.0E-4"),
            ("CD1_2", "0.0"),
            ("CD2_1", "0.0"),
            ("CD2_2", "1.0E-4"),
            ("A_ORDER", "2"),
            ("A_2_0", "1.0E-7"),
            ("LTV1", "10"),
        ])
    }

    fn formatted(key: &str, value: f64) -> String {
        let mut h = HduHeader::empty();
        h.set_f64(key, value);
        h.get(key).map(str::to_string).unwrap_or_default()
    }

    #[test]
    fn scaled_header_scales_wcs_sip_ltv_and_binning_and_strips_cfa_cards() {
        let source = wcs_header();
        let out = scaled_header(&source, (16, 16), (8, 8));
        for key in ["BAYERPAT", "COLORTYP", "XBAYROFF", "YBAYROFF"] {
            assert!(out.get(key).is_none(), "{key} kept");
        }
        assert_eq!(out.get_f64("CRPIX1"), Some(4.5));
        assert_eq!(out.get_f64("CRPIX2"), Some(4.5));
        assert_eq!(out.get_f64("CD1_1"), Some(2.0e-4));
        assert_eq!(out.get_f64("CD2_2"), Some(2.0e-4));
        assert_eq!(out.get_f64("A_2_0"), Some(2.0e-7));
        assert_eq!(out.get_f64("LTV1"), Some(5.25));
        assert_eq!(out.get_f64("LTM1_1"), Some(0.5));
        assert_eq!(out.get("XBINNING"), Some("2"));
        assert_eq!(out.get("YBINNING"), Some("2"));
        assert_eq!(out.get("NAXIS1"), Some("8"));
        assert_eq!(out.get("NAXIS2"), Some("8"));
        for (key, value) in compute_updates(&source) {
            if key.starts_with("NAXIS") {
                continue;
            }
            assert_eq!(
                out.get(&key).map(str::to_string),
                Some(formatted(&key, value)),
                "{key}"
            );
        }

        let binned = header(&[("XBINNING", "2"), ("YBINNING", "2")]);
        let out = scaled_header(&binned, (16, 16), (8, 8));
        assert_eq!(
            (out.get("XBINNING"), out.get("YBINNING")),
            (Some("4"), Some("4"))
        );
        let sharpcap = header(&[("BINX", "2"), ("BINY", "3")]);
        let out = scaled_header(&sharpcap, (16, 16), (8, 8));
        assert_eq!(
            (out.get("XBINNING"), out.get("YBINNING")),
            (Some("4"), Some("6"))
        );
        let out = scaled_header(&HduHeader::empty(), (18, 21), (6, 7));
        assert_eq!(
            (out.get("XBINNING"), out.get("YBINNING")),
            (Some("3"), Some("3"))
        );

        let untouched = scaled_header(&source, (0, 0), (0, 0));
        assert!(untouched.get("BAYERPAT").is_none());
        assert_eq!(untouched.get("CRPIX1"), Some("8.5"));
        assert!(untouched.get("XBINNING").is_none());
    }

    #[test]
    fn scaled_header_scales_every_binning_card_and_leaves_none_stale() {
        let cards = |out: &HduHeader| {
            ["XBINNING", "BINX", "YBINNING", "BINY"].map(|key| out.get(key).map(str::to_string))
        };
        let some = |value: &str| Some(value.to_string());

        let sharpcap = header(&[("BINX", "2"), ("BINY", "3")]);
        assert_eq!(
            cards(&scaled_header(&sharpcap, (16, 16), (8, 8))),
            [some("4"), some("4"), some("6"), some("6")]
        );

        let both = header(&[
            ("XBINNING", "2"),
            ("BINX", "2"),
            ("YBINNING", "1"),
            ("BINY", "1.0"),
        ]);
        assert_eq!(
            cards(&scaled_header(&both, (18, 21), (6, 7))),
            [some("6"), some("6"), some("3"), some("3")]
        );

        let nina = header(&[("XBINNING", "2"), ("YBINNING", "2")]);
        assert_eq!(
            cards(&scaled_header(&nina, (16, 16), (8, 8))),
            [some("4"), None, some("4"), None]
        );

        let text_binning = header(&[("XBINNING", "'2x2'"), ("BINX", "2")]);
        let out = scaled_header(&text_binning, (16, 16), (8, 8));
        assert_eq!(
            (out.get("XBINNING"), out.get("BINX")),
            (Some("4"), Some("4"))
        );

        let uneven = header(&[("BINX", "2"), ("BINY", "2")]);
        assert_eq!(
            cards(&scaled_header(&uneven, (17, 17), (8, 8))),
            [None, some("2"), None, some("2")]
        );
    }

    #[test]
    fn scaled_header_scales_the_pixel_pitch_with_the_binning() {
        let pitch = |out: &HduHeader, key: &str| out.get_f64(key);
        let close =
            |found: Option<f64>, expected: f64| found.is_some_and(|v| (v - expected).abs() < 1e-9);

        let nina = header(&[
            ("XBINNING", "1"),
            ("YBINNING", "1"),
            ("XPIXSZ", "3.76"),
            ("YPIXSZ", "3.76"),
            ("FOCALLEN", "400."),
        ]);
        let out = scaled_header(&nina, (16, 18), (8, 6));
        assert!(
            close(pitch(&out, "XPIXSZ"), 3.76 * 3.0),
            "{:?}",
            out.get("XPIXSZ")
        );
        assert!(
            close(pitch(&out, "YPIXSZ"), 3.76 * 2.0),
            "{:?}",
            out.get("YPIXSZ")
        );
        let cards =
            crate::core::stacking::frame_cards::frame_cards_from_header("binned.fits", &out);
        assert!(cards
            .pixel_size_um
            .is_some_and(|(um, key)| key == "XPIXSZ" && (um - 3.76 * 3.0).abs() < 1e-9));
        assert_eq!((cards.xbinning, cards.ybinning), (Some(3), Some(2)));

        let sharpcap = header(&[("PIXSIZE1", "2.9"), ("PIXSIZE2", "2.9"), ("BINX", "1")]);
        let out = scaled_header(&sharpcap, (16, 16), (8, 8));
        assert!(close(pitch(&out, "PIXSIZE1"), 5.8) && close(pitch(&out, "PIXSIZE2"), 5.8));
        assert!(out.get("XPIXSZ").is_none() && out.get("YPIXSZ").is_none());

        let uneven = scaled_header(&nina, (17, 17), (8, 8));
        assert_eq!(
            (uneven.get("XPIXSZ"), uneven.get("XBINNING")),
            (Some("3.76"), Some("1"))
        );
    }

    fn compute_updates(source: &HduHeader) -> Vec<(String, f64)> {
        crate::core::imaging::resample::compute_wcs_updates(source, (16, 16), (8, 8))
    }

    fn legacy_superpixel_header(source: &HduHeader) -> HduHeader {
        let mut h = source.clone();
        h.remove("BAYERPAT");
        h.remove("XBAYROFF");
        h.remove("YBAYROFF");
        h.remove("COLORTYP");
        for key in ["CRPIX1", "CRPIX2"] {
            if let Some(v) = h.get_f64(key) {
                h.set_f64(key, (v - 0.5) / 2.0 + 0.5);
            }
        }
        for key in ["CD1_1", "CD1_2", "CD2_1", "CD2_2", "CDELT1", "CDELT2"] {
            if let Some(v) = h.get_f64(key) {
                h.set_f64(key, v * 2.0);
            }
        }
        h
    }

    #[test]
    fn scaled_header_equals_the_legacy_superpixel_wcs_bit_exactly() {
        let source = header(&[
            ("BAYERPAT", "'RGGB'"),
            ("CRPIX1", "8.5"),
            ("CRPIX2", "7.123456789012345"),
            ("CD1_1", "-1.234567890123456E-4"),
            ("CD1_2", "3.3E-6"),
            ("CD2_1", "-2.1E-6"),
            ("CD2_2", "1.0E-4"),
        ]);
        let legacy = legacy_superpixel_header(&source);
        let scaled = scaled_header(&source, (16, 16), (8, 8));
        for key in ["CRPIX1", "CRPIX2", "CD1_1", "CD1_2", "CD2_1", "CD2_2"] {
            assert!(scaled.get(key).is_some(), "{key} missing");
            assert_eq!(scaled.get(key), legacy.get(key), "{key}");
        }
        assert!(scaled.get("BAYERPAT").is_none() && legacy.get("BAYERPAT").is_none());
    }

    #[test]
    fn remove_cfa_cards_strips_the_four_cfa_cards_only() {
        let mut h = wcs_header();
        h.set("ROWORDER", "'BOTTOM-UP'".to_string());
        remove_cfa_cards(&mut h);
        for key in ["BAYERPAT", "COLORTYP", "XBAYROFF", "YBAYROFF"] {
            assert!(h.get(key).is_none(), "{key} kept");
        }
        assert_eq!(h.get("ROWORDER"), Some("'BOTTOM-UP'"));
        assert_eq!(h.get("CRPIX1"), Some("8.5"));
    }
}
