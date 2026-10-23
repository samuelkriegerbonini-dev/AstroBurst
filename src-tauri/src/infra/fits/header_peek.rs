use anyhow::{bail, Context, Result};

use super::compress::is_compressed_image_hdu;
use super::reader::{parse_header_at, ParsedHdu};
use crate::core::astrometry::wcs::{pixel_center, pixel_edge_corners};
use crate::infra::wcs_source::header_wcs;
use crate::types::constants::BLOCK_SIZE;
use crate::types::header::{HduHeader, MAX_FITS_AXES};

const CARD_BYTES: usize = 80;
const KEYWORD_BYTES: usize = 8;
const PRIMARY_KEYWORD_FIELD: &[u8; KEYWORD_BYTES] = b"SIMPLE  ";
const EXTENSION_KEYWORD: &str = "XTENSION";
const END_KEYWORD: &str = "END";
const IMAGE_XTENSION: &str = "IMAGE";
const SCI_EXTNAME: &str = "SCI";
const SCIENCE_GROUP_EXTNAMES: [&str; 8] = ["SCI", "ERR", "DQ", "SAMP", "TIME", "WHT", "CTX", "CON"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeekTarget {
    AllSci,
    FirstImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeekBudget {
    pub first_range: u64,
    pub next_range: u64,
    pub max_ranges: usize,
    pub max_bytes: u64,
}

const UNBOUNDED: PeekBudget = PeekBudget {
    first_range: u64::MAX,
    next_range: u64::MAX,
    max_ranges: usize::MAX,
    max_bytes: u64::MAX,
};

#[derive(Debug, Clone)]
pub struct PeekedHdu {
    pub index: usize,
    pub extname: Option<String>,
    pub extver: Option<i64>,
    pub bitpix: i64,
    pub dims: Vec<u64>,
    pub header: HduHeader,
    pub header_start: u64,
    pub data_start: u64,
    pub wcs_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Footprint {
    pub hdu_index: usize,
    pub corners: [[f64; 2]; 4],
    pub center: [f64; 2],
    pub fov_arcmin: [f64; 2],
}

#[derive(Debug, Clone)]
pub struct PeekedHeaders {
    pub primary: Option<HduHeader>,
    pub hdus: Vec<PeekedHdu>,
    pub footprints: Vec<Footprint>,
    pub complete: bool,
    pub next_needed: Option<u64>,
    pub walk_error: Option<String>,
    pub bytes_fetched: u64,
    pub ranges: usize,
}

pub struct HeaderWalker {
    want: PeekTarget,
    max_hdus: usize,
    budget: PeekBudget,
    file_len: Option<u64>,
    cursor: u64,
    index: usize,
    pending: Vec<u8>,
    primary: Option<HduHeader>,
    hdus: Vec<PeekedHdu>,
    footprints: Vec<Footprint>,
    complete: bool,
    walk_error: Option<String>,
    bytes_fetched: u64,
    ranges: usize,
}

impl HeaderWalker {
    pub fn new(want: PeekTarget, max_hdus: usize, budget: PeekBudget) -> Self {
        Self {
            want,
            max_hdus,
            budget,
            file_len: None,
            cursor: 0,
            index: 0,
            pending: Vec::new(),
            primary: None,
            hdus: Vec::new(),
            footprints: Vec::new(),
            complete: false,
            walk_error: None,
            bytes_fetched: 0,
            ranges: 0,
        }
    }

    pub fn set_file_len(&mut self, len: u64) {
        self.file_len = Some(len);
        self.check_end();
    }

    pub fn feed(&mut self, bytes: &[u8], offset: u64) -> Result<()> {
        self.ranges += 1;
        self.bytes_fetched += bytes.len() as u64;
        if self.complete {
            return Ok(());
        }
        let wanted = self.cursor + self.pending.len() as u64;
        if offset > wanted {
            bail!("peek bytes start at offset {offset} but the header walk needs offset {wanted} first");
        }
        let skip = usize::try_from(wanted - offset).unwrap_or(usize::MAX);
        let Some(fresh) = bytes.get(skip..).filter(|fresh| !fresh.is_empty()) else {
            return Ok(());
        };
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(fresh);
        self.walk(&buf)
    }

    pub fn next_needed(&self) -> Option<u64> {
        (!self.complete).then(|| self.cursor + self.pending.len() as u64)
    }

    pub fn next_range(&self) -> Option<(u64, u64)> {
        let offset = self.next_needed()?;
        if self.ranges >= self.budget.max_ranges {
            return None;
        }
        let left = self.budget.max_bytes.saturating_sub(self.bytes_fetched);
        let wanted = if self.ranges == 0 {
            self.budget.first_range
        } else {
            self.budget.next_range
        };
        let in_file = self
            .file_len
            .map_or(u64::MAX, |len| len.saturating_sub(offset));
        let len = wanted.min(left).min(in_file);
        (len > 0).then_some((offset, len))
    }

    pub fn finish(self) -> PeekedHeaders {
        PeekedHeaders {
            next_needed: self.next_needed(),
            walk_error: self.walk_error,
            primary: self.primary,
            hdus: self.hdus,
            footprints: self.footprints,
            complete: self.complete,
            bytes_fetched: self.bytes_fetched,
            ranges: self.ranges,
        }
    }

    fn walk(&mut self, buf: &[u8]) -> Result<()> {
        let start = self.cursor;
        while !self.complete {
            let Some(rel) = usize::try_from(self.cursor - start)
                .ok()
                .filter(|rel| *rel < buf.len())
            else {
                return Ok(());
            };
            let rest = &buf[rel..];
            if !self.starts_an_hdu(rest)? {
                return Ok(());
            }
            if !contains_end_card(rest) {
                self.pending = rest.to_vec();
                return Ok(());
            }
            let header_start = start + rel as u64;
            let (parsed, next) = match self.parse_hdu(buf, rel, start) {
                Ok(parsed) => parsed,
                Err(e) if !self.hdus.is_empty() => {
                    self.walk_error = Some(format!("{e:#}"));
                    self.complete = true;
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            self.record(
                parsed.header,
                header_start,
                start + parsed.data_start as u64,
            );
            self.cursor = next;
            self.index += 1;
            self.check_end();
        }
        Ok(())
    }

    fn parse_hdu(&self, buf: &[u8], rel: usize, start: u64) -> Result<(ParsedHdu, u64)> {
        let parsed = parse_header_at(buf, rel).with_context(|| {
            format!(
                "HDU {} header at file offset {}",
                self.index,
                start + rel as u64
            )
        })?;
        let next = start
            .checked_add(parsed.next_hdu_offset as u64)
            .with_context(|| format!("HDU {} data unit end overflows", self.index))?;
        Ok((parsed, next))
    }

    fn starts_an_hdu(&mut self, rest: &[u8]) -> Result<bool> {
        if self.index == 0 {
            let seen = rest.len().min(KEYWORD_BYTES);
            if rest[..seen] != PRIMARY_KEYWORD_FIELD[..seen] {
                bail!(
                    "the bytes at offset 0 are not a FITS primary header (first keyword '{}')",
                    card_keyword(rest)
                );
            }
            return Ok(true);
        }
        if rest.len() < CARD_BYTES || card_keyword(rest) == EXTENSION_KEYWORD {
            return Ok(true);
        }
        self.complete = true;
        Ok(false)
    }

    fn record(&mut self, header: HduHeader, header_start: u64, data_start: u64) {
        let index = self.index;
        let (bitpix, dims) = image_shape(&header);
        let is_image = is_image_hdu(&header);
        let extname = header.get("EXTNAME").map(|name| name.trim().to_string());
        let is_sci = extname
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(SCI_EXTNAME));
        let plane = plane_dims(&dims);
        let wanted = is_image && plane.is_some() && (self.want == PeekTarget::FirstImage || is_sci);
        if index == 0 {
            self.primary = Some(header.clone());
        }
        let in_science_group = is_image
            && extname.as_deref().is_some_and(|name| {
                SCIENCE_GROUP_EXTNAMES
                    .iter()
                    .any(|group| name.eq_ignore_ascii_case(group))
            });
        let Some((cols, rows)) = plane.filter(|_| wanted) else {
            if self.want == PeekTarget::AllSci && !in_science_group && !self.hdus.is_empty() {
                self.complete = true;
            }
            return;
        };
        let wcs_header = match self.primary.as_ref().filter(|_| index > 0) {
            Some(primary) => primary.merge_with(&header),
            None => header.clone(),
        };
        let wcs_error = match footprint(index, &wcs_header, cols, rows) {
            Ok(footprint) => {
                self.footprints.push(footprint);
                None
            }
            Err(e) => Some(format!("{e:#}")),
        };
        self.hdus.push(PeekedHdu {
            index,
            extname,
            extver: header.get_i64("EXTVER"),
            bitpix,
            dims,
            header,
            header_start,
            data_start,
            wcs_error,
        });
        if self.want == PeekTarget::FirstImage {
            self.complete = true;
        }
    }

    fn check_end(&mut self) {
        let past_end = self.file_len.is_some_and(|len| {
            let left = len.saturating_sub(self.cursor);
            left == 0 || (self.index > 0 && left < BLOCK_SIZE as u64)
        });
        if self.index >= self.max_hdus || past_end {
            self.complete = true;
        }
    }
}

fn card_keyword(card: &[u8]) -> String {
    String::from_utf8_lossy(&card[..card.len().min(KEYWORD_BYTES)])
        .trim()
        .to_string()
}

fn contains_end_card(bytes: &[u8]) -> bool {
    bytes
        .chunks_exact(BLOCK_SIZE)
        .flat_map(|block| block.chunks_exact(CARD_BYTES))
        .any(|card| card_keyword(card) == END_KEYWORD)
}

fn is_image_hdu(header: &HduHeader) -> bool {
    let image_extension = header
        .get("XTENSION")
        .is_none_or(|xtension| xtension.trim().eq_ignore_ascii_case(IMAGE_XTENSION));
    image_extension || is_compressed_image_hdu(header)
}

fn image_shape(header: &HduHeader) -> (i64, Vec<u64>) {
    let prefix = if is_compressed_image_hdu(header) {
        "Z"
    } else {
        ""
    };
    let naxis = header
        .get_i64(&format!("{prefix}NAXIS"))
        .unwrap_or(0)
        .clamp(0, MAX_FITS_AXES);
    let dims = (1..=naxis)
        .map(|axis| {
            header
                .get_i64(&format!("{prefix}NAXIS{axis}"))
                .unwrap_or(0)
                .max(0) as u64
        })
        .collect();
    (
        header.get_i64(&format!("{prefix}BITPIX")).unwrap_or(0),
        dims,
    )
}

fn plane_dims(dims: &[u64]) -> Option<(usize, usize)> {
    let cols = usize::try_from(*dims.first()?).ok().filter(|n| *n > 0)?;
    let rows = usize::try_from(*dims.get(1)?).ok().filter(|n| *n > 0)?;
    Some((cols, rows))
}

fn footprint(index: usize, header: &HduHeader, cols: usize, rows: usize) -> Result<Footprint> {
    let wcs = header_wcs(header)?;
    let sky = |(x, y): (f64, f64)| -> Result<[f64; 2]> {
        let c = wcs.pixel_to_world(x, y);
        if !c.ra.is_finite() || !c.dec.is_finite() {
            bail!("pixel ({x}, {y}) of HDU {index} does not project to a valid sky position");
        }
        Ok([c.ra, c.dec])
    };
    let [a, b, c, d] = pixel_edge_corners(cols, rows);
    let corners = [sky(a)?, sky(b)?, sky(c)?, sky(d)?];
    let center = sky(pixel_center(cols, rows))?;
    let (width, height) = wcs.field_of_view(cols, rows);
    Ok(Footprint {
        hdu_index: index,
        corners,
        center,
        fov_arcmin: [width, height],
    })
}

pub fn parse_peek(bytes: &[u8], want: PeekTarget, max_hdus: usize) -> Result<PeekedHeaders> {
    let mut walker = HeaderWalker::new(want, max_hdus, UNBOUNDED);
    walker.feed(bytes, 0)?;
    Ok(walker.finish())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::PeekBudget;
    use crate::types::constants::BLOCK_SIZE;

    const CARD_BYTES: usize = 80;

    pub(crate) const MAST_BUDGET: PeekBudget = PeekBudget {
        first_range: 131_072,
        next_range: 65_536,
        max_ranges: 8,
        max_bytes: 524_288,
    };

    pub(crate) type Cards = Vec<(&'static str, String)>;

    fn card(key: &str, value: &str) -> Vec<u8> {
        let mut bytes = if crate::types::header::is_commentary_key(key) {
            format!("{key:<8}{value}").into_bytes()
        } else {
            format!("{key:<8}= {value}").into_bytes()
        };
        bytes.resize(CARD_BYTES, b' ');
        bytes
    }

    pub(crate) fn header_bytes(cards: &[(&str, String)]) -> Vec<u8> {
        let mut out: Vec<u8> = cards.iter().flat_map(|(k, v)| card(k, v)).collect();
        let mut end = b"END".to_vec();
        end.resize(CARD_BYTES, b' ');
        out.extend_from_slice(&end);
        out.resize(out.len().div_ceil(BLOCK_SIZE) * BLOCK_SIZE, b' ');
        out
    }

    pub(crate) fn padded(bytes: u64) -> u64 {
        bytes.div_ceil(BLOCK_SIZE as u64) * BLOCK_SIZE as u64
    }

    #[derive(Default)]
    pub(crate) struct SparseFits {
        segments: Vec<(u64, Vec<u8>)>,
        len: u64,
    }

    impl SparseFits {
        pub(crate) fn push_hdu(&mut self, cards: &[(&str, String)], data_bytes: u64) -> u64 {
            let start = self.len;
            let header = header_bytes(cards);
            self.len += header.len() as u64 + padded(data_bytes);
            self.segments.push((start, header));
            start
        }

        pub(crate) fn push_tail(&mut self, bytes: &[u8]) {
            self.segments.push((self.len, bytes.to_vec()));
            self.len += bytes.len() as u64;
        }

        pub(crate) fn len(&self) -> u64 {
            self.len
        }

        pub(crate) fn read(&self, offset: u64, len: u64) -> Vec<u8> {
            let end = (offset + len).min(self.len);
            let mut out = vec![0u8; end.saturating_sub(offset) as usize];
            for (start, bytes) in &self.segments {
                let seg_end = start + bytes.len() as u64;
                let lo = (*start).max(offset);
                let hi = seg_end.min(end);
                if lo < hi {
                    out[(lo - offset) as usize..(hi - offset) as usize]
                        .copy_from_slice(&bytes[(lo - start) as usize..(hi - start) as usize]);
                }
            }
            out
        }

        pub(crate) fn bytes(&self) -> Vec<u8> {
            self.read(0, self.len)
        }
    }

    pub(crate) fn primary_cards(extra: &[(&'static str, String)]) -> Cards {
        let mut cards: Cards = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "0".into()),
            ("EXTEND", "T".into()),
        ];
        cards.extend(extra.iter().cloned());
        cards
    }

    pub(crate) fn image_cards(
        extname: &str,
        extver: i64,
        bitpix: i64,
        cols: u64,
        rows: u64,
        extra: &[(&'static str, String)],
    ) -> Cards {
        let mut cards: Cards = vec![
            ("XTENSION", "'IMAGE   '".into()),
            ("BITPIX", bitpix.to_string()),
            ("NAXIS", "2".into()),
            ("NAXIS1", cols.to_string()),
            ("NAXIS2", rows.to_string()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("EXTNAME", format!("'{extname:<8}'")),
            ("EXTVER", extver.to_string()),
        ];
        cards.extend(extra.iter().cloned());
        cards
    }

    pub(crate) fn tan_wcs_cards(
        crval: (f64, f64),
        crpix: (f64, f64),
        scale_deg: f64,
        rot_deg: f64,
    ) -> Cards {
        let (s, c) = rot_deg.to_radians().sin_cos();
        vec![
            ("CTYPE1", "'RA---TAN'".into()),
            ("CTYPE2", "'DEC--TAN'".into()),
            ("CRPIX1", format!("{:.10E}", crpix.0)),
            ("CRPIX2", format!("{:.10E}", crpix.1)),
            ("CRVAL1", format!("{:.15E}", crval.0)),
            ("CRVAL2", format!("{:.15E}", crval.1)),
            ("CD1_1", format!("{:.15E}", -scale_deg * c)),
            ("CD1_2", format!("{:.15E}", scale_deg * s)),
            ("CD2_1", format!("{:.15E}", scale_deg * s)),
            ("CD2_2", format!("{:.15E}", scale_deg * c)),
        ]
    }

    pub(crate) fn two_chip_mef() -> SparseFits {
        let mut file = SparseFits::default();
        file.push_hdu(
            &primary_cards(&[("INSTRUME", "'ACS     '".into()), ("NEXTEND", "4".into())]),
            0,
        );
        let chip_bytes = 4096 * 2048 * 4;
        let dq_bytes = 4096 * 2048 * 2;
        let chip1 = tan_wcs_cards((150.1, 2.2), (2048.5, 1024.5), 1.4e-5, 30.0);
        let chip2 = tan_wcs_cards((150.1, 2.25), (2048.5, 1024.5), 1.4e-5, 30.0);
        file.push_hdu(&image_cards("SCI", 1, -32, 4096, 2048, &chip1), chip_bytes);
        file.push_hdu(&image_cards("ERR", 1, -32, 4096, 2048, &[]), chip_bytes);
        file.push_hdu(&image_cards("DQ", 1, 16, 4096, 2048, &[]), dq_bytes);
        file.push_hdu(&image_cards("SCI", 2, -32, 4096, 2048, &chip2), chip_bytes);
        file
    }

    pub(crate) fn acs_wfc_flc() -> SparseFits {
        let mut primary = primary_cards(&[
            ("TELESCOP", "'HST     '".into()),
            ("INSTRUME", "'ACS     '".into()),
            ("DETECTOR", "'WFC     '".into()),
            ("NEXTEND", "15".into()),
        ]);
        primary.extend((0..300).map(|i| ("HISTORY", format!("primary keyword {i}"))));
        let mut file = SparseFits::default();
        file.push_hdu(&primary, 0);
        let chip_bytes = 4096 * 2048 * 4;
        let dq_bytes = 4096 * 2048 * 2;
        for (chip, dec) in [(1, 2.2), (2, 2.25)] {
            let wcs = tan_wcs_cards((150.1, dec), (2048.5, 1024.5), 1.4e-5, 30.0);
            let mut sci = image_cards("SCI", chip, -32, 4096, 2048, &wcs);
            sci.extend((0..450).map(|i| ("HISTORY", format!("science keyword {i}"))));
            file.push_hdu(&sci, chip_bytes);
            file.push_hdu(&image_cards("ERR", chip, -32, 4096, 2048, &[]), chip_bytes);
            file.push_hdu(&image_cards("DQ", chip, 16, 4096, 2048, &[]), dq_bytes);
        }
        for name in ["D2IMARR", "WCSDVARR"] {
            for extver in 1..=4 {
                file.push_hdu(&image_cards(name, extver, -32, 64, 32, &[]), 64 * 32 * 4);
            }
        }
        file.push_hdu(
            &[
                ("XTENSION", "'HDRLET  '".into()),
                ("BITPIX", "8".into()),
                ("NAXIS", "1".into()),
                ("NAXIS1", "40320".into()),
                ("PCOUNT", "0".into()),
                ("GCOUNT", "1".into()),
                ("EXTNAME", "'HDRLET  '".into()),
                ("EXTVER", "1".into()),
            ],
            40_320,
        );
        file.push_hdu(
            &[
                ("XTENSION", "'BINTABLE'".into()),
                ("BITPIX", "8".into()),
                ("NAXIS", "2".into()),
                ("NAXIS1", "512".into()),
                ("NAXIS2", "14".into()),
                ("PCOUNT", "0".into()),
                ("GCOUNT", "1".into()),
                ("TFIELDS", "1".into()),
                ("EXTNAME", "'WCSCORR '".into()),
            ],
            512 * 14,
        );
        file
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::core::astrometry::wcs::{
        angular_separation, pixel_center, pixel_edge_corners, WcsTransform,
    };
    use crate::infra::fits::reader::parse_header_at;

    fn header_of(cards: &[(&str, String)]) -> HduHeader {
        parse_header_at(&header_bytes(cards), 0).unwrap().header
    }

    fn expected_footprint(
        primary: &[(&str, String)],
        sci: &[(&str, String)],
        cols: usize,
        rows: usize,
    ) -> ([[f64; 2]; 4], [f64; 2], [f64; 2]) {
        let merged = header_of(primary).merge_with(&header_of(sci));
        let wcs = WcsTransform::from_header(&merged).unwrap();
        let sky = |(x, y): (f64, f64)| {
            let c = wcs.pixel_to_world(x, y);
            [c.ra, c.dec]
        };
        let e = pixel_edge_corners(cols, rows);
        let (w, h) = wcs.field_of_view(cols, rows);
        (
            [sky(e[0]), sky(e[1]), sky(e[2]), sky(e[3])],
            sky(pixel_center(cols, rows)),
            [w, h],
        )
    }

    fn assert_sky_close(got: [f64; 2], want: [f64; 2], tol_deg: f64, what: &str) {
        let d = angular_separation(got[0], got[1], want[0], want[1]);
        assert!(
            d <= tol_deg,
            "{what}: {got:?} vs {want:?} differ by {d} deg (tolerance {tol_deg})"
        );
    }

    fn walk_sparse(file: &SparseFits, want: PeekTarget, max_hdus: usize) -> PeekedHeaders {
        let mut walker = HeaderWalker::new(want, max_hdus, MAST_BUDGET);
        let mut first = true;
        while let Some((offset, len)) = walker.next_range() {
            walker.feed(&file.read(offset, len), offset).unwrap();
            if first {
                walker.set_file_len(file.len());
                first = false;
            }
        }
        walker.finish()
    }

    fn jwst_like() -> (SparseFits, Cards, Cards) {
        let primary = primary_cards(&[
            ("TELESCOP", "'JWST    '".into()),
            ("INSTRUME", "'NIRCAM  '".into()),
            ("FILTER", "'F200W   '".into()),
            ("DATE-OBS", "'2022-04-01'".into()),
        ]);
        let mut sci = image_cards(
            "SCI",
            1,
            -32,
            2048,
            2048,
            &tan_wcs_cards((274.73, -13.87), (1024.5, 1024.5), 8.6e-6, 12.0),
        );
        sci.push(("BUNIT", "'MJy/sr  '".into()));
        let mut file = SparseFits::default();
        file.push_hdu(&primary, 0);
        file.push_hdu(&sci, 2048 * 2048 * 4);
        file.push_hdu(
            &image_cards("ERR", 1, -32, 2048, 2048, &[]),
            2048 * 2048 * 4,
        );
        (file, primary, sci)
    }

    #[test]
    fn jwst_style_first_image_yields_one_footprint_equal_to_the_merged_header_wcs() {
        let (file, primary, sci) = jwst_like();
        let peek = parse_peek(
            &file.read(0, MAST_BUDGET.first_range),
            PeekTarget::FirstImage,
            16,
        )
        .unwrap();
        assert!(peek.complete);
        assert_eq!(peek.next_needed, None);
        assert_eq!(peek.hdus.len(), 1);
        let hdu = &peek.hdus[0];
        assert_eq!(
            (hdu.index, hdu.extname.as_deref(), hdu.extver),
            (1, Some("SCI"), Some(1))
        );
        assert_eq!((hdu.bitpix, hdu.dims.clone()), (-32, vec![2048, 2048]));
        let primary_len = header_bytes(&primary).len() as u64;
        let sci_len = header_bytes(&sci).len() as u64;
        assert_eq!(
            (hdu.header_start, hdu.data_start),
            (primary_len, primary_len + sci_len)
        );
        assert_eq!(hdu.header.get("BUNIT"), Some("MJy/sr"));
        assert_eq!(hdu.wcs_error, None);
        assert_eq!(
            peek.primary.as_ref().and_then(|p| p.get("INSTRUME")),
            Some("NIRCAM")
        );
        assert_eq!(peek.footprints.len(), 1);
        let (corners, center, fov) = expected_footprint(&primary, &sci, 2048, 2048);
        let fp = peek.footprints[0];
        assert_eq!(fp.hdu_index, 1);
        for (got, want) in fp.corners.iter().zip(corners) {
            assert_sky_close(*got, want, 1e-12, "corner");
        }
        assert_sky_close(fp.center, center, 1e-12, "centre");
        assert_sky_close(fp.center, [274.73, -13.87], 1e-9, "centre at CRVAL");
        assert_eq!(fp.fov_arcmin, fov);
        assert_eq!(
            (peek.ranges, peek.bytes_fetched),
            (1, MAST_BUDGET.first_range)
        );
    }

    #[test]
    fn two_chip_mef_yields_two_footprints_within_five_ranges_and_the_mast_byte_budget() {
        let file = two_chip_mef();
        assert!(file.len() > 100_000_000);
        let peek = walk_sparse(&file, PeekTarget::AllSci, 16);
        assert!(peek.complete, "{:?}", peek.next_needed);
        assert_eq!(peek.footprints.len(), 2);
        let sci: Vec<_> = peek
            .hdus
            .iter()
            .map(|h| (h.index, h.extname.as_deref(), h.extver))
            .collect();
        assert_eq!(
            sci,
            vec![(1, Some("SCI"), Some(1)), (4, Some("SCI"), Some(2))]
        );
        assert!(peek.ranges <= 5, "{} ranges", peek.ranges);
        assert!(
            peek.bytes_fetched <= 524_288,
            "{} bytes",
            peek.bytes_fetched
        );
        assert_sky_close(
            peek.footprints[0].center,
            [150.1, 2.2],
            1e-9,
            "chip 1 centre",
        );
        assert_sky_close(
            peek.footprints[1].center,
            [150.1, 2.25],
            1e-9,
            "chip 2 centre",
        );
        assert_eq!(peek.footprints[1].hdu_index, 4);
        let w = peek.footprints[0].fov_arcmin[0];
        assert!((w - 4096.0 * 1.4e-5 * 60.0).abs() < 1e-6, "fov width {w}");
    }

    #[test]
    fn a_primary_header_longer_than_the_first_range_forces_a_second_range() {
        let mut primary = primary_cards(&[]);
        primary.extend((0..2000).map(|i| ("HISTORY", format!("padding card {i}"))));
        let primary_len = header_bytes(&primary).len() as u64;
        assert!(primary_len > MAST_BUDGET.first_range);
        let mut file = SparseFits::default();
        file.push_hdu(&primary, 0);
        file.push_hdu(
            &image_cards(
                "SCI",
                1,
                -32,
                1000,
                800,
                &tan_wcs_cards((10.0, 20.0), (500.5, 400.5), 1e-5, 0.0),
            ),
            1000 * 800 * 4,
        );
        let peek = walk_sparse(&file, PeekTarget::FirstImage, 16);
        assert!(peek.complete);
        assert_eq!(peek.ranges, 2);
        assert_eq!(peek.hdus[0].header_start, primary_len);
        assert_eq!(peek.footprints.len(), 1);
        assert_eq!(
            peek.bytes_fetched,
            MAST_BUDGET.first_range + MAST_BUDGET.next_range
        );
    }

    #[test]
    fn a_header_beyond_the_buffer_returns_next_needed_at_its_offset() {
        let file = two_chip_mef();
        let peek = parse_peek(
            &file.read(0, MAST_BUDGET.first_range),
            PeekTarget::AllSci,
            16,
        )
        .unwrap();
        assert!(!peek.complete);
        assert_eq!(peek.hdus.len(), 1);
        let sci1 = &peek.hdus[0];
        let err1 = sci1.data_start + padded(4096 * 2048 * 4);
        assert_eq!(peek.next_needed, Some(err1));
    }

    #[test]
    fn a_header_cut_by_the_buffer_end_resumes_from_the_first_missing_byte() {
        let (file, primary, _) = jwst_like();
        let cut = header_bytes(&primary).len() as u64 + 1000;
        let mut walker = HeaderWalker::new(PeekTarget::FirstImage, 16, MAST_BUDGET);
        walker.feed(&file.read(0, cut), 0).unwrap();
        assert_eq!(walker.next_needed(), Some(cut));
        assert_eq!(walker.next_range(), Some((cut, MAST_BUDGET.next_range)));
        walker.feed(&file.read(cut, 10_000), cut).unwrap();
        let peek = walker.finish();
        assert!(peek.complete);
        assert_eq!(peek.footprints.len(), 1);
        assert_eq!(peek.ranges, 2);
    }

    #[test]
    fn overlapping_ranges_reuse_only_the_missing_bytes() {
        let (file, primary, _) = jwst_like();
        let cut = header_bytes(&primary).len() as u64 + 1000;
        let mut walker = HeaderWalker::new(PeekTarget::FirstImage, 16, MAST_BUDGET);
        walker.feed(&file.read(0, cut), 0).unwrap();
        walker.feed(&file.read(0, 2 * cut), 0).unwrap();
        let peek = walker.finish();
        assert!(peek.complete);
        assert_eq!(peek.footprints.len(), 1);
    }

    #[test]
    fn bytes_that_are_not_a_fits_primary_header_are_refused_naming_the_first_keyword() {
        let mut html = b"<!DOCTYPE html><html><body>Service unavailable</body></html>".to_vec();
        html.resize(4000, b' ');
        let err = parse_peek(&html, PeekTarget::FirstImage, 16)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not a FITS primary header") && err.contains("<!DOCTYP"),
            "{err}"
        );
        let mut gzip = vec![0x1f, 0x8b, 0x08, 0x00];
        gzip.resize(4000, 0);
        let err = parse_peek(&gzip, PeekTarget::FirstImage, 16)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a FITS primary header"), "{err}");
    }

    #[test]
    fn a_malformed_extension_header_is_refused_naming_its_file_offset() {
        let mut file = SparseFits::default();
        let primary_len = file.push_hdu(&primary_cards(&[]), 0) + 2880;
        file.push_hdu(
            &[
                ("XTENSION", "'IMAGE   '".into()),
                ("BITPIX", "-32".into()),
                ("NAXIS", "2".into()),
                ("NAXIS1", "-5".into()),
                ("NAXIS2", "10".into()),
                ("EXTNAME", "'SCI     '".into()),
            ],
            0,
        );
        let err = parse_peek(&file.bytes(), PeekTarget::AllSci, 16).unwrap_err();
        let text = format!("{err:#}");
        assert!(
            text.contains(&format!("HDU 1 header at file offset {primary_len}")),
            "{text}"
        );
    }

    #[test]
    fn a_range_that_leaves_a_gap_is_refused() {
        let file = two_chip_mef();
        let mut walker = HeaderWalker::new(PeekTarget::AllSci, 16, MAST_BUDGET);
        walker.feed(&file.read(0, 131_072), 0).unwrap();
        let needed = walker.next_needed().unwrap();
        let err = walker
            .feed(&file.read(needed + 2880, 65_536), needed + 2880)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&needed.to_string()) && err.contains(&(needed + 2880).to_string()),
            "{err}"
        );
    }

    #[test]
    fn first_image_skips_a_binary_table_with_two_axes() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        file.push_hdu(
            &[
                ("XTENSION", "'BINTABLE'".into()),
                ("BITPIX", "8".into()),
                ("NAXIS", "2".into()),
                ("NAXIS1", "16".into()),
                ("NAXIS2", "100".into()),
                ("PCOUNT", "0".into()),
                ("GCOUNT", "1".into()),
                ("TFIELDS", "2".into()),
                ("EXTNAME", "'EXTRACT1D'".into()),
            ],
            1600,
        );
        file.push_hdu(
            &image_cards(
                "IMG",
                1,
                16,
                64,
                32,
                &tan_wcs_cards((1.0, 2.0), (32.5, 16.5), 1e-4, 0.0),
            ),
            64 * 32 * 2,
        );
        let peek = parse_peek(&file.bytes(), PeekTarget::FirstImage, 16).unwrap();
        assert!(peek.complete);
        assert_eq!(peek.hdus.len(), 1);
        assert_eq!(
            (peek.hdus[0].index, peek.hdus[0].extname.as_deref()),
            (2, Some("IMG"))
        );
        assert_eq!(peek.footprints.len(), 1);
    }

    #[test]
    fn first_image_accepts_a_primary_image() {
        let mut file = SparseFits::default();
        let mut cards = vec![
            ("SIMPLE", "T".to_string()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "300".into()),
            ("NAXIS2", "200".into()),
        ];
        cards.extend(tan_wcs_cards((83.8, -5.4), (150.5, 100.5), 2e-4, 0.0));
        file.push_hdu(&cards, 300 * 200 * 4);
        let peek = parse_peek(&file.read(0, 2880), PeekTarget::FirstImage, 16).unwrap();
        assert!(peek.complete);
        assert_eq!(peek.hdus[0].index, 0);
        assert_eq!(peek.hdus[0].data_start, 2880);
        assert_sky_close(
            peek.footprints[0].center,
            [83.8, -5.4],
            1e-9,
            "primary image centre",
        );
    }

    #[test]
    fn all_sci_stops_at_the_first_table_after_a_science_extension() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        let wcs = tan_wcs_cards((40.0, -1.0), (50.5, 50.5), 1e-4, 0.0);
        file.push_hdu(&image_cards("SCI", 1, -32, 100, 100, &wcs), 100 * 100 * 4);
        file.push_hdu(
            &[
                ("XTENSION", "'BINTABLE'".into()),
                ("BITPIX", "8".into()),
                ("NAXIS", "2".into()),
                ("NAXIS1", "8".into()),
                ("NAXIS2", "1".into()),
                ("PCOUNT", "0".into()),
                ("GCOUNT", "1".into()),
                ("TFIELDS", "1".into()),
                ("EXTNAME", "'WCSCORR '".into()),
            ],
            8,
        );
        file.push_hdu(&image_cards("SCI", 2, -32, 100, 100, &wcs), 100 * 100 * 4);
        let peek = parse_peek(&file.bytes(), PeekTarget::AllSci, 16).unwrap();
        assert!(peek.complete);
        assert_eq!(peek.hdus.len(), 1);
        assert_eq!(peek.next_needed, None);
    }

    #[test]
    fn max_hdus_bounds_the_walk() {
        let file = two_chip_mef();
        let peek = walk_sparse(&file, PeekTarget::AllSci, 2);
        assert!(peek.complete);
        assert_eq!(peek.hdus.len(), 1);
        assert_eq!(peek.ranges, 1);
    }

    #[test]
    fn a_science_header_without_wcs_is_kept_with_the_reason_and_no_footprint() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        file.push_hdu(&image_cards("SCI", 1, -32, 100, 100, &[]), 100 * 100 * 4);
        let peek = parse_peek(&file.read(0, 8 * 2880), PeekTarget::FirstImage, 16).unwrap();
        assert!(peek.complete);
        assert_eq!(peek.hdus.len(), 1);
        assert!(peek.footprints.is_empty());
        let reason = peek.hdus[0].wcs_error.clone().unwrap();
        assert!(reason.contains("CRPIX1"), "{reason}");
    }

    #[test]
    fn a_tan_header_with_pv_cards_gets_the_footprint_of_its_linear_tan() {
        use crate::infra::wcs_source::test_fixtures::PV_DISTORTION_CARDS;

        let linear = tan_wcs_cards((180.0, 45.0), (50.5, 50.5), 1e-5, 0.0);
        let mut with_pv = linear.clone();
        with_pv.extend(PV_DISTORTION_CARDS.map(|(key, value)| (key, value.to_string())));
        let peek = |wcs: &Cards| {
            let mut file = SparseFits::default();
            file.push_hdu(&primary_cards(&[]), 0);
            file.push_hdu(&image_cards("SCI", 1, -32, 100, 100, wcs), 100 * 100 * 4);
            parse_peek(&file.read(0, 8 * 2880), PeekTarget::FirstImage, 16).unwrap()
        };
        let (got, want) = (peek(&with_pv), peek(&linear));
        assert_eq!(got.hdus[0].wcs_error, None);
        assert_eq!((got.footprints.len(), want.footprints.len()), (1, 1));
        let (fp, reference) = (got.footprints[0], want.footprints[0]);
        for (got, want) in fp.corners.iter().zip(reference.corners) {
            assert_sky_close(*got, want, 1e-12, "corner");
        }
        assert_sky_close(fp.center, reference.center, 1e-12, "centre");
    }

    #[test]
    fn the_walk_completes_at_the_end_of_the_file() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        file.push_hdu(
            &image_cards(
                "SCI",
                1,
                -32,
                4096,
                4096,
                &tan_wcs_cards((5.0, 5.0), (2048.5, 2048.5), 1e-5, 0.0),
            ),
            4096 * 4096 * 4,
        );
        let peek = walk_sparse(&file, PeekTarget::AllSci, 16);
        assert!(peek.complete);
        assert_eq!(peek.next_needed, None);
        assert_eq!(peek.ranges, 1);
        assert_eq!(peek.footprints.len(), 1);
    }

    #[test]
    fn next_range_respects_the_range_and_byte_budget() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        for v in 1..=20 {
            file.push_hdu(
                &image_cards("ERR", v, -32, 1024, 1024, &[]),
                1024 * 1024 * 4,
            );
        }
        let peek = walk_sparse(&file, PeekTarget::AllSci, 64);
        assert!(!peek.complete);
        assert!(peek.next_needed.is_some());
        assert!(peek.ranges <= MAST_BUDGET.max_ranges);
        assert!(
            peek.bytes_fetched <= MAST_BUDGET.max_bytes,
            "{}",
            peek.bytes_fetched
        );
        let tight = PeekBudget {
            first_range: 2880,
            next_range: 2880,
            max_ranges: 3,
            max_bytes: 1_000_000,
        };
        let mut walker = HeaderWalker::new(PeekTarget::AllSci, 64, tight);
        while let Some((offset, len)) = walker.next_range() {
            assert_eq!(len, 2880);
            walker.feed(&file.read(offset, len), offset).unwrap();
        }
        assert_eq!(walker.finish().ranges, 3);
    }

    #[test]
    fn a_tile_compressed_science_extension_reports_its_image_shape() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        let mut cards: Cards = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "8".into()),
            ("NAXIS2", "50".into()),
            ("PCOUNT", "4000".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("ZIMAGE", "T".into()),
            ("ZBITPIX", "-32".into()),
            ("ZNAXIS", "2".into()),
            ("ZNAXIS1", "120".into()),
            ("ZNAXIS2", "50".into()),
            ("EXTNAME", "'SCI     '".into()),
        ];
        cards.extend(tan_wcs_cards((200.0, 30.0), (60.5, 25.5), 1e-4, 0.0));
        file.push_hdu(&cards, 8 * 50 + 4000);
        let peek = parse_peek(&file.bytes(), PeekTarget::AllSci, 16).unwrap();
        assert_eq!(peek.hdus.len(), 1);
        assert_eq!(
            (peek.hdus[0].bitpix, peek.hdus[0].dims.clone()),
            (-32, vec![120, 50])
        );
        assert!((peek.footprints[0].fov_arcmin[0] - 120.0 * 1e-4 * 60.0).abs() < 1e-9);
    }

    #[test]
    fn an_acs_wfc_flc_stops_at_the_first_extension_outside_the_science_groups_inside_the_mast_budget(
    ) {
        let file = acs_wfc_flc();
        let peek = walk_sparse(&file, PeekTarget::AllSci, 32);
        assert!(
            peek.complete,
            "next needed {:?} after {} ranges and {} bytes",
            peek.next_needed, peek.ranges, peek.bytes_fetched
        );
        assert_eq!(peek.next_needed, None);
        assert_eq!(peek.walk_error, None);
        let sci: Vec<_> = peek
            .hdus
            .iter()
            .map(|h| (h.index, h.extname.as_deref(), h.extver))
            .collect();
        assert_eq!(
            sci,
            vec![(1, Some("SCI"), Some(1)), (4, Some("SCI"), Some(2))]
        );
        assert_eq!(peek.footprints.len(), 2);
        assert_sky_close(
            peek.footprints[0].center,
            [150.1, 2.2],
            1e-9,
            "chip 1 centre",
        );
        assert_sky_close(
            peek.footprints[1].center,
            [150.1, 2.25],
            1e-9,
            "chip 2 centre",
        );
        assert_eq!(
            (peek.ranges, peek.bytes_fetched),
            (7, MAST_BUDGET.max_bytes)
        );
    }

    #[test]
    fn all_sci_walks_through_err_dq_samp_and_time_groups_to_the_next_chip() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        let wcs = tan_wcs_cards((60.0, 10.0), (50.5, 50.5), 1e-4, 0.0);
        for chip in 1..=2 {
            file.push_hdu(
                &image_cards("SCI", chip, -32, 100, 100, &wcs),
                100 * 100 * 4,
            );
            for group in ["ERR", "DQ", "SAMP", "TIME"] {
                file.push_hdu(&image_cards(group, chip, -32, 100, 100, &[]), 100 * 100 * 4);
            }
        }
        file.push_hdu(&image_cards("D2IMARR", 1, -32, 64, 32, &[]), 64 * 32 * 4);
        file.push_hdu(&image_cards("SCI", 3, -32, 100, 100, &wcs), 100 * 100 * 4);
        let peek = parse_peek(&file.bytes(), PeekTarget::AllSci, 32).unwrap();
        assert!(peek.complete);
        let found: Vec<_> = peek.hdus.iter().map(|h| (h.index, h.extver)).collect();
        assert_eq!(found, vec![(1, Some(1)), (6, Some(2))]);
        assert_eq!(peek.footprints.len(), 2);
    }

    #[test]
    fn a_malformed_header_after_a_collected_science_extension_ends_the_walk_with_the_reason() {
        let mut file = SparseFits::default();
        file.push_hdu(&primary_cards(&[]), 0);
        let wcs = tan_wcs_cards((40.0, -1.0), (50.5, 50.5), 1e-4, 0.0);
        file.push_hdu(&image_cards("SCI", 1, -32, 100, 100, &wcs), 100 * 100 * 4);
        let broken_at = file.push_hdu(
            &[
                ("XTENSION", "'IMAGE   '".into()),
                ("BITPIX", "-32".into()),
                ("NAXIS", "2".into()),
                ("NAXIS1", "-5".into()),
                ("NAXIS2", "10".into()),
                ("EXTNAME", "'ERR     '".into()),
            ],
            0,
        );
        let peek = parse_peek(&file.bytes(), PeekTarget::AllSci, 16).unwrap();
        assert!(peek.complete);
        assert_eq!(peek.next_needed, None);
        assert_eq!(peek.hdus.len(), 1);
        assert_eq!(peek.footprints.len(), 1);
        let reason = peek
            .walk_error
            .expect("the parse failure is kept as the reason");
        assert!(
            reason.contains(&format!("HDU 2 header at file offset {broken_at}")),
            "{reason}"
        );
    }

    #[test]
    fn a_partial_block_after_the_last_hdu_ends_the_walk_like_the_full_reader() {
        let mut trailer = b"XTENSION= 'IMAGE   '           / truncated".to_vec();
        trailer.resize(200, b' ');
        for tail in [trailer, vec![0u8; 40]] {
            let mut file = SparseFits::default();
            file.push_hdu(&primary_cards(&[]), 0);
            let wcs = tan_wcs_cards((40.0, -1.0), (50.5, 50.5), 1e-4, 0.0);
            file.push_hdu(&image_cards("SCI", 1, -32, 100, 100, &wcs), 100 * 100 * 4);
            file.push_tail(&tail);
            let peek = walk_sparse(&file, PeekTarget::AllSci, 16);
            assert!(
                peek.complete,
                "{}-byte tail: next needed {:?}",
                tail.len(),
                peek.next_needed
            );
            assert_eq!(peek.next_needed, None);
            assert_eq!(peek.footprints.len(), 1);
        }
    }

    #[test]
    fn a_short_body_that_is_not_fits_is_refused_without_asking_for_more_ranges() {
        let err = parse_peek(br#"{"error":"Not Found"}"#, PeekTarget::FirstImage, 16)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not a FITS primary header") && err.contains(r#"{"error""#),
            "{err}"
        );
        for short in [&b"<!D"[..], &[0x1f, 0x8b]] {
            let err = parse_peek(short, PeekTarget::FirstImage, 16)
                .unwrap_err()
                .to_string();
            assert!(err.contains("not a FITS primary header"), "{err}");
        }
        let mut walker = HeaderWalker::new(PeekTarget::FirstImage, 16, MAST_BUDGET);
        walker.feed(b"SIMPLE", 0).unwrap();
        assert_eq!(walker.next_needed(), Some(6));
    }

    #[test]
    #[ignore]
    fn jwst_nrca1_cal_peek_matches_the_full_file_header_wcs_within_the_sip_bound() {
        let Some(path) = std::env::var_os("ASTROBURST_MAST_PEEK_SAMPLE") else {
            eprintln!(
                "SKIPPED: ASTROBURST_MAST_PEEK_SAMPLE unset, the nrca1 peek oracle did not run"
            );
            return;
        };
        let path = path.to_string_lossy().into_owned();
        let file_len = std::fs::metadata(&path).unwrap().len();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(std::fs::File::open(&path).unwrap(), 262_144),
            &mut bytes,
        )
        .unwrap();
        let peek = parse_peek(&bytes[..131_072], PeekTarget::FirstImage, 16).unwrap();
        assert!(peek.complete);
        assert_eq!(peek.footprints.len(), 1);
        let sci = &peek.hdus[0];
        assert_eq!(
            (sci.index, sci.extname.as_deref(), sci.dims.clone()),
            (1, Some("SCI"), vec![2048, 2048])
        );

        let full = crate::infra::image_source::load_plane_header(
            &crate::types::image_ref::ImageRef::parse(&path),
        )
        .unwrap();
        let header_wcs = WcsTransform::from_header(&full).unwrap();
        let gwcs = crate::infra::wcs_source::load_wcs(&path, &full).unwrap();
        assert!(gwcs.gwcs_info().is_some(), "the sample must carry a gWCS");
        let fp = peek.footprints[0];
        let sip_max_err_px = gwcs
            .sip_fit_residuals()
            .0
            .or(header_wcs.sip_fit_residuals().0)
            .expect("SIPMXERR from the header or the gWCS wcsinfo");
        let sip_bound_deg = sip_max_err_px * header_wcs.pixel_scale_arcsec() / 3600.0 + 1e-9;
        let mut worst_gwcs = 0.0f64;
        for (got, (x, y)) in fp.corners.iter().zip(pixel_edge_corners(2048, 2048)) {
            let h = header_wcs.pixel_to_world(x, y);
            assert_sky_close(
                *got,
                [h.ra, h.dec],
                1e-9,
                "peek corner vs full-file header WCS",
            );
            let g = gwcs.pixel_to_world(x, y);
            let d = angular_separation(got[0], got[1], g.ra, g.dec);
            worst_gwcs = worst_gwcs.max(d);
            assert!(
                d <= sip_bound_deg,
                "corner ({x}, {y}) is {d} deg from the gWCS, bound {sip_bound_deg}"
            );
        }
        let (cx, cy) = pixel_center(2048, 2048);
        let h = header_wcs.pixel_to_world(cx, cy);
        assert_sky_close(
            fp.center,
            [h.ra, h.dec],
            1e-9,
            "peek centre vs full-file header WCS",
        );

        let mut walker = HeaderWalker::new(PeekTarget::FirstImage, 16, MAST_BUDGET);
        walker.set_file_len(file_len);
        while let Some((offset, len)) = walker.next_range() {
            let range = bytes
                .get(offset as usize..(offset + len) as usize)
                .expect("the walk stays inside the first 262,144 bytes");
            walker.feed(range, offset).unwrap();
        }
        let walked = walker.finish();
        assert!(walked.complete);
        assert!(walked.bytes_fetched <= 262_144, "{}", walked.bytes_fetched);
        println!(
            "nrca1 peek: SCI header at {}, data at {}, {} bytes in {} ranges, worst corner vs gWCS {:.3e} deg, SIP bound {:.3e} deg",
            sci.header_start, sci.data_start, walked.bytes_fetched, walked.ranges, worst_gwcs, sip_bound_deg
        );
    }
}
