use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use cdshealpix::compass_point::Cardinal;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};
use xz2::write::XzEncoder;

use astroburst_lib::core::astrometry::frames::icrs_to_galactic;
use astroburst_lib::core::astrometry::healpix_cat::{
    decode_star, write_catalog, CatHeader, CatType, CatalogStar, HealpixCatalog, Record,
    COMPACT_COLOUR_LIMIT, COMPACT_MAG_RANGE, HEADER_LEN,
};
use astroburst_lib::core::astrometry::wcs::angular_separation;

const USAGE: &str = "usage:
  build_catalog tycho2 --input <CDS I/259 folder> --out <file.abcat> [--epoch 2026.0] [--level 6] [--record auto|6|12]
  build_catalog twomass --input <psc_*.gz file or folder> --out <file.abcat> [--per-pixel 32]
  build_catalog measure [--catalog <file.abcat>] [--level 6]";
const TYCHO2_TITLE: &str = "ASTROBURST-TYC2-V1";
const TWOMASS_TITLE: &str = "ASTROBURST-2MASS-V1";
const TIER0_LEVEL: u8 = 6;
const TIER1_LEVEL: u8 = 8;
const TIER0_EPOCH_YEAR: f64 = 2026.0;
const TYCHO2_MEAN_EPOCH_YEAR: f64 = 2000.0;
const TYCHO2_OBSERVED_EPOCH_BASE: f64 = 1990.0;
const SUPPLEMENT_EPOCH_YEAR: f64 = 1991.25;
const TWOMASS_EPOCH_YEAR: f32 = 2000.0;
const TWOMASS_PER_PIXEL: usize = 32;
const TYCHO2_PARTS: usize = 20;
const TYCHO2_FIELDS: usize = 32;
const SUPPLEMENT_MIN_FIELDS: usize = 16;
const TWOMASS_FIELDS: usize = 60;
const TWOMASS_NULL: &str = "\\N";
const MAS_PER_DEGREE: f64 = 3_600_000.0;
const ARCSEC_PER_DEGREE: f64 = 3600.0;
const ARCMIN_PER_DEGREE: f64 = 60.0;
const SKY_DEG2: f64 = 41_252.961_249_419_27;
const COMPACT6_LIMIT_ARCSEC: f64 = i16::MAX as f64 / 10.0;
const COMPACT6_MARGIN: f64 = 0.10;
const EDGE_SEGMENTS: u32 = 64;
const ZSTD_LEVEL: i32 = 19;
const XZ_LEVEL: u32 = 9;
const LATITUDE_BINS: [(f64, f64); 8] = [
    (0.0, 5.0),
    (5.0, 10.0),
    (10.0, 20.0),
    (20.0, 30.0),
    (30.0, 45.0),
    (45.0, 60.0),
    (60.0, 75.0),
    (75.0, 90.0),
];
const POLE_CAP_DEG: f64 = 80.0;
const MAG_LIMITS: [f64; 9] = [5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0];
const STAR_TARGETS: [f64; 2] = [30.0, 50.0];
const FIELD_ASPECT: f64 = 1.5;
const DENSITY_PERCENTILE: f64 = 0.10;
const DENSITY_LEVEL: u8 = 6;
const PLEIADES_CONE: (f64, f64, f64) = (56.75, 24.12, 1.0);
const INTERIOR_MARGIN_DEG: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PositionKind {
    Mean,
    Observed,
    Supplement,
}

#[derive(Debug, Clone, PartialEq)]
struct SourceStar {
    tyc: [u32; 3],
    kind: PositionKind,
    ra_deg: f64,
    dec_deg: f64,
    epoch_year: f64,
    pm_ra_masyr: Option<f64>,
    pm_dec_masyr: Option<f64>,
    bt: Option<f64>,
    vt: Option<f64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Clamps {
    magnitude: usize,
    colour: usize,
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Tier0Stats {
    stars: usize,
    without_vt: usize,
    clamps: Clamps,
    max_offset_arcsec: f64,
}

struct InputFile {
    name: String,
    bytes: u64,
    sha256: String,
    lines: usize,
}

struct Tycho2Input {
    stars: Vec<SourceStar>,
    main: usize,
    supplement: usize,
    files: Vec<InputFile>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TwoMassSource {
    ra_deg: f64,
    dec_deg: f64,
    j: f64,
    k: Option<f64>,
}

struct TwoMassCut {
    level: u8,
    per_pixel: usize,
    pixels: HashMap<u32, Vec<TwoMassSource>>,
}

struct TwoMassInput {
    cut: TwoMassCut,
    files: Vec<InputFile>,
    lines: usize,
    without_j: usize,
    rejected: usize,
    dec_range: (f64, f64),
}

#[derive(Debug, Clone, Copy)]
struct CellBound {
    level: u8,
    vertex_deg: f64,
    vertex_pixel: u64,
    edge_deg: f64,
    axis_arcsec: f64,
    axis_pixel: u64,
}

struct Options(HashMap<String, String>);

impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut values = HashMap::new();
        let mut rest = args.iter();
        while let Some(key) = rest.next() {
            if !key.starts_with("--") {
                bail!("{key:?} is not an option\n{USAGE}");
            }
            let value = rest
                .next()
                .ok_or_else(|| anyhow!("{key} needs a value\n{USAGE}"))?;
            if values.insert(key.clone(), value.clone()).is_some() {
                bail!("{key} is given twice");
            }
        }
        Ok(Self(values))
    }

    fn only(&self, allowed: &[&str]) -> Result<()> {
        match self.0.keys().find(|key| !allowed.contains(&key.as_str())) {
            Some(key) => bail!("{key} is not an option of this subcommand\n{USAGE}"),
            None => Ok(()),
        }
    }

    fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        self.text(key)
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("{key} is required\n{USAGE}"))
    }

    fn parsed<T: FromStr>(&self, key: &str, default: T) -> Result<T> {
        match self.text(key) {
            None => Ok(default),
            Some(text) => text
                .parse::<T>()
                .map_err(|_| anyhow!("{key} {text:?} is not a valid value")),
        }
    }
}

fn present(field: &str) -> Option<&str> {
    let text = field.trim();
    (!text.is_empty() && text != TWOMASS_NULL).then_some(text)
}

fn optional_number(field: &str, name: &str) -> Result<Option<f64>, String> {
    present(field)
        .map(|text| {
            text.parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| format!("{name} {text:?} is not a number"))
        })
        .transpose()
}

fn required_number(field: &str, name: &str) -> Result<f64, String> {
    optional_number(field, name)?.ok_or_else(|| format!("{name} is blank"))
}

fn tyc_id(field: &str) -> Result<[u32; 3], String> {
    let parts = field
        .split_whitespace()
        .map(|part| {
            part.parse::<u32>()
                .map_err(|_| format!("TYC part {part:?} is not a number"))
        })
        .collect::<Result<Vec<u32>, String>>()?;
    <[u32; 3]>::try_from(parts)
        .map_err(|parts| format!("TYC {field:?} has {} parts, not 3", parts.len()))
}

fn fields_of(line: &str) -> Vec<&str> {
    line.trim_end_matches(['\r', '\n']).split('|').collect()
}

fn parse_tycho2_line(line: &str) -> Result<SourceStar, String> {
    let fields = fields_of(line);
    if fields.len() != TYCHO2_FIELDS {
        return Err(format!(
            "{} fields, the tyc2.dat layout has {TYCHO2_FIELDS}",
            fields.len()
        ));
    }
    let tyc = tyc_id(fields[0])?;
    let bt = optional_number(fields[17], "BTmag")?;
    let vt = optional_number(fields[19], "VTmag")?;
    match fields[1].trim() {
        "" | "P" => Ok(SourceStar {
            tyc,
            kind: PositionKind::Mean,
            ra_deg: required_number(fields[2], "RAmdeg")?,
            dec_deg: required_number(fields[3], "DEmdeg")?,
            epoch_year: TYCHO2_MEAN_EPOCH_YEAR,
            pm_ra_masyr: optional_number(fields[4], "pmRA")?,
            pm_dec_masyr: optional_number(fields[5], "pmDE")?,
            bt,
            vt,
        }),
        "X" => Ok(SourceStar {
            tyc,
            kind: PositionKind::Observed,
            ra_deg: required_number(fields[24], "RAdeg")?,
            dec_deg: required_number(fields[25], "DEdeg")?,
            epoch_year: TYCHO2_OBSERVED_EPOCH_BASE + required_number(fields[26], "EpRA-1990")?,
            pm_ra_masyr: None,
            pm_dec_masyr: None,
            bt,
            vt,
        }),
        other => Err(format!("pflag {other:?} is not blank, P or X")),
    }
}

