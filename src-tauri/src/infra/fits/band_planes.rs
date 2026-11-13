use std::collections::{HashSet, VecDeque};
use std::fmt::Display;
use std::fs::File;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};

use crate::infra::asdf::converter::is_asdf_file;
use crate::types::constants::BLOCK_SIZE;
use crate::types::header::is_commentary_key;
use crate::types::image_ref::{source_file, source_stem, BandPlaneKey};
use crate::types::{HduHeader, ImageRef};

use super::compress::is_compressed_image_hdu;
use super::dispatcher::resolve_single_image;
use super::file_bytes::read_file_bytes;
use super::gzip::{sweep_stale_copies_once, SharedTempDir};
use super::reader::{extract_header_by_index, extract_header_by_index_merged, list_extensions, parse_header_at, ParsedHdu};

pub const WARM_PLANE_COPIES: usize = 8;
pub const MAX_LISTED_PLANES: usize = 16;
pub const PLANE_HDU_CARD: &str = "ABPLHDU";
pub const PLANE_INDEX_CARD: &str = "ABPLANE";
pub const PLANE_LABEL_CARD: &str = "ABPLBAND";

const MAX_LABEL_LEN: usize = 16;
const RECORD_LEN: usize = 80;
const IMAGE_BITPIX: [i64; 6] = [8, 16, 32, 64, -32, -64];
const NEEDS_A_CUBE: &str = "a plane key needs an image HDU with exactly three axes";
const DROPPED_KEYWORDS: [&str; 14] = [
    "SIMPLE",
    "XTENSION",
    "BITPIX",
    "NAXIS",
    "EXTEND",
    "PCOUNT",
    "GCOUNT",
    "GROUPS",
    "CHECKSUM",
    "DATASUM",
    "WCSAXES",
    PLANE_HDU_CARD,
    PLANE_INDEX_CARD,
    PLANE_LABEL_CARD,
];
const AXIS_KEYWORD_PREFIXES: [&str; 6] = ["CTYPE", "CRVAL", "CRPIX", "CDELT", "CUNIT", "CROTA"];
const AXIS_MATRIX_PREFIXES: [&str; 2] = ["CD", "PC"];
const PROJECTION_PARAMETER_PREFIXES: [&str; 2] = ["PV", "PS"];
const PIXEL_ENCODING_KEYWORDS: [&str; 5] = ["BSCALE", "BZERO", "BLANK", "BSOFTEN", "BOFFSET"];
const ASINH_KEYWORDS: [&str; 2] = ["BSOFTEN", "BOFFSET"];
const PLANE_COPY_PREFIX: &str = "astroburst-gunzip-plane-";

static PLANE_COPIES: PlaneCopies = PlaneCopies::new();