fn parse_supplement_line(line: &str) -> Result<SourceStar, String> {
    let fields = fields_of(line);
    if fields.len() < SUPPLEMENT_MIN_FIELDS {
        return Err(format!(
            "{} fields, suppl_1.dat has at least {SUPPLEMENT_MIN_FIELDS}",
            fields.len()
        ));
    }
    let flag = fields[1].trim();
    if !matches!(flag, "H" | "T") {
        return Err(format!("flag {flag:?} is not H or T"));
    }
    Ok(SourceStar {
        tyc: tyc_id(fields[0])?,
        kind: PositionKind::Supplement,
        ra_deg: required_number(fields[2], "RAdeg")?,
        dec_deg: required_number(fields[3], "DEdeg")?,
        epoch_year: SUPPLEMENT_EPOCH_YEAR,
        pm_ra_masyr: optional_number(fields[4], "pmRA")?,
        pm_dec_masyr: optional_number(fields[5], "pmDE")?,
        bt: optional_number(fields[11], "BTmag")?,
        vt: optional_number(fields[13], "VTmag")?,
    })
}

fn propagate_position(star: &SourceStar, epoch_year: f64) -> (f64, f64) {
    let (Some(pm_ra), Some(pm_dec)) = (star.pm_ra_masyr, star.pm_dec_masyr) else {
        return (star.ra_deg, star.dec_deg);
    };
    let years = epoch_year - star.epoch_year;
    let cos_dec = star.dec_deg.to_radians().cos();
    let d_ra = if cos_dec.abs() > 1e-12 {
        pm_ra * years / MAS_PER_DEGREE / cos_dec
    } else {
        0.0
    };
    (
        (star.ra_deg + d_ra).rem_euclid(360.0),
        (star.dec_deg + pm_dec * years / MAS_PER_DEGREE).clamp(-90.0, 90.0),
    )
}

fn counted_clamp(value: f64, low: f64, high: f64, count: &mut usize) -> f64 {
    if (low..=high).contains(&value) {
        value
    } else {
        *count += 1;
        value.clamp(low, high)
    }
}

fn tier0_star(star: &SourceStar, epoch_year: f64, clamps: &mut Clamps) -> CatalogStar {
    let (ra, dec) = propagate_position(star, epoch_year);
    let (low, high) = COMPACT_MAG_RANGE;
    let mag = star
        .vt
        .map(|vt| counted_clamp(vt, low, high, &mut clamps.magnitude));
    let colour = star.bt.zip(star.vt).map(|(bt, vt)| {
        counted_clamp(
            bt - vt,
            -COMPACT_COLOUR_LIMIT,
            COMPACT_COLOUR_LIMIT,
            &mut clamps.colour,
        )
    });
    CatalogStar {
        colour,
        ..CatalogStar::at(ra, dec, mag)
    }
}

fn twomass_star(source: TwoMassSource, clamps: &mut Clamps) -> CatalogStar {
    let (low, high) = COMPACT_MAG_RANGE;
    let mag = counted_clamp(source.j, low, high, &mut clamps.magnitude);
    let colour = source.k.map(|k| {
        counted_clamp(
            source.j - k,
            -COMPACT_COLOUR_LIMIT,
            COMPACT_COLOUR_LIMIT,
            &mut clamps.colour,
        )
    });
    CatalogStar {
        colour,
        ..CatalogStar::at(source.ra_deg, source.dec_deg, Some(mag))
    }
}

fn record_type_for(bound_arcsec: f64) -> CatType {
    if bound_arcsec <= (1.0 - COMPACT6_MARGIN) * COMPACT6_LIMIT_ARCSEC {
        CatType::Compact6
    } else {
        CatType::Compact12
    }
}

fn pixel_of(level: u8, ra_deg: f64, dec_deg: f64) -> u32 {
    cdshealpix::nested::hash(level, ra_deg.to_radians(), dec_deg.to_radians()) as u32
}

fn centre_deg(level: u8, pixel: u64) -> (f64, f64) {
    let (lon, lat) = cdshealpix::nested::center(level, pixel);
    (lon.to_degrees(), lat.to_degrees())
}

fn brighter_first(a: Option<f64>, b: Option<f64>) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn grouped_by_pixel(level: u8, stars: Vec<CatalogStar>) -> Vec<(u32, Vec<Record>)> {
    let mut keyed: Vec<(u32, CatalogStar)> = stars
        .into_iter()
        .map(|star| (pixel_of(level, star.ra_deg, star.dec_deg), star))
        .collect();
    keyed.sort_by(|(pixel_a, a), (pixel_b, b)| {
        pixel_a
            .cmp(pixel_b)
            .then_with(|| brighter_first(a.mag, b.mag))
            .then_with(|| a.dec_deg.total_cmp(&b.dec_deg))
            .then_with(|| a.ra_deg.total_cmp(&b.ra_deg))
    });
    let mut pixels: Vec<(u32, Vec<Record>)> = Vec::new();
    for (pixel, star) in keyed {
        match pixels.last_mut() {
            Some((last, records)) if *last == pixel => records.push(Record::Star(star)),
            _ => pixels.push((pixel, vec![Record::Star(star)])),
        }
    }
    pixels
}

fn max_centre_distance_arcsec(level: u8, pixels: &[(u32, Vec<Record>)]) -> f64 {
    pixels
        .iter()
        .flat_map(|(pixel, records)| {
            let (ra, dec) = centre_deg(level, u64::from(*pixel));
            records.iter().filter_map(move |record| match record {
                Record::Star(star) => {
                    Some(angular_separation(ra, dec, star.ra_deg, star.dec_deg) * ARCSEC_PER_DEGREE)
                }
                Record::Raw(_) => None,
            })
        })
        .fold(0.0, f64::max)
}

fn tier0_catalogue(
    stars: &[SourceStar],
    epoch_year: f64,
    level: u8,
    record: CatType,
) -> Result<(Vec<u8>, Tier0Stats)> {
    let mut clamps = Clamps::default();
    let compact: Vec<CatalogStar> = stars
        .iter()
        .map(|star| tier0_star(star, epoch_year, &mut clamps))
        .collect();
    let pixels = grouped_by_pixel(level, compact);
    let header =
        CatHeader::whole(TYCHO2_TITLE, level, record.code()).with_epoch_year(epoch_year as f32);
    let mut bytes = Vec::new();
    write_catalog(&header, &pixels, &mut bytes)?;
    let stats = Tier0Stats {
        stars: stars.len(),
        without_vt: stars.iter().filter(|star| star.vt.is_none()).count(),
        clamps,
        max_offset_arcsec: max_centre_distance_arcsec(level, &pixels),
    };
    Ok((bytes, stats))
}

fn parse_twomass_line(line: &str) -> Result<Option<TwoMassSource>, String> {
    let fields = fields_of(line);
    if fields.len() != TWOMASS_FIELDS {
        return Err(format!(
            "{} fields, the PSC layout has {TWOMASS_FIELDS}",
            fields.len()
        ));
    }
    let Some(j) = optional_number(fields[6], "j_m")? else {
        return Ok(None);
    };
    Ok(Some(TwoMassSource {
        ra_deg: required_number(fields[0], "ra")?,
        dec_deg: required_number(fields[1], "dec")?,
        j,
        k: optional_number(fields[14], "k_m")?,
    }))
}

impl TwoMassCut {
    fn new(level: u8, per_pixel: usize) -> Self {
        Self {
            level,
            per_pixel,
            pixels: HashMap::new(),
        }
    }

    fn push(&mut self, source: TwoMassSource) {
        let kept = self
            .pixels
            .entry(pixel_of(self.level, source.ra_deg, source.dec_deg))
            .or_default();
        if kept.len() < self.per_pixel {
            kept.push(source);
            return;
        }
        if let Some(faintest) = kept
            .iter_mut()
            .max_by(|a, b| a.j.total_cmp(&b.j))
            .filter(|faintest| source.j < faintest.j)
        {
            *faintest = source;
        }
    }

    fn counts(&self) -> HashMap<u32, usize> {
        self.pixels
            .iter()
            .map(|(pixel, kept)| (*pixel, kept.len()))
            .collect()
    }

    fn into_pixels(self, clamps: &mut Clamps) -> Vec<(u32, Vec<Record>)> {
        let stars: Vec<CatalogStar> = self
            .pixels
            .into_values()
            .flatten()
            .map(|source| twomass_star(source, clamps))
            .collect();
        grouped_by_pixel(self.level, stars)
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn file_sha256(path: &Path) -> Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let bytes = io::copy(
        &mut File::open(path).with_context(|| format!("cannot open {}", path.display()))?,
        &mut hasher,
    )?;
    Ok((bytes, hex(&hasher.finalize())))
}

fn read_text(path: &Path) -> Result<(InputFile, String)> {
    let raw = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let name = file_name(path);
    let bytes = raw.len() as u64;
    let sha256 = hex(&Sha256::digest(&raw));
    let text = if name.ends_with(".gz") {
        let mut text = String::new();
        MultiGzDecoder::new(raw.as_slice())
            .read_to_string(&mut text)
            .with_context(|| format!("cannot decompress {name}"))?;
        text
    } else {
        String::from_utf8(raw).with_context(|| format!("{name} is not text"))?
    };
    let lines = text.lines().filter(|line| !line.trim().is_empty()).count();
    Ok((
        InputFile {
            name,
            bytes,
            sha256,
            lines,
        },
        text,
    ))
}

fn tycho2_files(dir: &Path) -> Result<(Vec<PathBuf>, Option<PathBuf>)> {
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut supplement = None;
    let mut bases = HashSet::new();
    for entry in fs::read_dir(dir).with_context(|| format!("cannot list {}", dir.display()))? {
        let path = entry?.path();
        let name = file_name(&path);
        let base = name.strip_suffix(".gz").unwrap_or(&name).to_owned();
        let wanted = base.starts_with("tyc2.dat") || base == "suppl_1.dat";
        if wanted && !bases.insert(base.clone()) {
            bail!("{} holds {base} twice (plain and .gz)", dir.display());
        }
        if base.starts_with("tyc2.dat") {
            parts.push(path);
        } else if base == "suppl_1.dat" {
            supplement = Some(path);
        }
    }
    parts.sort();
    if parts.is_empty() {
        bail!("{} holds no tyc2.dat file", dir.display());
    }
    Ok((parts, supplement))
}

fn parse_lines(
    file: &InputFile,
    text: &str,
    parse: fn(&str) -> Result<SourceStar, String>,
    stars: &mut Vec<SourceStar>,
) -> Result<usize> {
    let mut count = 0;
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let star = parse(line).map_err(|error| anyhow!("{}:{}: {error}", file.name, number + 1))?;
        stars.push(star);
        count += 1;
    }
    Ok(count)
}

fn load_tycho2(dir: &Path) -> Result<Tycho2Input> {
    let (parts, supplement) = tycho2_files(dir)?;
    let mut stars = Vec::new();
    let mut files = Vec::new();
    let mut main = 0;
    for path in &parts {
        let (file, text) = read_text(path)?;
        main += parse_lines(&file, &text, parse_tycho2_line, &mut stars)?;
        files.push(file);
    }
    let mut supplement_count = 0;
    if let Some(path) = &supplement {
        let (file, text) = read_text(path)?;
        supplement_count = parse_lines(&file, &text, parse_supplement_line, &mut stars)?;
        files.push(file);
    }
    Ok(Tycho2Input {
        stars,
        main,
        supplement: supplement_count,
        files,
    })
}

fn twomass_paths(input: &Path) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(input)
        .with_context(|| format!("cannot list {}", input.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| file_name(path).starts_with("psc_"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        bail!("{} holds no psc_* file", input.display());
    }
    Ok(paths)
}

fn read_twomass(paths: &[PathBuf], per_pixel: usize) -> Result<TwoMassInput> {
    let mut input = TwoMassInput {
        cut: TwoMassCut::new(TIER1_LEVEL, per_pixel),
        files: Vec::new(),
        lines: 0,
        without_j: 0,
        rejected: 0,
        dec_range: (f64::INFINITY, f64::NEG_INFINITY),
    };
    for path in paths {
        let name = file_name(path);
        let (bytes, sha256) = file_sha256(path)?;
        let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let reader: Box<dyn BufRead> = if name.ends_with(".gz") {
            Box::new(BufReader::new(MultiGzDecoder::new(BufReader::new(file))))
        } else {
            Box::new(BufReader::new(file))
        };
        let mut lines = 0;
        for (number, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("cannot read {name}"))?;
            if line.trim().is_empty() {
                continue;
            }
            lines += 1;
            match parse_twomass_line(&line) {
                Ok(Some(source)) => {
                    input.dec_range.0 = input.dec_range.0.min(source.dec_deg);
                    input.dec_range.1 = input.dec_range.1.max(source.dec_deg);
                    input.cut.push(source);
                }
                Ok(None) => input.without_j += 1,
                Err(error) => {
                    if input.rejected == 0 {
                        eprintln!("first rejected line {name}:{}: {error}", number + 1);
                    }
                    input.rejected += 1;
                }
            }
        }
        input.lines += lines;
        input.files.push(InputFile {
            name,
            bytes,
            sha256,
            lines,
        });
    }
    Ok(input)
}

fn per_axis_arcsec(ra0_deg: f64, dec0_deg: f64, ra_deg: f64, dec_deg: f64) -> f64 {
    let (sin_dec0, cos_dec0) = dec0_deg.to_radians().sin_cos();
    let (sin_dec, cos_dec) = dec_deg.to_radians().sin_cos();
    let (sin_dra, cos_dra) = (ra_deg - ra0_deg).to_radians().sin_cos();
    let denominator = sin_dec0 * sin_dec + cos_dec0 * cos_dec * cos_dra;
    if denominator <= 0.0 {
        return f64::INFINITY;
    }
    let xi = cos_dec * sin_dra / denominator;
    let eta = (cos_dec0 * sin_dec - sin_dec0 * cos_dec * cos_dra) / denominator;
    xi.abs().max(eta.abs()).to_degrees() * ARCSEC_PER_DEGREE
}

fn cell_bound(level: u8) -> CellBound {
    let mut bound = CellBound {
        level,
        vertex_deg: 0.0,
        vertex_pixel: 0,
        edge_deg: 0.0,
        axis_arcsec: 0.0,
        axis_pixel: 0,
    };
    for pixel in 0..cdshealpix::nested::n_hash(level) {
        let (ra, dec) = centre_deg(level, pixel);
        let distance = |&(lon, lat): &(f64, f64)| {
            angular_separation(ra, dec, lon.to_degrees(), lat.to_degrees())
        };
        let axis =
            |&(lon, lat): &(f64, f64)| per_axis_arcsec(ra, dec, lon.to_degrees(), lat.to_degrees());
        let vertices = cdshealpix::nested::vertices(level, pixel);
        let edge = cdshealpix::nested::path_along_cell_edge(
            level,
            pixel,
            &Cardinal::S,
            true,
            EDGE_SEGMENTS,
        );
        let vertex = vertices.iter().map(distance).fold(0.0, f64::max);
        let along_edge = edge.iter().map(distance).fold(0.0, f64::max);
        let per_axis = vertices
            .iter()
            .chain(edge.iter())
            .map(axis)
            .fold(0.0, f64::max);
        if vertex > bound.vertex_deg {
            bound.vertex_deg = vertex;
            bound.vertex_pixel = pixel;
        }
        if per_axis > bound.axis_arcsec {
            bound.axis_arcsec = per_axis;
            bound.axis_pixel = pixel;
        }
        bound.edge_deg = bound.edge_deg.max(along_edge);
    }
    bound
}