#[cfg(test)]
thread_local! {
    static COPIES_MADE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BandPlane {
    pub plane: usize,
    pub label: Option<String>,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HduBandPlanes {
    pub hdu: usize,
    pub planes: usize,
    pub bands: Vec<BandPlane>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedBandPlane {
    pub hdu: usize,
    pub plane: usize,
    pub label: Option<String>,
}

fn is_valid_label(label: &str) -> bool {
    (1..=MAX_LABEL_LEN).contains(&label.len()) && label.bytes().all(|b| (0x21..=0x7E).contains(&b) && b != b'\'')
}

fn numbered_labels(header: &HduHeader, planes: usize) -> Option<Vec<String>> {
    if header.get(&format!("BAND{planes}")).is_some() {
        return None;
    }
    (0..planes)
        .map(|n| header.get(&format!("BAND{n}")).map(str::trim).filter(|label| is_valid_label(label)).map(str::to_string))
        .collect()
}

fn bands_card_labels(header: &HduHeader, planes: usize) -> Option<Vec<String>> {
    let value = header.get("BANDS")?.trim();
    let is_separator = |c: char| c == ',' || c.is_whitespace();
    let tokens: Vec<String> = if value.contains(is_separator) {
        value.split(is_separator).filter(|token| !token.is_empty()).map(str::to_string).collect()
    } else {
        value.chars().map(String::from).collect()
    };
    (tokens.len() == planes && tokens.iter().all(|token| is_valid_label(token))).then_some(tokens)
}

pub fn band_labels(header: &HduHeader, planes: usize) -> Vec<String> {
    if planes == 0 {
        return Vec::new();
    }
    numbered_labels(header, planes).or_else(|| bands_card_labels(header, planes)).unwrap_or_default()
}

pub fn list_band_planes(path: &str) -> Result<Vec<HduBandPlanes>> {
    let reference = ImageRef::parse(path);
    let file = source_file(&reference.path);
    let (readable, _decompressed) = resolve_single_image(file)?;
    if is_asdf_file(&readable) {
        return Ok(Vec::new());
    }
    let handle = File::open(&readable).with_context(|| format!("Failed to open {file}"))?;
    let mut listed = Vec::new();
    for info in list_extensions(&handle)? {
        if !info.has_data || info.naxis != 3 || info.naxis3 < 2 {
            continue;
        }
        if is_compressed_image_hdu(&extract_header_by_index(&handle, info.index)?) {
            continue;
        }
        let planes = usize::try_from(info.naxis3)?;
        let bands = if planes <= MAX_LISTED_PLANES {
            let labels = band_labels(&extract_header_by_index_merged(&handle, info.index)?, planes);
            (0..planes)
                .map(|plane| BandPlane {
                    plane,
                    label: labels.get(plane).cloned(),
                    key: BandPlaneKey::new(file, info.index, plane).key(),
                })
                .collect()
        } else {
            Vec::new()
        };
        listed.push(HduBandPlanes { hdu: info.index, planes, bands });
    }
    Ok(listed)
}

fn file_hdus(bytes: &[u8]) -> Result<Vec<ParsedHdu>> {
    let mut hdus = vec![parse_header_at(bytes, 0)?];
    loop {
        let offset = hdus[hdus.len() - 1].next_hdu_offset;
        if offset >= bytes.len() {
            break;
        }
        match parse_header_at(bytes, offset) {
            Ok(parsed) => hdus.push(parsed),
            Err(_) => break,
        }
    }
    Ok(hdus)
}

fn keyword(record: &[u8]) -> String {
    String::from_utf8_lossy(&record[..8]).trim_end_matches(' ').to_string()
}

fn identity(record: &[u8]) -> String {
    let key = keyword(record);
    if key != "HIERARCH" {
        return key;
    }
    let text = String::from_utf8_lossy(&record[..RECORD_LEN]);
    text.split('=').next().unwrap_or_default().trim_end_matches(' ').to_string()
}

fn is_commentary(keyword: &str) -> bool {
    keyword.is_empty() || is_commentary_key(keyword)
}

fn axis_number(text: &str) -> Option<usize> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

fn axis_pair(text: &str) -> Option<(usize, usize)> {
    let (first, second) = text.split_once('_')?;
    Some((axis_number(first)?, axis_number(second)?))
}

fn is_dropped(keyword: &str) -> bool {
    let after = |prefixes: &[&str]| -> Vec<&str> { prefixes.iter().filter_map(|prefix| keyword.strip_prefix(prefix)).collect() };
    DROPPED_KEYWORDS.contains(&keyword)
        || keyword.strip_prefix("NAXIS").and_then(axis_number).is_some()
        || after(&AXIS_KEYWORD_PREFIXES).into_iter().filter_map(axis_number).any(|axis| axis >= 3)
        || after(&AXIS_MATRIX_PREFIXES).into_iter().filter_map(axis_pair).any(|(i, j)| i >= 3 || j >= 3)
        || after(&PROJECTION_PARAMETER_PREFIXES).into_iter().filter_map(axis_pair).any(|(i, _)| i >= 3)
}

fn header_entries<'a>(bytes: &'a [u8], hdu: &ParsedHdu) -> Vec<&'a [u8]> {
    let header = &bytes[hdu.header_start..hdu.data_start];
    let records: Vec<&[u8]> = header.chunks_exact(RECORD_LEN).take_while(|record| keyword(record) != "END").collect();
    let mut starts: Vec<usize> = (0..records.len()).filter(|&i| i == 0 || keyword(records[i]) != "CONTINUE").collect();
    starts.push(records.len());
    starts.windows(2).map(|pair| &header[pair[0] * RECORD_LEN..pair[1] * RECORD_LEN]).collect()
}

fn padded_record(text: String) -> Vec<u8> {
    let mut record = text.into_bytes();
    record.resize(RECORD_LEN, b' ');
    record
}

fn value_record(key: &str, value: impl Display) -> Vec<u8> {
    padded_record(format!("{key:<8}= {value:>20}"))
}

fn pad_to_block(out: &mut Vec<u8>, fill: u8) {
    out.resize(out.len().div_ceil(BLOCK_SIZE) * BLOCK_SIZE, fill);
}

fn has_asinh_card(header: &HduHeader) -> bool {
    ASINH_KEYWORDS.iter().any(|key| header.get(key).is_some())
}

fn plane_byte_range(data_start: usize, naxis1: i64, naxis2: i64, bitpix: i64, plane: usize) -> Option<Range<usize>> {
    let pixels = usize::try_from(naxis1).ok()?.checked_mul(usize::try_from(naxis2).ok()?)?;
    let plane_len = pixels.checked_mul(usize::try_from(bitpix.unsigned_abs() / 8).ok()?)?;
    let start = plane_len.checked_mul(plane)?.checked_add(data_start)?;
    Some(start..start.checked_add(plane_len)?)
}

fn copied_records(bytes: &[u8], hdus: &[ParsedHdu], hdu: usize) -> Vec<u8> {
    let own: Vec<&[u8]> = header_entries(bytes, &hdus[hdu]).into_iter().filter(|entry| !is_dropped(&keyword(entry))).collect();
    let written: HashSet<String> = own.iter().map(|entry| identity(entry)).collect();
    let mut out = own.concat();
    if hdu > 0 {
        for entry in header_entries(bytes, &hdus[0]) {
            let key = keyword(entry);
            if is_dropped(&key) || PIXEL_ENCODING_KEYWORDS.contains(&key.as_str()) {
                continue;
            }
            if is_commentary(&key) || !written.contains(&identity(entry)) {
                out.extend_from_slice(entry);
            }
        }
    }
    out
}

pub fn band_plane_file_bytes(bytes: &[u8], hdu: usize, plane: usize) -> Result<Vec<u8>> {
    let hdus = file_hdus(bytes)?;
    let Some(cube) = hdus.get(hdu) else {
        bail!("HDU index {hdu} out of range (file has {} HDUs)", hdus.len());
    };
    let header = &cube.header;
    if is_compressed_image_hdu(header) {
        bail!("HDU {hdu} is a tile-compressed image; a plane key addresses an uncompressed cube");
    }
    if let Some(xtension) = header.get("XTENSION").map(str::trim).filter(|x| !x.eq_ignore_ascii_case("IMAGE")) {
        bail!("HDU {hdu} is a {xtension} extension; {NEEDS_A_CUBE}");
    }
    let integer = |key: &str| header.get_i64(key).unwrap_or(0);
    let (naxis, naxis1, naxis2, naxis3) = (integer("NAXIS"), integer("NAXIS1"), integer("NAXIS2"), integer("NAXIS3"));
    if naxis != 3 || naxis1 <= 1 || naxis2 <= 1 || naxis3 < 1 {
        bail!("HDU {hdu} is not a cube of images (NAXIS = {naxis}, {naxis1} x {naxis2}); {NEEDS_A_CUBE}");
    }
    let bitpix = integer("BITPIX");
    if !IMAGE_BITPIX.contains(&bitpix) {
        bail!("HDU {hdu} has BITPIX = {bitpix}, which is not an image type");
    }
    if hdu > 0 && !has_asinh_card(header) && has_asinh_card(&hdus[0].header) {
        bail!("HDU {hdu} has no BSOFTEN or BOFFSET card but the primary HDU has: it is not stated whether these planes are asinh-scaled, so a plane key is refused");
    }
    if i64::try_from(plane).map_or(true, |k| k >= naxis3) {
        bail!("plane {plane} is out of range: HDU {hdu} has {naxis3} planes, numbered 0 to {}", naxis3 - 1);
    }
    let Some(range) = plane_byte_range(cube.data_start, naxis1, naxis2, bitpix, plane) else {
        bail!("HDU {hdu}: the size of plane {plane} overflows");
    };
    let Some(pixels) = bytes.get(range) else {
        bail!("HDU {hdu} declares {naxis3} planes of {naxis1} x {naxis2} pixels but the file ends inside plane {plane}");
    };
    let planes = usize::try_from(naxis3)?;
    let labels = if hdu == 0 { band_labels(header, planes) } else { band_labels(&hdus[0].header.merge_with(header), planes) };

    let mut out = value_record("SIMPLE", "T");
    for (key, value) in [("BITPIX", bitpix), ("NAXIS", 2), ("NAXIS1", naxis1), ("NAXIS2", naxis2)] {
        out.extend(value_record(key, value));
    }
    out.extend(copied_records(bytes, &hdus, hdu));
    out.extend(value_record(PLANE_HDU_CARD, hdu));
    out.extend(value_record(PLANE_INDEX_CARD, plane));
    if let Some(label) = labels.get(plane) {
        out.extend(padded_record(format!("{PLANE_LABEL_CARD:<8}= '{label:<8}'")));
    }
    out.extend(padded_record("END".to_string()));
    pad_to_block(&mut out, b' ');
    out.extend_from_slice(pixels);
    pad_to_block(&mut out, 0);
    Ok(out)
}

pub fn loaded_band_plane(header: &HduHeader) -> Option<LoadedBandPlane> {
    let index = |key: &str| header.get(key)?.trim().parse::<usize>().ok();
    Some(LoadedBandPlane {
        hdu: index(PLANE_HDU_CARD)?,
        plane: index(PLANE_INDEX_CARD)?,
        label: header.get(PLANE_LABEL_CARD).map(str::trim).filter(|label| !label.is_empty()).map(str::to_string),
    })
}

pub fn band_plane_of_path(path: &str) -> Option<BandPlaneKey> {
    BandPlaneKey::parse(path).filter(|_| !Path::new(path).is_file())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceStamp {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

impl SourceStamp {
    fn of(path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path).with_context(|| format!("Failed to open {}", path.display()))?;
        Ok(Self {
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

struct PlaneCopy {
    stamp: SourceStamp,
    hdu: usize,
    plane: usize,
    path: PathBuf,
    dir: SharedTempDir,
}

pub(super) struct PlaneCopies {
    copies: Mutex<VecDeque<PlaneCopy>>,
}

impl PlaneCopies {
    pub(super) const fn new() -> Self {
        Self { copies: Mutex::new(VecDeque::new()) }
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<PlaneCopy>> {
        self.copies.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn resolve(&self, band: &BandPlaneKey) -> Result<(PathBuf, SharedTempDir)> {
        self.warm_or_new_copy(band).with_context(|| format!("cannot open the plane {}", band.key()))
    }

    fn warm_or_new_copy(&self, band: &BandPlaneKey) -> Result<(PathBuf, SharedTempDir)> {
        sweep_stale_copies_once();
        let stamp = SourceStamp::of(Path::new(&band.file))?;
        {
            let mut copies = self.lock();
            let warm = copies
                .iter()
                .position(|copy| copy.stamp == stamp && copy.hdu == band.hdu && copy.plane == band.plane && copy.path.is_file());
            if let Some(copy) = warm.and_then(|index| copies.remove(index)) {
                let found = (copy.path.clone(), Arc::clone(&copy.dir));
                copies.push_front(copy);
                return Ok(found);
            }
        }
        let (path, dir) = new_plane_copy(band)?;
        let dropped = {
            let mut copies = self.lock();
            copies.push_front(PlaneCopy { stamp, hdu: band.hdu, plane: band.plane, path: path.clone(), dir: Arc::clone(&dir) });
            let kept = WARM_PLANE_COPIES.min(copies.len());
            copies.split_off(kept)
        };
        drop(dropped);
        Ok((path, dir))
    }
}

fn new_plane_copy(band: &BandPlaneKey) -> Result<(PathBuf, SharedTempDir)> {
    #[cfg(test)]
    COPIES_MADE.with(|count| count.set(count.get() + 1));
    let (readable, _decompressed) = resolve_single_image(&band.file)?;
    if is_asdf_file(&readable) {
        bail!("{} is an ASDF file; a plane key addresses a FITS cube", band.file);
    }
    let bytes = read_file_bytes(&File::open(&readable)?)?;
    let copy = band_plane_file_bytes(&bytes, band.hdu, band.plane)?;
    let root = std::env::temp_dir();
    let dir = tempfile::Builder::new()
        .prefix(PLANE_COPY_PREFIX)
        .tempdir_in(&root)
        .map_err(|error| anyhow!("cannot write the copy of the plane to {}: {error}", root.display()))?;
    let path = dir.path().join(format!("{}.fits", source_stem(&band.key())));
    std::fs::write(&path, copy)
        .map_err(|error| anyhow!("cannot write the copy of the plane to {}: {error}", dir.path().display()))?;
    Ok((path, Arc::new(dir)))
}

pub(super) fn resolve_band_plane(band: &BandPlaneKey) -> Result<(PathBuf, SharedTempDir)> {
    PLANE_COPIES.resolve(band)
}

pub(super) fn resolve_band_plane_fresh(band: &BandPlaneKey) -> Result<(PathBuf, SharedTempDir)> {
    PlaneCopies::new().resolve(band)
}

#[cfg(test)]
pub(crate) mod test_support {
    pub fn copies_made_on_this_thread() -> usize {
        super::COPIES_MADE.with(std::cell::Cell::get)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use ndarray::Array2;

    use super::test_support::copies_made_on_this_thread;
    use super::*;
    use crate::core::astrometry::gwcs::test_support::env_dir;
    use crate::infra::cache::ImageCache;
    use crate::infra::fits::bzip::test_support::bzip2_copy;
    use crate::infra::fits::dispatcher::test_support::write_zip;
    use crate::infra::fits::dispatcher::{list_source_extensions, resolve_single_image_fresh};
    use crate::infra::fits::gzip::test_support::gzip_copy;
    use crate::infra::fits::gzip::GZIP_BYTES_REFUSAL;
    use crate::infra::fits::reader::test_fixtures::{cube_hdu, empty_primary_cards, plane_hdu, write_raw_hdus};
    use crate::infra::fits::reader::{
        auto_hdu_index, decode_pixels, decode_pixels_blank, extract_image_mmap, extract_image_planes_by_index,
        extract_int_plane_by_index, load_fits_image, try_extract_rgb_mmap,
    };
    use crate::infra::image_source::{list_planes, load_plane, load_plane_header};
    use crate::service::common::{cached_header, load_cached};

    type FixtureHdu = (Vec<(&'static str, String)>, Vec<u8>);

    const NEEDS_A_CUBE: &str = "a plane key needs an image HDU with exactly three axes";

    const THREE_BAND_CUBE: [&str; 12] = [
        "SIMPLE  =                    T",
        "BITPIX  =                  -32",
        "NAXIS   =                    3",
        "NAXIS1  =                    4",
        "NAXIS2  =                    3",
        "NAXIS3  =                    3",
        "EXTEND  =                    T",
        "BANDS   = 'gri     '",
        "BAND0   = 'g       '",
        "BAND1   = 'r       '",
        "BAND2   = 'i       '",
        "OBJECT  = 'cut     '           / target",
    ];

    const SCALED_CUBE: [&str; 29] = [
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    3",
        "NAXIS1  =                    2",
        "NAXIS2  =                    2",
        "NAXIS3  =                    2",
        "EXTEND  =                    T",
        "BSCALE  =                  0.5",
        "BZERO   =                 10.0",
        "BLANK   =               -32768",
        "BUNIT   = 'nanomaggy'",
        "CTYPE1  = 'RA---TAN'",
        "CTYPE2  = 'DEC--TAN'",
        "CTYPE3  = 'BAND    '",
        "CRVAL3  =                  1.0",
        "CDELT3  =                  1.0",
        "CD1_1   =             -7.5E-05",
        "CD3_3   =                  1.0",
        "PC3_1   =                  0.0",
        "PV3_1   =                  0.0",
        "WCSAXES =                    3",
        "CHECKSUM= 'abcdefghijklmnop'",
        "DATASUM = '123     '",
        "FILENAME= 'a_very_long_name_&'",
        "CONTINUE  'that_continues.fits'",
        "HIERARCH ESO DET DIT = 10.0 / integration time",
        "COMMENT   first note",
        "HISTORY   made for a test",
        "ABPLANE =                    7",
    ];

    const EMPTY_PRIMARY: [&str; 9] = [
        "SIMPLE  =                    T",
        "BITPIX  =                    8",
        "NAXIS   =                    0",
        "EXTEND  =                    T",
        "TELESCOP= 'X       '",
        "OBJECT  = 'P       '",
        "BSCALE  =                  2.0",
        "BUNIT   = 'adu     '",
        "COMMENT   primary note",
    ];

    const SCI_CUBE_EXTENSION: [&str; 11] = [
        "XTENSION= 'IMAGE   '",
        "BITPIX  =                  -32",
        "NAXIS   =                    3",
        "NAXIS1  =                    4",
        "NAXIS2  =                    3",
        "NAXIS3  =                    2",
        "PCOUNT  =                    0",
        "GCOUNT  =                    1",
        "EXTNAME = 'SCI     '",
        "OBJECT  = 'E       '",
        "COMMENT   extension note",
    ];

    const ASINH_CARDS: [&str; 2] = ["BSOFTEN =                 0.25", "BOFFSET =                  3.5"];

    fn raw_header_block(cards: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for card in cards.iter().chain(std::iter::once(&"END")) {
            let mut bytes = card.as_bytes().to_vec();
            assert!(bytes.len() <= 80, "card longer than 80 bytes: {card}");
            bytes.resize(80, b' ');
            out.extend_from_slice(&bytes);
        }
        while out.len() % BLOCK_SIZE != 0 {
            out.push(b' ');
        }
        out
    }

    fn fits_bytes(hdus: &[(&[&str], Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (cards, data) in hdus {
            out.extend(raw_header_block(cards));
            out.extend_from_slice(data);
            out.resize(out.len().div_ceil(BLOCK_SIZE) * BLOCK_SIZE, 0);
        }
        out
    }

    fn f32_be(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn i16_be(values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn ramp(count: usize) -> Vec<f32> {
        (0..count).map(|i| i as f32).collect()
    }

    fn edited(records: &[&'static str], replacements: &[&'static str], additions: &[&'static str]) -> Vec<&'static str> {
        records
            .iter()
            .map(|record| *replacements.iter().find(|new| new[..8] == record[..8]).unwrap_or(record))
            .chain(additions.iter().copied())
            .collect()
    }

    fn three_band_cube() -> Vec<u8> {
        fits_bytes(&[(&THREE_BAND_CUBE, f32_be(&ramp(36)))])
    }

    fn sci_cube_file(primary_extra: &[&'static str], extension_extra: &[&'static str]) -> Vec<u8> {
        let primary = edited(&EMPTY_PRIMARY, &[], primary_extra);
        let extension = edited(&SCI_CUBE_EXTENSION, &[], extension_extra);
        fits_bytes(&[(&primary, Vec::new()), (&extension, f32_be(&ramp(24)))])
    }

    fn compressed_cube_hdu() -> FixtureHdu {
        let cards: Vec<(&'static str, String)> = vec![
            ("XTENSION", "'BINTABLE'".into()),
            ("BITPIX", "8".into()),
            ("NAXIS", "2".into()),
            ("NAXIS1", "8".into()),
            ("NAXIS2", "3".into()),
            ("PCOUNT", "0".into()),
            ("GCOUNT", "1".into()),
            ("TFIELDS", "1".into()),
            ("TTYPE1", "'COMPRESSED_DATA'".into()),
            ("TFORM1", "'1PB     '".into()),
            ("ZIMAGE", "T".into()),
            ("ZCMPTYPE", "'RICE_1  '".into()),
            ("ZBITPIX", "16".into()),
            ("ZNAXIS", "3".into()),
            ("ZNAXIS1", "4".into()),
            ("ZNAXIS2", "3".into()),
            ("ZNAXIS3", "5".into()),
            ("EXTNAME", "'SCI     '".into()),
        ];
        (cards, vec![0u8; 24])
    }

    fn write_fixture(dir: &tempfile::TempDir, name: &str, hdus: &[FixtureHdu]) -> String {
        let path = dir.path().join(name);
        write_raw_hdus(&path, hdus);
        path.to_str().unwrap().to_string()
    }

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (key, value) in cards {
            h.set(key, value.to_string());
        }
        h
    }

    fn labels(cards: &[(&str, &str)], planes: usize) -> Vec<String> {
        band_labels(&header(cards), planes)
    }

    fn refusal(bytes: &[u8], hdu: usize, plane: usize) -> String {
        format!("{:#}", band_plane_file_bytes(bytes, hdu, plane).unwrap_err())
    }

    fn listed_planes(file: &str, hdus: &[usize], planes: usize, labels: &[&str]) -> Vec<HduBandPlanes> {
        hdus.iter()
            .map(|&hdu| HduBandPlanes {
                hdu,
                planes,
                bands: (0..planes)
                    .map(|plane| BandPlane {
                        plane,
                        label: labels.get(plane).map(|label| label.to_string()),
                        key: format!("{file}#hdu={hdu}&plane={plane}"),
                    })
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn labels_come_from_the_band_n_cards() {
        assert_eq!(labels(&[("BANDS", "griz"), ("BAND0", "g"), ("BAND1", "r"), ("BAND2", "i"), ("BAND3", "z")], 4), ["g", "r", "i", "z"]);
        assert_eq!(labels(&[("BAND0", "x"), ("BAND1", "y"), ("BANDS", "gr")], 2), ["x", "y"]);
        assert_eq!(labels(&[("BAND0", "abcdefghijklmnop")], 1), ["abcdefghijklmnop"]);
    }

    #[test]
    fn labels_fall_back_to_the_bands_card() {
        assert_eq!(labels(&[("BANDS", "grz")], 3), ["g", "r", "z"]);
        assert_eq!(labels(&[("BANDS", "F090W, F200W")], 2), ["F090W", "F200W"]);
        assert_eq!(labels(&[("BANDS", "VIS Y J H")], 4), ["VIS", "Y", "J", "H"]);
        assert_eq!(labels(&[("BAND0", "g"), ("BAND1", ""), ("BANDS", "gri")], 3), ["g", "r", "i"]);
    }

    #[test]
    fn incomplete_or_mismatched_cards_give_no_labels() {
        let cases: [(&[(&str, &str)], usize); 7] = [
            (&[("BAND0", "g"), ("BAND1", "r")], 3),
            (&[("BANDS", "griz")], 3),
            (&[("BAND0", "g"), ("BAND1", "r"), ("BAND2", "i"), ("BAND3", "z"), ("BANDS", "griz")], 3),
            (&[("BAND0", "it's")], 1),
            (&[("BAND0", "abcdefghijklmnopq")], 1),
            (&[("OBJECT", "M31")], 2),
            (&[("BANDS", "g")], 0),
        ];
        for (cards, planes) in cases {
            assert!(labels(cards, planes).is_empty(), "{cards:?} with {planes} planes");
        }
    }

    #[test]
    fn a_file_lists_the_planes_of_its_cubes_with_labels_and_keys() {
        let dir = tempfile::tempdir().unwrap();
        let primary: Vec<(&'static str, String)> = vec![
            ("SIMPLE", "T".into()),
            ("BITPIX", "-32".into()),
            ("NAXIS", "3".into()),
            ("NAXIS1", "4".into()),
            ("NAXIS2", "3".into()),
            ("NAXIS3", "4".into()),
            ("EXTEND", "T".into()),
            ("BANDS", "'griz    '".into()),
            ("BAND0", "'g       '".into()),
            ("BAND1", "'r       '".into()),
            ("BAND2", "'i       '".into()),
            ("BAND3", "'z       '".into()),
        ];
        let path = write_fixture(
            &dir,
            "cut.fits",
            &[
                (primary, f32_be(&ramp(48))),
                cube_hdu("INVVAR", 4, 3, 4, &[("BANDS", "'griz    '".into())]),
                plane_hdu("MASK", 4, 3),
            ],
        );

        let listed = list_band_planes(&path).unwrap();
        let expected = listed_planes(&path, &[0, 1], 4, &["g", "r", "i", "z"]);
        assert_eq!(listed, expected);
        assert_eq!(list_band_planes(&format!("{path}#hdu=2")).unwrap(), expected);
        assert_eq!(list_band_planes(&format!("{path}#hdu=0&plane=1")).unwrap(), expected);
        assert_eq!(
            serde_json::to_value(&listed[0]).unwrap(),
            serde_json::json!({
                "hdu": 0,
                "planes": 4,
                "bands": [
                    { "plane": 0, "label": "g", "key": format!("{path}#hdu=0&plane=0") },
                    { "plane": 1, "label": "r", "key": format!("{path}#hdu=0&plane=1") },
                    { "plane": 2, "label": "i", "key": format!("{path}#hdu=0&plane=2") },
                    { "plane": 3, "label": "z", "key": format!("{path}#hdu=0&plane=3") },
                ],
            })
        );
    }

    #[test]
    fn cubes_without_band_cards_list_unlabelled_planes_and_long_cubes_list_none() {
        let dir = tempfile::tempdir().unwrap();
        let three = write_fixture(&dir, "three.fits", &[(empty_primary_cards(), Vec::new()), cube_hdu("SCI", 4, 3, 3, &[])]);
        let listed = list_band_planes(&three).unwrap();
        assert_eq!(listed, listed_planes(&three, &[1], 3, &[]));
        assert_eq!(serde_json::to_value(&listed[0].bands[0]).unwrap()["label"], serde_json::Value::Null);

        let long = write_fixture(&dir, "long.fits", &[(empty_primary_cards(), Vec::new()), cube_hdu("SCI", 4, 3, 17, &[])]);
        assert_eq!(list_band_planes(&long).unwrap(), [HduBandPlanes { hdu: 1, planes: 17, bands: Vec::new() }]);

        let single = write_fixture(&dir, "single.fits", &[(empty_primary_cards(), Vec::new()), cube_hdu("SCI", 4, 3, 1, &[])]);
        assert!(list_band_planes(&single).unwrap().is_empty());

        let compressed = write_fixture(&dir, "compressed.fits", &[(empty_primary_cards(), Vec::new()), compressed_cube_hdu()]);
        assert!(list_band_planes(&compressed).unwrap().is_empty());

        let flat = write_fixture(
            &dir,
            "flat.fits",
            &[(empty_primary_cards(), Vec::new()), plane_hdu("SCI", 4, 3), plane_hdu("ERR", 4, 3)],
        );
        assert!(list_band_planes(&flat).unwrap().is_empty());

        let missing = dir.path().join("missing.fits");
        let missing = missing.to_str().unwrap();
        let error = format!("{:#}", list_band_planes(missing).unwrap_err());
        assert!(error.contains(missing), "{error}");
    }

    #[test]
    fn the_copy_of_a_plane_is_one_image_hdu_holding_the_bytes_of_that_plane() {
        let bytes = three_band_cube();
        let copy = band_plane_file_bytes(&bytes, 0, 1).unwrap();
        assert_eq!(copy.len(), 5760);
        assert_eq!(&copy[..30], b"SIMPLE  =                    T");

        let parsed = parse_header_at(&copy, 0).unwrap();
        assert_eq!(parsed.data_start, 2880);
        assert_eq!(parsed.next_hdu_offset, 5760);
        let h = &parsed.header;
        let keys: Vec<&str> = h.cards.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            keys,
            ["SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "BANDS", "BAND0", "BAND1", "BAND2", "OBJECT", "ABPLHDU", "ABPLANE", "ABPLBAND"]
        );
        assert_eq!(h.get_i64("BITPIX"), Some(-32));
        assert_eq!(h.get_i64("NAXIS"), Some(2));
        assert_eq!(h.get_i64("NAXIS1"), Some(4));
        assert_eq!(h.get_i64("NAXIS2"), Some(3));
        assert_eq!(h.get("OBJECT"), Some("cut"));
        assert_eq!(h.get_i64("ABPLHDU"), Some(0));
        assert_eq!(h.get_i64("ABPLANE"), Some(1));
        assert_eq!(h.get("ABPLBAND"), Some("r"));
        assert_eq!(h.is_string_value("ABPLBAND"), Some(true));

        assert_eq!(&copy[2880..2928], &bytes[2880 + 48..2880 + 96]);
        assert!(copy[2928..].iter().all(|&b| b == 0));
        assert_eq!(decode_pixels(&copy[2880..2928], -32, 1.0, 0.0), (12..24).map(|i| i as f32).collect::<Vec<_>>());

        assert_eq!(loaded_band_plane(h), Some(LoadedBandPlane { hdu: 0, plane: 1, label: Some("r".into()) }));
        assert_eq!(loaded_band_plane(&parse_header_at(&bytes, 0).unwrap().header), None);
    }

    #[test]
    fn the_copy_keeps_scaling_long_strings_and_commentary_and_drops_layout_and_third_axis_cards() {
        let bytes = fits_bytes(&[(&SCALED_CUBE, i16_be(&[0, 0, 0, 0, 1, 2, -32768, 4]))]);
        let copy = band_plane_file_bytes(&bytes, 0, 1).unwrap();
        let h = parse_header_at(&copy, 0).unwrap().header;

        assert_eq!(h.get_f64("BSCALE"), Some(0.5));
        assert_eq!(h.get_f64("BZERO"), Some(10.0));
        assert_eq!(h.get_i64("BLANK"), Some(-32768));
        assert_eq!(h.get("BUNIT"), Some("nanomaggy"));
        assert_eq!(h.get("CTYPE1"), Some("RA---TAN"));
        assert_eq!(h.get_f64("CD1_1"), Some(-7.5e-5));
        assert_eq!(h.get("FILENAME"), Some("a_very_long_name_that_continues.fits"));
        assert_eq!(h.get_f64("HIERARCH ESO DET DIT"), Some(10.0));
        assert!(h.cards.contains(&("COMMENT".to_string(), "first note".to_string())));
        assert!(h.cards.contains(&("HISTORY".to_string(), "made for a test".to_string())));
        for key in ["NAXIS3", "EXTEND", "CTYPE3", "CRVAL3", "CDELT3", "CD3_3", "PC3_1", "PV3_1", "WCSAXES", "CHECKSUM", "DATASUM", "ABPLBAND"] {
            assert_eq!(h.get(key), None, "{key}");
        }
        assert_eq!(h.get_i64("ABPLANE"), Some(1));
        assert_eq!(h.cards.iter().filter(|(key, _)| key == "ABPLANE").count(), 1);
        assert_eq!(h.get_i64("ABPLHDU"), Some(0));

        let pixels = decode_pixels_blank(&copy[2880..2888], 16, 0.5, 10.0, Some(-32768));
        assert_eq!(pixels.len(), 4);
        assert_eq!((pixels[0], pixels[1], pixels[3]), (10.5, 11.0, 12.0));
        assert!(pixels[2].is_nan(), "{pixels:?}");
    }

    #[test]
    fn a_plane_of_an_extension_cube_inherits_the_primary_cards_it_lacks() {
        let copy = band_plane_file_bytes(&sci_cube_file(&[], &[]), 1, 0).unwrap();
        assert_eq!(&copy[..30], b"SIMPLE  =                    T");
        let h = parse_header_at(&copy, 0).unwrap().header;

        assert_eq!(h.get("OBJECT"), Some("E"));
        assert_eq!(h.get("TELESCOP"), Some("X"));
        assert_eq!(h.get("EXTNAME"), Some("SCI"));
        assert_eq!(h.get("BUNIT"), Some("adu"));
        for key in ["BSCALE", "XTENSION", "PCOUNT", "GCOUNT"] {
            assert_eq!(h.get(key), None, "{key}");
        }
        let commentary: Vec<(&str, &str)> = h
            .cards
            .iter()
            .filter(|(key, _)| key == "COMMENT" || key == "HISTORY")
            .map(|(key, text)| (key.as_str(), text.as_str()))
            .collect();
        assert_eq!(commentary, [("COMMENT", "extension note"), ("COMMENT", "primary note")]);
        assert_eq!(h.get_i64("ABPLHDU"), Some(1));
        assert_eq!(h.get_i64("ABPLANE"), Some(0));
        assert_eq!(decode_pixels(&copy[2880..2928], -32, 1.0, 0.0), ramp(12));
    }

    #[test]
    fn planes_that_cannot_be_copied_are_refused_with_the_reason() {
        let cube = three_band_cube();
        assert_eq!(refusal(&cube, 5, 0), "HDU index 5 out of range (file has 1 HDUs)");

        let flat = fits_bytes(&[(
            &[
                "SIMPLE  =                    T",
                "BITPIX  =                  -32",
                "NAXIS   =                    2",
                "NAXIS1  =                    4",
                "NAXIS2  =                    3",
            ],
            f32_be(&ramp(12)),
        )]);
        assert_eq!(refusal(&flat, 0, 0), format!("HDU 0 is not a cube of images (NAXIS = 2, 4 x 3); {NEEDS_A_CUBE}"));

        let four_axes = edited(&THREE_BAND_CUBE, &["NAXIS   =                    4"], &["NAXIS4  =                    1"]);
        let four_axes = fits_bytes(&[(&four_axes, f32_be(&ramp(36)))]);
        assert_eq!(refusal(&four_axes, 0, 0), format!("HDU 0 is not a cube of images (NAXIS = 4, 4 x 3); {NEEDS_A_CUBE}"));

        let one_column = edited(&THREE_BAND_CUBE, &["NAXIS1  =                    1"], &[]);
        let one_column = fits_bytes(&[(&one_column, f32_be(&ramp(9)))]);
        assert_eq!(refusal(&one_column, 0, 0), format!("HDU 0 is not a cube of images (NAXIS = 3, 1 x 3); {NEEDS_A_CUBE}"));

        let table = fits_bytes(&[
            (&EMPTY_PRIMARY[..4], Vec::new()),
            (
                &[
                    "XTENSION= 'BINTABLE'",
                    "BITPIX  =                    8",
                    "NAXIS   =                    2",
                    "NAXIS1  =                    4",
                    "NAXIS2  =                    2",
                    "PCOUNT  =                    0",
                    "GCOUNT  =                    1",
                    "TFIELDS =                    1",
                    "TTYPE1  = 'FLUX    '",
                    "TFORM1  = '1E      '",
                ],
                vec![0u8; 8],
            ),
        ]);
        assert_eq!(refusal(&table, 1, 0), format!("HDU 1 is a BINTABLE extension; {NEEDS_A_CUBE}"));

        let dir = tempfile::tempdir().unwrap();
        let compressed = write_fixture(&dir, "compressed.fits", &[(empty_primary_cards(), Vec::new()), compressed_cube_hdu()]);
        let compressed = std::fs::read(compressed).unwrap();
        assert_eq!(
            refusal(&compressed, 1, 0),
            "HDU 1 is a tile-compressed image; a plane key addresses an uncompressed cube"
        );

        assert_eq!(refusal(&cube, 0, 3), "plane 3 is out of range: HDU 0 has 3 planes, numbered 0 to 2");

        let truncated = &cube[..2880 + 96 + 10];
        assert_eq!(
            refusal(truncated, 0, 2),
            "HDU 0 declares 3 planes of 4 x 3 pixels but the file ends inside plane 2"
        );
        assert_eq!(band_plane_file_bytes(truncated, 0, 0).unwrap(), band_plane_file_bytes(&cube, 0, 0).unwrap());

        let odd_bitpix = edited(&THREE_BAND_CUBE, &["BITPIX  =                   12"], &[]);
        let odd_bitpix = fits_bytes(&[(&odd_bitpix, vec![0u8; 36])]);
        assert_eq!(refusal(&odd_bitpix, 0, 0), "HDU 0 has BITPIX = 12, which is not an image type");

        assert_eq!(
            refusal(&sci_cube_file(&ASINH_CARDS, &[]), 1, 0),
            "HDU 1 has no BSOFTEN or BOFFSET card but the primary HDU has: it is not stated whether these planes are asinh-scaled, so a plane key is refused"
        );
        let own_asinh = band_plane_file_bytes(&sci_cube_file(&[], &ASINH_CARDS), 1, 0).unwrap();
        let own_asinh = parse_header_at(&own_asinh, 0).unwrap().header;
        assert_eq!(own_asinh.get_f64("BSOFTEN"), Some(0.25));
        assert_eq!(own_asinh.get_f64("BOFFSET"), Some(3.5));

        let gzip = [&[0x1F, 0x8B][..], &[0u8; 64][..]].concat();
        assert_eq!(refusal(&gzip, 0, 0), GZIP_BYTES_REFUSAL);

        assert!(band_plane_file_bytes(&[], 0, 0).is_err());
    }

    const LEGACY_PIXEL_BYTES: usize = 458 * 458 * 4;

    const LEGACY_PLANES: [(usize, usize, &str, &str, f32, f32); 8] = [
        (0, 0, "g", "27188bcdebb6b0fbfa19eb800db7d59a79534fc9229e0535db19b73582a635df", -0.009764722548425198, 1.578244686126709),
        (0, 1, "r", "c5feb34d90afa503f72f12db087bc05c94bff66560558d25d4a9b0d656e88a72", -0.011948428116738796, 4.3409810066223145),
        (0, 2, "i", "7ef65d69ed45273e50a2b8c71284d7acce40fec3ce08d62e3aa66bee41d5b20a", -0.013855567201972008, 6.782732963562012),
        (0, 3, "z", "e036edb7ae31ce40da3eaaf8e35e3352a3144820d4c93e9f39e00634327e5782", -0.03435531631112099, 9.024134635925293),
        (1, 0, "g", "63312fa3e3579fb732ded55958272824cd19b42af4d8e0f789730d7eb5ddefea", 46756.796875, 2825193.75),
        (1, 1, "r", "ba5f6ce60e0964f15cc6be48e426cfdb9d41b9443281eb96349c455ee9e93fde", 16778.505859375, 2206574.0),
        (1, 2, "i", "b118676935ea82ab846b3293b593b47ac01d0a8b050f4ee3acdd948fab232a66", 6203.28857421875, 780587.125),
        (1, 3, "z", "b7b09289c6d1a3eb09da1016069b47ea3d3904d3e98a61e3dbd810a8c82b4b61", 1623.225341796875, 209983.375),
    ];

    const LEGACY_R_SAMPLES: [(usize, usize, f32); 4] = [
        (0, 0, 0.0009306051069870591),
        (228, 229, 0.0005343275261111557),
        (457, 457, 0.28462979197502136),
        (100, 300, -0.0006670705042779446),
    ];

    fn legacy_cutout() -> PathBuf {
        env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join("L1").join("ls-dr10_A_griz_invvar_maskbits_458px.fits")
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_cutout_lists_g_r_i_z_for_its_two_cubes() {
        let path = legacy_cutout();
        let path = path.to_str().unwrap();
        let listed = list_band_planes(path).unwrap();
        for entry in &listed {
            let labels: Vec<Option<&str>> = entry.bands.iter().map(|band| band.label.as_deref()).collect();
            println!("HDU {}: {} planes, labels {labels:?}", entry.hdu, entry.planes);
        }
        assert_eq!(listed, listed_planes(path, &[0, 1], 4, &["g", "r", "i", "z"]));
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_cutout_planes_copy_to_the_pixels_of_the_oracle() {
        let bytes = std::fs::read(legacy_cutout()).unwrap();
        assert_eq!(bytes.len(), 7_565_760);
        for (hdu, plane, label, pixel_sha256, minimum, maximum) in LEGACY_PLANES {
            let copy = band_plane_file_bytes(&bytes, hdu, plane).unwrap();
            let parsed = parse_header_at(&copy, 0).unwrap();
            let h = &parsed.header;
            let pixels = decode_pixels(&copy[2880..2880 + LEGACY_PIXEL_BYTES], -32, 1.0, 0.0);
            let low = pixels.iter().copied().fold(f32::INFINITY, f32::min);
            let high = pixels.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let measured_sha256 = sha256_hex(&pixels.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
            println!(
                "HDU {hdu} plane {plane}: {} bytes, ABPLBAND {:?}, IMAGETYP {:?}, minimum {low:?}, maximum {high:?}, pixel SHA-256 {measured_sha256}",
                copy.len(),
                h.get("ABPLBAND"),
                h.get("IMAGETYP")
            );

            assert_eq!(copy.len(), 843_840, "HDU {hdu} plane {plane}");
            assert_eq!(parsed.data_start, 2880, "HDU {hdu} plane {plane}");
            assert_eq!(h.get_i64("NAXIS"), Some(2));
            assert_eq!(h.get_i64("NAXIS1"), Some(458));
            assert_eq!(h.get_i64("NAXIS2"), Some(458));
            assert_eq!(h.get("NAXIS3"), None);
            assert_eq!(h.get("BANDS"), Some("griz"));
            for (n, band) in ["g", "r", "i", "z"].iter().enumerate() {
                assert_eq!(h.get(&format!("BAND{n}")), Some(*band), "BAND{n}");
            }
            assert_eq!(h.get("SURVEY"), Some("LegacySurvey"));
            assert_eq!(h.get_f64("CRVAL1"), Some(53.13));
            assert_eq!(h.get_f64("CRPIX1"), Some(229.5));
            assert_eq!(h.get_f64("CD1_1"), Some(-7.27777777777778e-5));
            assert_eq!(h.get_i64("ABPLHDU"), Some(hdu as i64));
            assert_eq!(h.get_i64("ABPLANE"), Some(plane as i64));
            assert_eq!(h.get("ABPLBAND"), Some(label));
            assert_eq!(h.get("IMAGETYP"), Some(if hdu == 0 { "IMAGE" } else { "INVVAR" }));
            assert!(copy[2880 + LEGACY_PIXEL_BYTES..].iter().all(|&b| b == 0));

            if (hdu, plane) == (0, 1) {
                for (y, x, value) in LEGACY_R_SAMPLES {
                    assert_eq!(pixels[y * 458 + x], value, "[{y}, {x}]");
                }
            }
            assert_eq!((low, high), (minimum, maximum), "HDU {hdu} plane {plane}");
            assert_eq!(measured_sha256, pixel_sha256, "HDU {hdu} plane {plane}");
        }
        let mask = refusal(&bytes, 2, 0);
        println!("HDU 2: {mask}");
        assert!(mask.contains("NAXIS = 2, 458 x 458"), "{mask}");
    }

    fn primary_image(axes: &[usize], extra: &[(&'static str, String)], values: &[f32]) -> FixtureHdu {
        const AXIS_KEYS: [&str; 3] = ["NAXIS1", "NAXIS2", "NAXIS3"];
        let mut cards: Vec<(&'static str, String)> =
            vec![("SIMPLE", "T".into()), ("BITPIX", "-32".into()), ("NAXIS", axes.len().to_string())];
        cards.extend(AXIS_KEYS.iter().zip(axes).map(|(key, len)| (*key, len.to_string())));
        cards.push(("EXTEND", "T".into()));
        cards.extend(extra.iter().cloned());
        (cards, f32_be(values))
    }

    fn cut_hdus(cube_values: &[f32]) -> Vec<FixtureHdu> {
        let griz: Vec<(&'static str, String)> = vec![
            ("BANDS", "'griz    '".into()),
            ("BAND0", "'g       '".into()),
            ("BAND1", "'r       '".into()),
            ("BAND2", "'i       '".into()),
            ("BAND3", "'z       '".into()),
        ];
        vec![
            primary_image(&[4, 3, 4], &griz, cube_values),
            cube_hdu("INVVAR", 4, 3, 4, &[("BANDS", "'griz    '".into())]),
            plane_hdu("MASK", 4, 3),
        ]
    }

    fn cut_cube(dir: &Path) -> PathBuf {
        let path = dir.join("cut.fits");
        write_raw_hdus(&path, &cut_hdus(&ramp(48)));
        path
    }

    fn key(path: &Path, hdu: usize, plane: usize) -> String {
        format!("{}#hdu={hdu}&plane={plane}", path.to_str().unwrap())
    }

    fn bits(image: &Array2<f32>) -> Vec<u32> {
        image.iter().map(|v| v.to_bits()).collect()
    }

    fn resolve_refusal(key: &str) -> String {
        format!("{:#}", resolve_single_image(key).unwrap_err())
    }

    fn pixel_sha256(image: &Array2<f32>) -> String {
        sha256_hex(&image.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>())
    }

    #[test]
    fn a_band_plane_key_resolves_to_a_copy_that_the_loader_opens_as_that_plane() {
        let dir = tempfile::tempdir().unwrap();
        let path = cut_cube(dir.path());
        for hdu in 0..2 {
            let cube = extract_image_planes_by_index(&File::open(&path).unwrap(), hdu).unwrap();
            for (plane, label) in ["g", "r", "i", "z"].into_iter().enumerate() {
                let plane_key = key(&path, hdu, plane);
                let loaded = load_plane(&ImageRef::parse(&plane_key)).unwrap();
                assert_eq!(loaded.arr.dim(), cube[plane].dim(), "{plane_key}");
                assert_eq!(bits(&loaded.arr), bits(&cube[plane]), "{plane_key}");

                let h = &loaded.header;
                assert_eq!(h.get_i64("NAXIS"), Some(2), "{plane_key}");
                assert_eq!(h.get_i64("NAXIS1"), Some(4), "{plane_key}");
                assert_eq!(h.get_i64("NAXIS2"), Some(3), "{plane_key}");
                assert_eq!(h.get("NAXIS3"), None, "{plane_key}");
                assert_eq!(
                    loaded_band_plane(h),
                    Some(LoadedBandPlane { hdu, plane, label: Some(label.to_string()) }),
                    "{plane_key}"
                );
                if hdu == 1 {
                    assert_eq!(h.get("EXTNAME"), Some("INVVAR"), "{plane_key}");
                }
                assert_eq!(load_plane_header(&ImageRef::parse(&plane_key)).unwrap().cards, h.cards, "{plane_key}");

                let (listed, is_asdf) = list_planes(&plane_key).unwrap();
                assert_eq!((listed.len(), is_asdf), (1, false), "{plane_key}");
                let extensions = list_source_extensions(&plane_key).unwrap();
                assert_eq!(extensions.len(), 1, "{plane_key}");
                assert_eq!((extensions[0].naxis, extensions[0].naxis1, extensions[0].naxis2), (2, 4, 3), "{plane_key}");
            }
        }

        let (copy, guard) = resolve_single_image(&key(&path, 0, 2)).unwrap();
        assert!(guard.is_some());
        assert_eq!(copy.file_name().unwrap(), "cut_hdu0_plane2.fits");
    }

    #[test]
    fn the_image_cache_opens_a_band_plane_and_the_plain_file_opens_on_its_first_band() {
        let dir = tempfile::tempdir().unwrap();
        let path = cut_cube(dir.path());
        let cache = ImageCache::new(8, 1 << 30);
        let plane_key = key(&path, 0, 1);

        let entry = load_cached(&cache, &plane_key).unwrap();
        assert_eq!(entry.arr().dim(), (3, 4));
        assert_eq!(entry.arr()[[0, 0]], 12.0);
        assert_eq!(entry.arr()[[2, 3]], 23.0);
        let copies = copies_made_on_this_thread();
        let again = load_cached(&cache, &plane_key).unwrap();
        assert!(std::ptr::eq(again.arr(), entry.arr()), "the second call must return the cached entry");
        assert_eq!(copies_made_on_this_thread(), copies);
        assert_eq!(cached_header(&cache, &plane_key).unwrap().get_i64("ABPLANE"), Some(1));

        assert!(try_extract_rgb_mmap(&File::open(&path).unwrap()).unwrap().is_none());
        let plain = load_plane(&ImageRef::parse(path.to_str().unwrap())).unwrap();
        assert_eq!(plain.arr.dim(), (3, 4));
        assert_eq!(plain.arr.iter().copied().collect::<Vec<f32>>(), ramp(12));
        let (copy, _guard) = resolve_single_image(&plane_key).unwrap();
        assert!(try_extract_rgb_mmap(&File::open(&copy).unwrap()).unwrap().is_none());
    }

    #[test]
    fn repeated_resolves_reuse_one_copy_until_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = cut_cube(dir.path());
        let plane_key = key(&path, 0, 1);
        let band = BandPlaneKey::parse(&plane_key).unwrap();
        let store = PlaneCopies::new();

        let before = copies_made_on_this_thread();
        let (first, _first_guard) = store.resolve(&band).unwrap();
        for _ in 0..2 {
            assert_eq!(store.resolve(&band).unwrap().0, first);
        }
        assert_eq!(copies_made_on_this_thread(), before + 1);

        let rewritten: Vec<f32> = (0..48).map(|v| 1000.0 + v as f32).collect();
        let mut hdus = cut_hdus(&rewritten);
        hdus.push(plane_hdu("EXTRA", 4, 3));
        write_raw_hdus(&path, &hdus);
        let (second, _second_guard) = store.resolve(&band).unwrap();
        assert_eq!(copies_made_on_this_thread(), before + 2);
        let pixels = extract_image_planes_by_index(&File::open(&second).unwrap(), 0).unwrap();
        assert_eq!(pixels.len(), 1);
        assert_eq!(pixels[0].iter().copied().collect::<Vec<f32>>(), rewritten[12..24]);

        let (one, one_guard) = resolve_single_image_fresh(&plane_key).unwrap();
        let (two, two_guard) = resolve_single_image_fresh(&plane_key).unwrap();
        assert_eq!(copies_made_on_this_thread(), before + 4);
        assert!(one_guard.is_some() && two_guard.is_some());
        assert_ne!(one.parent(), two.parent());
    }

    #[test]
    fn a_store_keeps_only_the_newest_copies_warm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep.fits");
        write_raw_hdus(&path, &[primary_image(&[4, 3, 10], &[], &ramp(120))]);
        let store = PlaneCopies::new();
        let band = |plane| BandPlaneKey::new(path.to_str().unwrap(), 0, plane);

        let copies: Vec<PathBuf> = (0..10).map(|plane| store.resolve(&band(plane)).unwrap().0).collect();
        for (plane, copy) in copies.iter().enumerate() {
            assert_eq!(copy.is_file(), plane >= 2, "plane {plane}");
        }

        let before = copies_made_on_this_thread();
        assert_eq!(store.resolve(&band(5)).unwrap().0, copies[5]);
        assert_eq!(copies_made_on_this_thread(), before);
        store.resolve(&band(0)).unwrap();
        assert_eq!(copies_made_on_this_thread(), before + 1);
    }

    #[test]
    fn planes_of_compressed_containers_resolve_like_the_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = cut_cube(dir.path());
        let plain = load_plane(&ImageRef::parse(&key(&path, 0, 1))).unwrap().arr;
        assert_eq!(plain.iter().copied().collect::<Vec<f32>>(), (12..24).map(|v| v as f32).collect::<Vec<f32>>());

        let zip = dir.path().join("cut.zip");
        write_zip(&zip, &[("a/cut.fits", &std::fs::read(&path).unwrap())]);
        for container in [gzip_copy(&path, "cut.fits.gz"), bzip2_copy(&path, "cut.fits.bz2"), zip] {
            let loaded = load_plane(&ImageRef::parse(&key(&container, 0, 1))).unwrap();
            assert_eq!(loaded.arr, plain, "{}", container.display());
        }
    }

    #[test]
    fn refusals_name_the_key_and_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = cut_cube(dir.path());
        let missing = dir.path().join("missing.fits");
        let asdf = dir.path().join("tree.asdf");
        std::fs::write(&asdf, b"not a cube at all").unwrap();

        for (plane_key, reason) in [
            (key(&path, 2, 0), "is not a cube of images (NAXIS = 2, 4 x 3)"),
            (key(&path, 0, 4), "plane 4 is out of range: HDU 0 has 4 planes, numbered 0 to 3"),
            (key(&path, 9, 0), "HDU index 9 out of range (file has 3 HDUs)"),
            (key(&missing, 0, 0), missing.to_str().unwrap()),
            (key(&asdf, 0, 0), "is an ASDF file; a plane key addresses a FITS cube"),
        ] {
            let error = resolve_refusal(&plane_key);
            for expected in ["cannot open the plane", plane_key.as_str(), reason] {
                assert!(error.contains(expected), "missing {expected}: {error}");
            }
        }

        let unresolved = key(&path, 0, 1);
        let error = format!("{:#}", load_fits_image(&unresolved).unwrap_err());
        assert!(error.contains(&unresolved), "{error}");
    }

    #[test]
    fn a_file_named_like_a_plane_key_is_opened_as_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let flat = dir.path().join("flat.fits");
        write_raw_hdus(&flat, &[primary_image(&[4, 3], &[], &ramp(12))]);
        let odd = dir.path().join("odd.fits#hdu=0&plane=1");
        std::fs::copy(&flat, &odd).unwrap();
        let odd_path = odd.to_str().unwrap();

        let (resolved, guard) = resolve_single_image(odd_path).unwrap();
        assert_eq!(resolved, odd);
        assert!(guard.is_none());
        let loaded = load_plane(&ImageRef::parse(odd_path)).unwrap();
        assert_eq!(loaded.arr.dim(), (3, 4));
        assert_eq!(loaded.arr.iter().copied().collect::<Vec<f32>>(), ramp(12));
    }

    #[test]
    fn asinh_cards_of_a_cube_are_applied_once_to_its_plane() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asinh.fits");
        let asinh = [("BSOFTEN", "0.25".to_string()), ("BOFFSET", "3.5".to_string())];
        write_raw_hdus(&path, &[primary_image(&[2, 2, 2], &asinh, &[0.0, 2.5, 5.0, -5.0, 10.0, 0.0, 2.5, 5.0])]);

        let loaded = load_plane(&ImageRef::parse(&key(&path, 0, 1))).unwrap();
        let expected = [2503.499975f64, 3.5, 5.975, 28.4975];
        assert_eq!(loaded.arr.len(), expected.len());
        for (value, wanted) in loaded.arr.iter().zip(expected) {
            assert!((f64::from(*value) - wanted).abs() <= 1e-6 * wanted.abs(), "{value} against {wanted}");
        }
        let h = &loaded.header;
        assert_eq!(h.get_f64("ABSOFTEN"), Some(0.25));
        assert_eq!(h.get("BSOFTEN"), None);
        assert_eq!(h.get_i64("ABPLANE"), Some(1));
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_cutout_planes_open_as_g_r_i_z_images() {
        let path = legacy_cutout();
        for (hdu, plane, label, wanted_sha256, minimum, maximum) in LEGACY_PLANES {
            let plane_key = key(&path, hdu, plane);
            let loaded = load_plane(&ImageRef::parse(&plane_key)).unwrap();
            let (copy, _guard) = resolve_single_image(&plane_key).unwrap();
            let copy_len = std::fs::metadata(&copy).unwrap().len();
            let h = &loaded.header;
            let low = loaded.arr.iter().copied().fold(f32::INFINITY, f32::min);
            let high = loaded.arr.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let measured_sha256 = pixel_sha256(&loaded.arr);
            println!(
                "HDU {hdu} plane {plane}: shape {:?}, copy {copy_len} bytes, ABPLBAND {:?}, IMAGETYP {:?}, minimum {low:?}, maximum {high:?}, pixel SHA-256 {measured_sha256}",
                loaded.arr.dim(),
                h.get("ABPLBAND"),
                h.get("IMAGETYP")
            );

            assert_eq!(loaded.arr.dim(), (458, 458), "{plane_key}");
            assert_eq!(copy_len, 843_840, "{plane_key}");
            assert_eq!(h.get("ABPLBAND"), Some(label), "{plane_key}");
            assert_eq!(h.get("IMAGETYP"), Some(if hdu == 0 { "IMAGE" } else { "INVVAR" }), "{plane_key}");
            assert_eq!(h.get_i64("NAXIS"), Some(2), "{plane_key}");
            assert_eq!(h.get("NAXIS3"), None, "{plane_key}");
            assert_eq!(h.get_f64("CRVAL1"), Some(53.13), "{plane_key}");
            assert_eq!(h.get_f64("CD1_1"), Some(-7.27777777777778e-5), "{plane_key}");
            if (hdu, plane) == (0, 1) {
                for (y, x, value) in LEGACY_R_SAMPLES {
                    assert_eq!(loaded.arr[[y, x]], value, "[{y}, {x}]");
                }
            }
            assert_eq!((low, high), (minimum, maximum), "{plane_key}");
            assert_eq!(measured_sha256, wanted_sha256, "{plane_key}");
        }
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/L1/ls-dr10_A_griz_invvar_maskbits_458px.fits"]
    fn real_legacy_cutout_opens_on_its_g_band_and_its_mask_stays_exact() {
        let path = legacy_cutout();
        let file = File::open(&path).unwrap();
        let rgb = try_extract_rgb_mmap(&file).unwrap().is_some();
        let auto = auto_hdu_index(&file).map_err(|e| format!("{e:#}"));
        let plain = load_plane(&ImageRef::parse(path.to_str().unwrap())).unwrap();
        let plain_sha256 = pixel_sha256(&plain.arr);
        let selected_extension = extract_image_mmap(&file).unwrap().selected_extension;
        println!(
            "colour composite {rgb}, auto HDU {auto:?}, plain path shape {:?}, pixel SHA-256 {plain_sha256}, selected extension {selected_extension:?}",
            plain.arr.dim()
        );

        let mask = extract_int_plane_by_index(&file, 2).unwrap();
        let values: Vec<i32> = mask.bits.iter().map(|&raw| raw as i32).collect();
        let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
        for value in &values {
            *counts.entry(*value).or_default() += 1;
        }
        let mask_sha256 = sha256_hex(&values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>());
        let refusal = resolve_refusal(&key(&path, 2, 0));
        println!(
            "mask: shape {:?}, signed {}, maximum {:?}, SHA-256 {mask_sha256}, counts {counts:?}",
            mask.bits.dim(),
            mask.signed,
            values.iter().max()
        );
        println!("HDU 2 as a plane key: {refusal}");

        assert!(!rgb);
        assert_eq!(auto, Ok(0));
        assert_eq!(plain.arr.dim(), (458, 458));
        assert_eq!(plain_sha256, LEGACY_PLANES[0].3);
        assert_eq!(selected_extension.as_deref(), Some("HDU 0 (plane 1 of 4)"));
        assert_eq!(mask.bits.dim(), (458, 458));
        assert!(mask.signed);
        assert_eq!(values.iter().max(), Some(&2048));
        assert_eq!(mask_sha256, "69b449f514105af2ec18a762bb7a0dc511f02d47cda03fb435ecdca6fffebe39");
        assert_eq!(
            counts.into_iter().collect::<Vec<_>>(),
            [(0, 146_279), (1, 31_087), (256, 7), (1024, 22_590), (1025, 8_968), (1280, 161), (2048, 672)]
        );
        assert!(refusal.contains("NAXIS = 2, 458 x 458"), "{refusal}");
    }
}