impl CellBound {
    fn component_arcsec(&self) -> f64 {
        self.axis_arcsec
    }

    fn radial_arcsec(&self) -> f64 {
        self.vertex_deg
            .max(self.edge_deg)
            .to_radians()
            .tan()
            .to_degrees()
            * ARCSEC_PER_DEGREE
    }

    fn margin(&self) -> f64 {
        1.0 - self.component_arcsec() / COMPACT6_LIMIT_ARCSEC
    }
}

fn print_cell_bound(bound: &CellBound) {
    let (ra, dec) = centre_deg(bound.level, bound.vertex_pixel);
    let (axis_ra, axis_dec) = centre_deg(bound.level, bound.axis_pixel);
    println!(
        "level {} cells: {} pixels of {:.4} deg2",
        bound.level,
        cdshealpix::nested::n_hash(bound.level),
        SKY_DEG2 / cdshealpix::nested::n_hash(bound.level) as f64
    );
    println!(
        "largest centre-to-vertex distance {:.3} arcmin (pixel {} at RA {ra:.3} Dec {dec:.3}); largest centre-to-edge distance over {EDGE_SEGMENTS} samples per side {:.3} arcmin; radial tangent-plane bound {:.1} arcsec",
        bound.vertex_deg * ARCMIN_PER_DEGREE,
        bound.vertex_pixel,
        bound.edge_deg * ARCMIN_PER_DEGREE,
        bound.radial_arcsec()
    );
    println!(
        "largest per-axis standard coordinate max(|xi|, |eta|) {:.1} arcsec (pixel {} at RA {axis_ra:.3} Dec {axis_dec:.3}) against the int16 limit {COMPACT6_LIMIT_ARCSEC:.1} arcsec ({:.2} arcmin): margin {:.1} %, needed {:.0} %; record type {}",
        bound.component_arcsec(),
        bound.axis_pixel,
        COMPACT6_LIMIT_ARCSEC / 60.0,
        100.0 * bound.margin(),
        100.0 * COMPACT6_MARGIN,
        record_type_for(bound.component_arcsec()).name()
    );
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn field_for(stars: f64, density: f64) -> f64 {
    (FIELD_ASPECT * stars / density).sqrt()
}

fn compressed_sizes(bytes: &[u8]) -> Result<[(&'static str, usize); 3]> {
    let zstd = zstd::bulk::compress(bytes, ZSTD_LEVEL)?.len();
    let mut xz = XzEncoder::new(Vec::new(), XZ_LEVEL);
    xz.write_all(bytes)?;
    let xz = xz.finish()?.len();
    let mut gzip = GzEncoder::new(Vec::new(), Compression::best());
    gzip.write_all(bytes)?;
    let gzip = gzip.finish()?.len();
    Ok([("zstd -19", zstd), ("xz -9", xz), ("gzip -9", gzip)])
}

fn megabytes(bytes: usize) -> f64 {
    bytes as f64 / 1_000_000.0
}

struct LatitudeBin {
    pixels: usize,
    stars: u64,
    densities: Vec<f64>,
    below: [u64; MAG_LIMITS.len()],
}

fn measure_catalogue(path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let catalog = HealpixCatalog::open_file(path)?;
    let header = catalog.header().clone();
    if header.is_chunked() {
        bail!(
            "measure reads whole-sky files, {} is one chunk",
            path.display()
        );
    }
    let level = header.healpix_level;
    let cat_type = catalog.cat_type();
    let record_len = header
        .record_len()
        .ok_or_else(|| anyhow!("{} has no fixed record length", cat_type.name()))?;
    let pixel_count = cdshealpix::nested::n_hash(level) as usize;
    let mut counts = vec![0u32; pixel_count];
    let mut below = vec![[0u32; MAG_LIMITS.len()]; pixel_count];
    let (mut no_magnitude, mut edge_codes, mut undecodable) = (0usize, 0usize, 0usize);
    let (low, high) = COMPACT_MAG_RANGE;
    let whole_sky = 0..pixel_count as u32;
    catalog.visit_pixel_runs(std::slice::from_ref(&whole_sky), |pixel, records| {
        let slot = pixel as usize;
        for record in records.chunks_exact(record_len) {
            counts[slot] += 1;
            match decode_star(cat_type, level, pixel, record).map(|star| star.mag) {
                None => undecodable += 1,
                Some(None) => no_magnitude += 1,
                Some(Some(mag)) => {
                    if mag <= low + 1e-9 || mag >= high - 1e-9 {
                        edge_codes += 1;
                    }
                    for (limit, count) in MAG_LIMITS.iter().zip(below[slot].iter_mut()) {
                        if mag < *limit {
                            *count += 1;
                        }
                    }
                }
            }
        }
        Ok(())
    })?;

    let density_level = level.min(DENSITY_LEVEL);
    let shift = 2 * u32::from(level - density_level);
    let cell_count = cdshealpix::nested::n_hash(density_level) as usize;
    let cell_area = SKY_DEG2 / cell_count as f64;
    let mut cell_counts = vec![0u32; cell_count];
    let mut cell_below = vec![[0u32; MAG_LIMITS.len()]; cell_count];
    for (pixel, (&count, pixel_below)) in counts.iter().zip(&below).enumerate() {
        let cell = pixel >> shift;
        cell_counts[cell] += count;
        for (total, &value) in cell_below[cell].iter_mut().zip(pixel_below) {
            *total += value;
        }
    }
    let mut bins: Vec<LatitudeBin> = LATITUDE_BINS
        .iter()
        .map(|_| LatitudeBin {
            pixels: 0,
            stars: 0,
            densities: Vec::new(),
            below: [0; MAG_LIMITS.len()],
        })
        .collect();
    let mut caps: [(&str, usize, u64, f64); 4] = [
        ("north galactic pole b >= 80", 0, 0, f64::INFINITY),
        ("south galactic pole b <= -80", 0, 0, f64::INFINITY),
        ("north celestial pole Dec >= 80", 0, 0, f64::INFINITY),
        ("south celestial pole Dec <= -80", 0, 0, f64::INFINITY),
    ];
    for (cell, (&count, pixel_below)) in cell_counts.iter().zip(&cell_below).enumerate() {
        let (ra, dec) = centre_deg(density_level, cell as u64);
        let (_, b) = icrs_to_galactic(ra, dec);
        let density = f64::from(count) / cell_area;
        if let Some(bin) = LATITUDE_BINS
            .iter()
            .position(|&(from, to)| b.abs() >= from && (b.abs() < to || to >= 90.0))
            .and_then(|index| bins.get_mut(index))
        {
            bin.pixels += 1;
            bin.stars += u64::from(count);
            bin.densities.push(density);
            for (total, &value) in bin.below.iter_mut().zip(pixel_below) {
                *total += u64::from(value);
            }
        }
        let inside = [
            b >= POLE_CAP_DEG,
            b <= -POLE_CAP_DEG,
            dec >= POLE_CAP_DEG,
            dec <= -POLE_CAP_DEG,
        ];
        for (cap, _) in caps.iter_mut().zip(inside).filter(|(_, inside)| *inside) {
            cap.1 += 1;
            cap.2 += u64::from(count);
            cap.3 = cap.3.min(density);
        }
    }

    let mut sorted_counts: Vec<u32> = counts.clone();
    sorted_counts.sort_unstable();
    println!(
        "catalogue {}: title {:?}, level {level}, type {} ({}), epoch {}, {} records, {} bytes",
        path.display(),
        header.title,
        header.cat_type,
        cat_type.name(),
        header.epoch_year,
        catalog.total_records(),
        bytes.len()
    );
    println!(
        "records per pixel: empty pixels {}, min {}, median {}, p99 {}, max {}",
        counts.iter().filter(|&&count| count == 0).count(),
        sorted_counts.first().copied().unwrap_or(0),
        sorted_counts[sorted_counts.len() / 2],
        sorted_counts[(sorted_counts.len() - 1) * 99 / 100],
        sorted_counts.last().copied().unwrap_or(0)
    );
    println!(
        "magnitudes: {no_magnitude} records without magnitude (code 255), {edge_codes} records at the range edge codes (clamped), {undecodable} undecodable"
    );
    println!("density by galactic |b| on level-{density_level} cells of {cell_area:.4} deg2 (stars per deg2; L = long side of a 3:2 field reaching the star count at the p10 density):");
    println!(
        "{:>9} {:>7} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "|b| deg", "cells", "mean", "p10", "min", "max", "L30 deg", "L50 deg"
    );
    for (&(from, to), bin) in LATITUDE_BINS.iter().zip(bins.iter_mut()) {
        bin.densities.sort_by(f64::total_cmp);
        let mean = bin.stars as f64 / (bin.pixels as f64 * cell_area);
        let p10 = percentile(&bin.densities, DENSITY_PERCENTILE);
        println!(
            "{from:>4}-{to:<4} {:>7} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.2} {:>9.2}",
            bin.pixels,
            mean,
            p10,
            bin.densities.first().copied().unwrap_or(f64::NAN),
            bin.densities.last().copied().unwrap_or(f64::NAN),
            field_for(STAR_TARGETS[0], p10),
            field_for(STAR_TARGETS[1], p10)
        );
    }
    for (name, pixels, stars, min) in caps {
        println!(
            "{name}: {pixels} cells, mean {:.1} per deg2, min cell {min:.1} per deg2",
            stars as f64 / (pixels as f64 * cell_area)
        );
    }
    let mut heading = format!("{:>9}", "|b| deg");
    for limit in MAG_LIMITS {
        let _ = write!(heading, " {:>7}", format!("m<{limit}"));
    }
    println!("log10 N(<m) per deg2 by galactic |b| (m is the stored magnitude):");
    println!("{heading}");
    for (&(from, to), bin) in LATITUDE_BINS.iter().zip(&bins) {
        let mut row = format!("{from:>4}-{to:<4}");
        let area = bin.pixels as f64 * cell_area;
        for &count in &bin.below {
            let _ = write!(row, " {:>7.3}", (count as f64 / area).log10());
        }
        println!("{row}");
    }
    let (ra, dec, radius) = PLEIADES_CONE;
    let rows = catalog.cone(ra, dec, radius, None)?;
    println!(
        "cone at RA {ra} Dec {dec} radius {radius} deg: {} rows, brightest {:?}",
        rows.len(),
        rows.first().and_then(|row| row.g)
    );
    println!(
        "size: raw {} bytes ({:.2} MB)",
        bytes.len(),
        megabytes(bytes.len())
    );
    for (name, size) in compressed_sizes(&bytes)? {
        println!(
            "size: {name} {size} bytes ({:.2} MB, {:.1} % of raw)",
            megabytes(size),
            100.0 * size as f64 / bytes.len() as f64
        );
    }
    Ok(())
}

fn sidecar_path(path: &Path) -> PathBuf {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(".sha256");
    PathBuf::from(sidecar)
}

fn write_output(path: &Path, bytes: &[u8]) -> Result<String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
    let sha256 = hex(&Sha256::digest(bytes));
    fs::write(
        sidecar_path(path),
        format!("{sha256}  {}\n", file_name(path)),
    )?;
    Ok(sha256)
}

fn kind_count(stars: &[SourceStar], kind: PositionKind) -> usize {
    stars.iter().filter(|star| star.kind == kind).count()
}

fn duplicate_ids(stars: &[SourceStar]) -> usize {
    let mut seen = HashSet::with_capacity(stars.len());
    stars.iter().filter(|star| !seen.insert(star.tyc)).count()
}

fn tier0_provenance(
    out: &Path,
    sha256: &str,
    size: usize,
    header: &CatHeader,
    input: &Tycho2Input,
    stats: &Tier0Stats,
) -> String {
    let record = header.cat_type().map_or("unknown", CatType::name);
    let (level, epoch_year) = (header.healpix_level, header.epoch_year);
    let mut text = String::new();
    let _ = writeln!(text, "# Tier-0 catalogue provenance\n");
    let _ = writeln!(text, "- File: `{}`", file_name(out));
    let _ = writeln!(text, "- SHA-256: `{sha256}`");
    let _ = writeln!(text, "- Size: {size} bytes");
    let _ = writeln!(
        text,
        "- Container: Siril HEALPix catalogue format (zenodo record 14697486, CC BY 4.0), whole sky, HEALPix NESTED level {level}, record type {} ({record}), AstroBurst record kind {}, title `{}`, epoch {epoch_year:.1} in the header.",
        header.cat_type, header.record_kind, header.title
    );
    let _ = writeln!(
        text,
        "- Source: Tycho-2 Catalogue (Hog E. et al. 2000, A&A 355, L27), CDS catalogue I/259, https://cdsarc.cds.unistra.fr/ftp/I/259/"
    );
    let _ = writeln!(
        text,
        "- Licence: BSD-2-Clause as recorded in the Debian copyright file of the tycho2 package, https://tracker.debian.org/media/packages/t/tycho2/copyright-2-4"
    );
    let _ = writeln!(
        text,
        "- Acknowledgement: built from data obtained from the CDS, Strasbourg, France.\n"
    );
    let _ = writeln!(text, "## Inputs\n");
    let _ = writeln!(text, "| File | Bytes | Lines | SHA-256 |");
    let _ = writeln!(text, "|---|---|---|---|");
    for file in &input.files {
        let _ = writeln!(
            text,
            "| `{}` | {} | {} | `{}` |",
            file.name, file.bytes, file.lines, file.sha256
        );
    }
    let _ = writeln!(text, "\n## Processing\n");
    let _ = writeln!(
        text,
        "- tyc2.dat stars with a mean position (pflag blank or P): RAmdeg, DEmdeg (ICRS, epoch J2000.0) moved linearly with pmRA, pmDE to J{epoch_year:.1}: {} stars.",
        kind_count(&input.stars, PositionKind::Mean)
    );
    let _ = writeln!(
        text,
        "- tyc2.dat stars without a mean position (pflag X): observed RAdeg, DEdeg, no proper motion: {} stars.",
        kind_count(&input.stars, PositionKind::Observed)
    );
    let _ = writeln!(
        text,
        "- suppl_1.dat stars: RAdeg, DEdeg (ICRS, epoch J1991.25) moved with pmRA, pmDE when given: {} stars.",
        kind_count(&input.stars, PositionKind::Supplement)
    );
    let _ = writeln!(
        text,
        "- Magnitude: VTmag in 0.1 mag steps; {} stars without VTmag carry the no-magnitude code 255. Colour: BTmag - VTmag in 0.02 mag steps when both exist.",
        stats.without_vt
    );
    let _ = writeln!(
        text,
        "- Clamped to the record range: {} magnitudes, {} colours. Largest star offset from its pixel centre: {:.1} arcsec.",
        stats.clamps.magnitude, stats.clamps.colour, stats.max_offset_arcsec
    );
    let _ = writeln!(
        text,
        "- Records inside each pixel are ordered brightest first (no-magnitude records last), then by Dec and RA, so the build is reproducible."
    );
    let _ = writeln!(
        text,
        "- Builder: `cargo run --release --features tools --bin build_catalog -- tycho2` (AstroBurst `src-tauri/src/bin/build_catalog.rs`)."
    );
    text
}

fn run_tycho2(options: &Options) -> Result<()> {
    options.only(&["--input", "--out", "--epoch", "--level", "--record"])?;
    let input_dir = options.path("--input")?;
    let out = options.path("--out")?;
    let epoch_year: f64 = options.parsed("--epoch", TIER0_EPOCH_YEAR)?;
    let level: u8 = options.parsed("--level", TIER0_LEVEL)?;
    let started = Instant::now();
    let bound = cell_bound(level);
    print_cell_bound(&bound);
    let record = match options.text("--record").unwrap_or("auto") {
        "auto" => record_type_for(bound.component_arcsec()),
        "6" => CatType::Compact6,
        "12" => CatType::Compact12,
        other => bail!("--record {other:?} is not auto, 6 or 12"),
    };
    let input = load_tycho2(&input_dir)?;
    let parts = input
        .files
        .iter()
        .filter(|file| file.name.starts_with("tyc2.dat"))
        .count();
    if parts != TYCHO2_PARTS {
        println!("note: {parts} tyc2.dat parts read, the full catalogue has {TYCHO2_PARTS}");
    }
    let (bytes, stats) = tier0_catalogue(&input.stars, epoch_year, level, record)?;
    let sha256 = write_output(&out, &bytes)?;
    let header = bytes
        .get(..HEADER_LEN)
        .and_then(|head| <&[u8; HEADER_LEN]>::try_from(head).ok())
        .map(CatHeader::parse)
        .ok_or_else(|| anyhow!("the written catalogue has no header"))?;
    let provenance = tier0_provenance(&out, &sha256, bytes.len(), &header, &input, &stats);
    let provenance_path = out
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("PROVENANCE.md");
    fs::write(&provenance_path, provenance)?;
    println!(
        "read {} main and {} supplement stars from {} files; {} duplicate TYC ids",
        input.main,
        input.supplement,
        input.files.len(),
        duplicate_ids(&input.stars)
    );
    println!(
        "wrote {} records of type {} to {} ({} bytes, sha-256 {sha256}) and {}",
        stats.stars,
        record.code(),
        out.display(),
        bytes.len(),
        provenance_path.display()
    );
    println!(
        "{} stars without VT; clamped {} magnitudes and {} colours; largest offset from a pixel centre {:.1} arcsec ({:.2} arcmin); {:.1} s",
        stats.without_vt,
        stats.clamps.magnitude,
        stats.clamps.colour,
        stats.max_offset_arcsec,
        stats.max_offset_arcsec / 60.0,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn run_twomass(options: &Options) -> Result<()> {
    options.only(&["--input", "--out", "--per-pixel"])?;
    let paths = twomass_paths(&options.path("--input")?)?;
    let out = options.path("--out")?;
    let per_pixel: usize = options.parsed("--per-pixel", TWOMASS_PER_PIXEL)?;
    let started = Instant::now();
    let bound = cell_bound(TIER1_LEVEL);
    print_cell_bound(&bound);
    let record = record_type_for(bound.component_arcsec());
    let input = read_twomass(&paths, per_pixel)?;
    let counts = input.cut.counts();
    let mut clamps = Clamps::default();
    let pixels = input.cut.into_pixels(&mut clamps);
    let header = CatHeader::whole(TWOMASS_TITLE, TIER1_LEVEL, record.code())
        .with_epoch_year(TWOMASS_EPOCH_YEAR);
    let mut bytes = Vec::new();
    write_catalog(&header, &pixels, &mut bytes)?;
    let sha256 = write_output(&out, &bytes)?;
    let records: usize = pixels.iter().map(|(_, records)| records.len()).sum();
    let record_len = record.record_len().unwrap_or(0);
    let pixel_count = cdshealpix::nested::n_hash(TIER1_LEVEL);
    let pixel_area = SKY_DEG2 / pixel_count as f64;
    let (dec_low, dec_high) = input.dec_range;
    let mut interior: Vec<f64> = (0..pixel_count)
        .filter(|&pixel| {
            let (_, dec) = centre_deg(TIER1_LEVEL, pixel);
            dec > dec_low + INTERIOR_MARGIN_DEG && dec < dec_high - INTERIOR_MARGIN_DEG
        })
        .map(|pixel| counts.get(&(pixel as u32)).copied().unwrap_or(0) as f64)
        .collect();
    interior.sort_by(f64::total_cmp);
    let interior_mean = interior.iter().sum::<f64>() / interior.len().max(1) as f64;
    let p10_density = percentile(&interior, DENSITY_PERCENTILE) / pixel_area;
    let projected_records = interior_mean * pixel_count as f64;
    let projected_bytes =
        header.index_bytes() as f64 + HEADER_LEN as f64 + projected_records * record_len as f64;
    for file in &input.files {
        println!(
            "input {}: {} bytes, {} lines, sha-256 {}",
            file.name, file.bytes, file.lines, file.sha256
        );
    }
    println!(
        "{} lines, {} without J, {} rejected; Dec range {dec_low:.3} to {dec_high:.3}",
        input.lines, input.without_j, input.rejected
    );
    println!(
        "wrote {records} records of type {} in {} non-empty level-{TIER1_LEVEL} pixels to {} ({} bytes, sha-256 {sha256}); {} pixels at the cap of {per_pixel}; clamped {} magnitudes and {} colours",
        record.code(),
        pixels.len(),
        out.display(),
        bytes.len(),
        pixels.iter().filter(|(_, records)| records.len() >= per_pixel).count(),
        clamps.magnitude,
        clamps.colour
    );
    println!(
        "interior pixels (centre Dec inside the sample band by {INTERIOR_MARGIN_DEG} deg): {}, mean {interior_mean:.2} records, p10 {:.1} records ({p10_density:.1} per deg2)",
        interior.len(),
        percentile(&interior, DENSITY_PERCENTILE)
    );
    println!(
        "projected all-sky cut at this band's mean: {projected_records:.0} records, {projected_bytes:.0} bytes ({:.1} MB) at {record_len} B per record",
        projected_bytes / 1e6
    );
    println!(
        "smallest 3:2 field (long side) with {} and {} stars at the band's p10 density: {:.3} and {:.3} deg; {:.1} s",
        STAR_TARGETS[0],
        STAR_TARGETS[1],
        field_for(STAR_TARGETS[0], p10_density),
        field_for(STAR_TARGETS[1], p10_density),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn run_measure(options: &Options) -> Result<()> {
    options.only(&["--catalog", "--level"])?;
    let catalog = options.text("--catalog").map(PathBuf::from);
    let level = match (&catalog, options.text("--level")) {
        (_, Some(_)) => options.parsed("--level", TIER0_LEVEL)?,
        (Some(path), None) => HealpixCatalog::open_file(path)?.header().healpix_level,
        (None, None) => TIER0_LEVEL,
    };
    print_cell_bound(&cell_bound(level));
    match catalog {
        Some(path) => measure_catalogue(&path),
        None => Ok(()),
    }
}

fn run(args: &[String]) -> Result<()> {
    let Some((command, rest)) = args.split_first() else {
        bail!("{USAGE}");
    };
    let options = Options::parse(rest)?;
    match command.as_str() {
        "tycho2" => run_tycho2(&options),
        "twomass" => run_twomass(&options),
        "measure" => run_measure(&options),
        other => bail!("unknown subcommand {other:?}\n{USAGE}"),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("build_catalog: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use astroburst_lib::core::astrometry::catalog::{
        propagate_epoch, CatalogRow, GAIA_REFERENCE_EPOCH_YEAR,
    };
    use astroburst_lib::core::astrometry::healpix_cat::{
        encode_record, MemSource, COMPACT_NO_MAGNITUDE,
    };
    use astroburst_lib::core::astrometry::wcs::angular_separation;

    const ENV_TYCHO2_DIR: &str = "ASTROBURST_TYCHO2_DIR";
    const ENV_2MASS_SAMPLE: &str = "ASTROBURST_2MASS_SAMPLE";
    const FIRST_TYC2_LINE: &str = "0001 00008 1| |  2.31750494|  2.23184345|  -16.3|   -9.0| 68| 73| 1.7| 1.8|1958.89|1951.94| 4|1.0|1.0|0.9|1.0|12.146|0.158|12.146|0.223|999| |         |  2.31754222|  2.23186444|1.67|1.54| 88.0|100.8| |-0.2";
    const TYCHO2_WIDTHS: [usize; 32] = [
        12, 1, 12, 12, 7, 7, 3, 3, 4, 4, 7, 7, 2, 3, 3, 3, 3, 6, 5, 6, 5, 3, 1, 9, 12, 12, 4, 4, 5,
        5, 1, 4,
    ];
    const SUPPLEMENT_WIDTHS: [usize; 18] =
        [12, 1, 12, 12, 7, 7, 5, 5, 5, 5, 1, 6, 5, 6, 5, 3, 1, 7];
    const ARCSEC: f64 = 1.0 / 3600.0;
    const PLEIADES_MEMBERS: [(&str, f64, f64); 7] = [
        ("Alcyone", 56.87115, 24.10514),
        ("Atlas", 57.29059, 24.05342),
        ("Electra", 56.21890, 24.11334),
        ("Maia", 56.45670, 24.36775),
        ("Merope", 56.58156, 23.94836),
        ("Taygeta", 56.30207, 24.46728),
        ("Pleione", 57.29673, 24.13671),
    ];

    fn fixed_width(fields: &[&str], widths: &[usize]) -> String {
        assert_eq!(fields.len(), widths.len());
        fields
            .iter()
            .zip(widths)
            .map(|(field, &width)| format!("{field:>width$}"))
            .collect::<Vec<_>>()
            .join("|")
    }

    fn observed_only_line() -> String {
        fixed_width(
            &[
                "0001 00017 1",
                "X",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "12.952",
                "0.210",
                "11.873",
                "0.150",
                "999",
                "",
                "",
                "3.12345678",
                "-1.98765432",
                "1.73",
                "1.65",
                "120.0",
                "130.0",
                "",
                "-0.1",
            ],
            &TYCHO2_WIDTHS,
        )
    }

    fn without_vt_line() -> String {
        fixed_width(
            &[
                "0002 00034 1",
                "",
                "10.00000000",
                "-10.00000000",
                "12.0",
                "-5.0",
                "50",
                "60",
                "1.5",
                "1.6",
                "1950.00",
                "1951.00",
                "3",
                "1.1",
                "1.2",
                "1.3",
                "1.4",
                "11.800",
                "0.120",
                "",
                "",
                "999",
                "",
                "",
                "10.00000300",
                "-10.00000100",
                "1.50",
                "1.60",
                "70.0",
                "80.0",
                "",
                "0.1",
            ],
            &TYCHO2_WIDTHS,
        )
    }

    fn supplement_line() -> String {
        fixed_width(
            &[
                "0008 01089 1",
                "H",
                "101.28715533",
                "-16.71611586",
                "-546.0",
                "-1223.1",
                "0.7",
                "0.6",
                "0.6",
                "0.5",
                "",
                "-0.952",
                "0.007",
                "-1.088",
                "0.007",
                "999",
                "T",
                " 32349 ",
            ],
            &SUPPLEMENT_WIDTHS,
        )
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9
    }

    fn skip_unless_set(name: &str) -> Option<PathBuf> {
        let value = std::env::var_os(name).map(PathBuf::from);
        if value.is_none() {
            eprintln!("skipped: {name} is not set");
        }
        value
    }

    #[test]
    fn tycho2_line_parses_the_readme_columns() {
        assert_eq!(FIRST_TYC2_LINE.len(), 206);
        let first = parse_tycho2_line(FIRST_TYC2_LINE).unwrap();
        assert_eq!(first.tyc, [1, 8, 1]);
        assert_eq!(first.kind, PositionKind::Mean);
        assert!(near(first.ra_deg, 2.31750494) && near(first.dec_deg, 2.23184345));
        assert!(near(first.epoch_year, 2000.0));
        assert_eq!(first.pm_ra_masyr, Some(-16.3));
        assert_eq!(first.pm_dec_masyr, Some(-9.0));
        assert_eq!((first.bt, first.vt), (Some(12.146), Some(12.146)));

        let observed = observed_only_line();
        assert_eq!(observed.len(), 206);
        let observed = parse_tycho2_line(&observed).unwrap();
        assert_eq!(observed.tyc, [1, 17, 1]);
        assert_eq!(observed.kind, PositionKind::Observed);
        assert!(near(observed.ra_deg, 3.12345678) && near(observed.dec_deg, -1.98765432));
        assert!(near(observed.epoch_year, 1991.73));
        assert_eq!((observed.pm_ra_masyr, observed.pm_dec_masyr), (None, None));
        assert_eq!((observed.bt, observed.vt), (Some(12.952), Some(11.873)));

        let supplement = supplement_line();
        assert_eq!(supplement.len(), 122);
        let supplement = parse_supplement_line(&supplement).unwrap();
        assert_eq!(supplement.tyc, [8, 1089, 1]);
        assert_eq!(supplement.kind, PositionKind::Supplement);
        assert!(near(supplement.ra_deg, 101.28715533) && near(supplement.dec_deg, -16.71611586));
        assert!(near(supplement.epoch_year, 1991.25));
        assert_eq!(supplement.pm_ra_masyr, Some(-546.0));
        assert_eq!(supplement.pm_dec_masyr, Some(-1223.1));
        assert_eq!((supplement.bt, supplement.vt), (Some(-0.952), Some(-1.088)));

        let short = FIRST_TYC2_LINE.rsplit_once('|').unwrap().0;
        let refusal = parse_tycho2_line(short).unwrap_err();
        assert!(refusal.contains("31 fields"), "{refusal}");
        let bad_flag = FIRST_TYC2_LINE.replacen("1| |", "1|Q|", 1);
        let refusal = parse_tycho2_line(&bad_flag).unwrap_err();
        assert!(refusal.contains("pflag"), "{refusal}");
    }

    #[test]
    fn pm_propagation_to_2026_matches_propagate_epoch() {
        let first = parse_tycho2_line(FIRST_TYC2_LINE).unwrap();
        let polar = SourceStar {
            dec_deg: 89.9,
            pm_ra_masyr: Some(250.0),
            ..first.clone()
        };
        let wrapping = SourceStar {
            ra_deg: 359.99999,
            pm_ra_masyr: Some(900.0),
            ..first.clone()
        };
        let years = TIER0_EPOCH_YEAR - 2000.0;
        for star in [&first, &polar, &wrapping] {
            let mut rows = vec![CatalogRow {
                id: "t".to_owned(),
                ra: star.ra_deg,
                dec: star.dec_deg,
                ra_epoch: star.ra_deg,
                dec_epoch: star.dec_deg,
                pm_ra_masyr: star.pm_ra_masyr,
                pm_dec_masyr: star.pm_dec_masyr,
                g: None,
                bp: None,
                rp: None,
                bp_rp: None,
                parallax_mas: None,
                teff_k: None,
            }];
            propagate_epoch(&mut rows, GAIA_REFERENCE_EPOCH_YEAR + years);
            let (ra, dec) = propagate_position(star, TIER0_EPOCH_YEAR);
            assert!(
                (ra - rows[0].ra_epoch).abs() <= 1e-12 && (dec - rows[0].dec_epoch).abs() <= 1e-12,
                "{star:?}: ({ra}, {dec}) against ({}, {})",
                rows[0].ra_epoch,
                rows[0].dec_epoch
            );
        }
        let (ra, _) = propagate_position(&wrapping, TIER0_EPOCH_YEAR);
        assert!(ra < 1.0, "RA wraps through 0: {ra}");
        let observed = parse_tycho2_line(&observed_only_line()).unwrap();
        assert_eq!(
            propagate_position(&observed, TIER0_EPOCH_YEAR),
            (observed.ra_deg, observed.dec_deg)
        );
    }

    #[test]
    fn a_star_without_vt_gets_the_no_magnitude_code() {
        let star = parse_tycho2_line(&without_vt_line()).unwrap();
        assert_eq!((star.bt, star.vt), (Some(11.8), None));
        let mut clamps = Clamps::default();
        let compact = tier0_star(&star, TIER0_EPOCH_YEAR, &mut clamps);
        assert_eq!((compact.mag, compact.colour), (None, None));
        let pixel = cdshealpix::nested::hash(
            TIER0_LEVEL,
            compact.ra_deg.to_radians(),
            compact.dec_deg.to_radians(),
        ) as u32;
        let mut record = Vec::new();
        encode_record(
            CatType::Compact6,
            TIER0_LEVEL,
            pixel,
            &Record::Star(compact),
            &mut record,
        )
        .unwrap();
        assert_eq!(record[4], COMPACT_NO_MAGNITUDE);
        assert_eq!(record[5] as i8, i8::MIN);
        assert_eq!(clamps, Clamps::default());
    }

    #[test]
    fn the_level_6_cell_bound_is_the_largest_per_axis_offset_and_allows_six_byte_records() {
        let bound = cell_bound(TIER0_LEVEL);
        let per_axis = bound.component_arcsec();
        assert!(
            (per_axis - 2886.1).abs() < 2.0,
            "per-axis bound {per_axis:.1} arcsec, expected about 2886.1 (the radial bound is 3435.3)"
        );
        assert!(
            bound.margin() >= COMPACT6_MARGIN,
            "margin {:.3}",
            bound.margin()
        );
        assert_eq!(record_type_for(per_axis), CatType::Compact6);
    }

    #[test]
    fn six_byte_records_need_a_ten_percent_margin_on_the_int16_range() {
        let allowed = (1.0 - COMPACT6_MARGIN) * COMPACT6_LIMIT_ARCSEC;
        assert!((COMPACT6_LIMIT_ARCSEC / 60.0 - 54.61).abs() < 0.01);
        assert_eq!(record_type_for(allowed - 0.1), CatType::Compact6);
        assert_eq!(record_type_for(allowed + 0.1), CatType::Compact12);
        assert_eq!(record_type_for(f64::NAN), CatType::Compact12);
    }

    #[test]
    fn a_built_tier0_reopens_and_a_cone_returns_its_stars_within_the_record_quantum() {
        let stars = vec![
            parse_tycho2_line(FIRST_TYC2_LINE).unwrap(),
            parse_tycho2_line(&observed_only_line()).unwrap(),
            parse_tycho2_line(&without_vt_line()).unwrap(),
            parse_supplement_line(&supplement_line()).unwrap(),
        ];
        let (bytes, stats) =
            tier0_catalogue(&stars, TIER0_EPOCH_YEAR, TIER0_LEVEL, CatType::Compact6).unwrap();
        assert_eq!(stats.stars, 4);
        assert_eq!(stats.without_vt, 1);
        assert_eq!(stats.clamps, Clamps::default());
        assert!(stats.max_offset_arcsec > 0.0 && stats.max_offset_arcsec < COMPACT6_LIMIT_ARCSEC);
        let catalog = HealpixCatalog::new(MemSource::new(bytes)).unwrap();
        let header = catalog.header();
        assert_eq!(header.title, TYCHO2_TITLE);
        assert_eq!(header.healpix_level, TIER0_LEVEL);
        assert_eq!(header.cat_type, CatType::Compact6.code());
        assert_eq!(header.epoch_year, TIER0_EPOCH_YEAR as f32);
        assert_eq!(catalog.total_records(), 4);
        for star in &stars {
            let (ra, dec) = propagate_position(star, TIER0_EPOCH_YEAR);
            let rows = catalog.cone(ra, dec, 10.0 * ARCSEC, None).unwrap();
            assert_eq!(rows.len(), 1, "{star:?}");
            let separation = angular_separation(ra, dec, rows[0].ra, rows[0].dec) * 3600.0;
            assert!(separation <= 0.08, "{star:?}: {separation} arcsec");
            let expected = star.vt.map(|vt| (vt * 10.0).round() / 10.0);
            match (rows[0].g, expected) {
                (Some(mag), Some(vt)) => assert!((mag - vt).abs() < 1e-9, "{mag} {vt}"),
                (found, wanted) => assert_eq!(found, wanted),
            }
        }
    }

    #[test]
    fn twomass_psc_line_gives_ra_dec_j_and_j_minus_k() {
        let mut fields = vec!["\\N"; 60];
        fields[0] = "10.684737";
        fields[1] = "41.269065";
        fields[5] = "00424433+4116087";
        fields[6] = "12.345";
        fields[14] = "11.111";
        let source = parse_twomass_line(&fields.join("|")).unwrap().unwrap();
        assert_eq!(
            source,
            TwoMassSource {
                ra_deg: 10.684737,
                dec_deg: 41.269065,
                j: 12.345,
                k: Some(11.111),
            }
        );
        fields[14] = "\\N";
        assert_eq!(
            parse_twomass_line(&fields.join("|")).unwrap().unwrap().k,
            None
        );
        fields[6] = "\\N";
        assert_eq!(parse_twomass_line(&fields.join("|")).unwrap(), None);
        let refusal = parse_twomass_line(&fields[..59].join("|")).unwrap_err();
        assert!(refusal.contains("59 fields"), "{refusal}");
    }

    #[test]
    fn the_twomass_cut_keeps_the_brightest_n_per_level_8_pixel() {
        let (ra, dec) = (120.0, -30.0);
        let mut cut = TwoMassCut::new(TIER1_LEVEL, 3);
        for (n, j) in [14.0, 10.0, 12.0, 11.0, 13.0].into_iter().enumerate() {
            cut.push(TwoMassSource {
                ra_deg: ra + n as f64 * 1e-4,
                dec_deg: dec,
                j,
                k: Some(j - 0.5),
            });
        }
        cut.push(TwoMassSource {
            ra_deg: ra + 10.0,
            dec_deg: dec,
            j: 15.0,
            k: None,
        });
        let mut clamps = Clamps::default();
        let pixels = cut.into_pixels(&mut clamps);
        assert_eq!(pixels.len(), 2);
        let crowded = pixels
            .iter()
            .find(|(_, records)| records.len() == 3)
            .unwrap();
        let mags: Vec<Option<f64>> = crowded
            .1
            .iter()
            .map(|record| match record {
                Record::Star(star) => star.mag,
                Record::Raw(_) => None,
            })
            .collect();
        assert_eq!(mags, vec![Some(10.0), Some(11.0), Some(12.0)]);
        let colours: Vec<Option<f64>> = crowded
            .1
            .iter()
            .map(|record| match record {
                Record::Star(star) => star.colour,
                Record::Raw(_) => None,
            })
            .collect();
        assert_eq!(colours, vec![Some(0.5); 3]);
        assert!(pixels.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    #[ignore]
    fn tier0_file_reopens_through_healpix_cat_and_a_cone_at_m45_returns_the_pleiades_members() {
        let Some(dir) = skip_unless_set(ENV_TYCHO2_DIR) else {
            return;
        };
        let input = load_tycho2(&dir).unwrap();
        assert_eq!(input.main, 2_539_913);
        assert_eq!(input.supplement, 17_588);
        let record = record_type_for(cell_bound(TIER0_LEVEL).component_arcsec());
        let (bytes, stats) =
            tier0_catalogue(&input.stars, TIER0_EPOCH_YEAR, TIER0_LEVEL, record).unwrap();
        assert_eq!(stats.clamps.magnitude, 0);
        let catalog = HealpixCatalog::new(MemSource::new(bytes)).unwrap();
        let rows = catalog.cone(56.75, 24.12, 1.0, None).unwrap();
        assert!(rows.len() >= 200, "{} rows", rows.len());
        for (name, ra, dec) in PLEIADES_MEMBERS {
            let found = rows.iter().find(|row| {
                angular_separation(ra, dec, row.ra, row.dec) <= 5.0 * ARCSEC
                    && row.g.is_some_and(|mag| mag < 6.0)
            });
            assert!(found.is_some(), "{name} at ({ra}, {dec})");
        }
    }

    #[test]
    #[ignore]
    fn the_2mass_sample_parses_every_line_and_fills_level_8_pixels() {
        let Some(sample) = skip_unless_set(ENV_2MASS_SAMPLE) else {
            return;
        };
        let input = read_twomass(&[sample], 32).unwrap();
        assert_eq!(input.rejected, 0);
        let mut clamps = Clamps::default();
        let pixels = input.cut.into_pixels(&mut clamps);
        assert!(pixels.len() >= 1000, "{} pixels", pixels.len());
        assert!(pixels.iter().all(|(_, records)| records.len() <= 32));
        assert_eq!(clamps.magnitude, 0);
    }
}
