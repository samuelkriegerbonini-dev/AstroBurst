use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::*;
use crate::core::astrometry::gwcs::test_support::{env_dir, skip_if_absent};

const ARCSEC_PER_DEGREE: f64 = 3600.0;
const SIRIL_QUANTUM_DEG: f64 = 360.0 / 2_147_483_647.0;

fn written(header: &CatHeader, pixels: &[(u32, Vec<Record>)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_catalog(header, pixels, &mut bytes)
        .unwrap_or_else(|e| panic!("the test writer accepts its input: {e}"));
    bytes
}

fn mem_catalog(bytes: Vec<u8>) -> HealpixCatalog<MemSource> {
    HealpixCatalog::new(MemSource::new(bytes))
        .unwrap_or_else(|e| panic!("the written catalogue opens: {e}"))
}

fn refusal<T>(result: Result<T, CatalogReadError>) -> String {
    match result {
        Ok(_) => panic!("expected a refusal"),
        Err(error) => error.to_string(),
    }
}

fn pixel_of(level: u8, ra_deg: f64, dec_deg: f64) -> u32 {
    cdshealpix::nested::hash(level, ra_deg.to_radians(), dec_deg.to_radians()) as u32
}

fn centre_deg(level: u8, pixel: u32) -> (f64, f64) {
    let (lon, lat) = cdshealpix::nested::center(level, u64::from(pixel));
    (lon.to_degrees(), lat.to_degrees())
}

fn star_at(ra_deg: f64, dec_deg: f64, mag: f64) -> Record {
    Record::Star(CatalogStar::at(ra_deg, dec_deg, Some(mag)))
}

fn stars_in(level: u8, pixel: u32, count: usize, seed: u64) -> Vec<Record> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|i| {
            let (lon, lat) =
                cdshealpix::nested::sph_coo(level, u64::from(pixel), rng.gen(), rng.gen());
            star_at(
                lon.to_degrees(),
                lat.to_degrees(),
                8.0 + (i % 1000) as f64 * 0.01,
            )
        })
        .collect()
}

fn pattern_around(
    ra0_deg: f64,
    dec0_deg: f64,
    count: usize,
    half_width_deg: f64,
    seed: u64,
) -> Vec<(f64, f64)> {
    let mut rng = StdRng::seed_from_u64(seed);
    let half_width = half_width_deg.to_radians();
    (0..count)
        .map(|_| {
            let xi = (rng.gen::<f64>() * 2.0 - 1.0) * half_width;
            let eta = (rng.gen::<f64>() * 2.0 - 1.0) * half_width;
            let (ra, dec) = from_standard(ra0_deg.to_radians(), dec0_deg.to_radians(), xi, eta);
            (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
        })
        .collect()
}

fn grouped(level: u8, stars: &[(f64, f64)]) -> Vec<(u32, Vec<Record>)> {
    let mut by_pixel: BTreeMap<u32, Vec<Record>> = BTreeMap::new();
    for (i, &(ra, dec)) in stars.iter().enumerate() {
        by_pixel
            .entry(pixel_of(level, ra, dec))
            .or_default()
            .push(star_at(ra, dec, 10.0 + i as f64 * 0.001));
    }
    by_pixel.into_iter().collect()
}

fn expected_ids(
    pixels: &[(u32, Vec<Record>)],
    ra0: f64,
    dec0: f64,
    radius: f64,
) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for (pixel, records) in pixels {
        for (n, record) in records.iter().enumerate() {
            if let Record::Star(star) = record {
                if angular_separation(ra0, dec0, star.ra_deg, star.dec_deg) <= radius {
                    ids.insert(format!("G{pixel}-{n}"));
                }
            }
        }
    }
    ids
}

fn ids_of(rows: &[CatalogRow]) -> BTreeSet<String> {
    rows.iter().map(|row| row.id.clone()).collect()
}

fn round_trip(cat_type: CatType, level: u8, pixel: u32, star: CatalogStar) -> CatalogStar {
    let mut bytes = Vec::new();
    encode_record(cat_type, level, pixel, &Record::Star(star), &mut bytes)
        .unwrap_or_else(|e| panic!("{star:?} encodes: {e}"));
    assert_eq!(Some(bytes.len()), cat_type.record_len());
    decode_star(cat_type, level, pixel, &bytes).unwrap_or_else(|| panic!("{cat_type:?} decodes"))
}

fn ra_difference_deg(a: f64, b: f64) -> f64 {
    ((a - b + 540.0).rem_euclid(360.0) - 180.0).abs()
}

fn quantised(deg: f64) -> f64 {
    decode_siril_coordinate(encode_siril_coordinate(deg).unwrap_or_default())
}

fn whole_sky_stars(count: usize, seed: u64) -> Vec<(f64, f64)> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|_| {
            let ra = rng.gen::<f64>() * 360.0;
            let dec = (rng.gen::<f64>() * 2.0 - 1.0).asin().to_degrees();
            (quantised(ra), quantised(dec))
        })
        .collect()
}

fn siril_like_extract() -> Vec<u8> {
    let level = 8;
    let mut rng = StdRng::seed_from_u64(31);
    let mut by_pixel: BTreeMap<u32, Vec<Record>> = BTreeMap::new();
    for (ra0, dec0, count, half_width) in [(0.0, 0.0, 300, 0.3), (83.82, -5.39, 600, 0.6)] {
        for (ra, dec) in pattern_around(ra0, dec0, count, half_width, rng.gen()) {
            let (ra, dec) = (quantised(ra), quantised(dec));
            let star = CatalogStar {
                pm_ra_masyr: Some(rng.gen_range(-20.0f64..20.0).round()),
                pm_dec_masyr: Some(rng.gen_range(-20.0f64..20.0).round()),
                teff_k: Some(rng.gen_range(3500.0f64..7500.0).round()),
                ..CatalogStar::at(ra, dec, Some(rng.gen_range(12.0..18.0)))
            };
            by_pixel
                .entry(pixel_of(level, ra, dec))
                .or_default()
                .push(Record::Star(star));
        }
    }
    let pixels: Vec<(u32, Vec<Record>)> = by_pixel.into_iter().collect();
    written(
        &CatHeader::whole("Siril-like astrometric extract", level, 1),
        &pixels,
    )
}

const ORIGIN_CONE: (f64, f64, f64) = (0.0, 0.0, 0.2);
const ORION_CONE: (f64, f64, f64) = (83.82, -5.39, 0.5);

fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(sorted.len() / 2).copied()
}

fn check_distribution(
    problems: &mut Vec<String>,
    at: &str,
    what: &str,
    values: &[f64],
    plausible: RangeInclusive<f64>,
    typical_median: RangeInclusive<f64>,
) {
    let implausible = values
        .iter()
        .filter(|value| !plausible.contains(value))
        .count();
    if implausible * 1000 > values.len() {
        problems.push(format!(
            "{at}: {implausible} of {} {what} values lie outside {plausible:?}",
            values.len()
        ));
    }
    match median(values) {
        Some(median) if typical_median.contains(&median) => {}
        median => problems.push(format!(
            "{at}: median {what} {median:?} is outside {typical_median:?}"
        )),
    }
}

fn records_outside_their_pixel<S: ByteSource>(
    catalog: &HealpixCatalog<S>,
    (ra0, dec0, radius): (f64, f64, f64),
) -> Result<(usize, usize), CatalogReadError> {
    let header = catalog.header();
    let level = header.healpix_level;
    let runs: Vec<Range<u32>> =
        pixel_ranges(level, header.chunk_level_of_file(), ra0, dec0, radius)?
            .into_iter()
            .filter(|(chunk, _)| *chunk == header.chunk_id())
            .map(|(_, run)| run)
            .collect();
    let (mut outside, mut records) = (0, 0);
    catalog.visit_pixel_runs(&runs, |pixel, bytes| {
        for record in bytes.chunks_exact(16) {
            records += 1;
            let home = decode_star(CatType::Astrometric, level, pixel, record)
                .map(|star| pixel_of(level, star.ra_deg, star.dec_deg));
            if home != Some(pixel) {
                outside += 1;
            }
        }
        Ok(())
    })?;
    Ok((outside, records))
}

fn siril_extract_problems<S: ByteSource>(catalog: &HealpixCatalog<S>) -> Vec<String> {
    let mut problems = Vec::new();
    for cone in [ORIGIN_CONE, ORION_CONE] {
        let (ra0, dec0, radius) = cone;
        let at = format!("cone ({ra0}, {dec0}) radius {radius}");
        let rows = match catalog.cone(ra0, dec0, radius, None) {
            Ok(rows) => rows,
            Err(error) => {
                problems.push(format!("{at}: {error}"));
                continue;
            }
        };
        if rows.len() < 50 {
            problems.push(format!("{at}: {} rows", rows.len()));
        }
        let with_teff = rows.iter().filter(|row| row.teff_k.is_some()).count();
        if cone == ORIGIN_CONE && with_teff * 10 < rows.len() * 9 {
            problems.push(format!(
                "{at}: {with_teff} of {} rows carry Teff",
                rows.len()
            ));
        }
        match records_outside_their_pixel(catalog, cone) {
            Ok((outside, records)) if records > 0 && outside * 1000 <= records => {}
            Ok((outside, records)) => problems.push(format!(
                "{at}: {outside} of {records} records decode outside the pixel whose index entry holds them"
            )),
            Err(error) => problems.push(format!("{at}: {error}")),
        }
        let magnitudes: Vec<f64> = rows.iter().filter_map(|row| row.g).collect();
        let temperatures: Vec<f64> = rows.iter().filter_map(|row| row.teff_k).collect();
        let proper_motions: Vec<f64> = rows
            .iter()
            .flat_map(|row| [row.pm_ra_masyr, row.pm_dec_masyr])
            .flatten()
            .map(f64::abs)
            .collect();
        check_distribution(
            &mut problems,
            &at,
            "G",
            &magnitudes,
            -2.0..=21.0,
            10.0..=21.0,
        );
        check_distribution(
            &mut problems,
            &at,
            "Teff",
            &temperatures,
            2000.0..=60_000.0,
            3000.0..=10_000.0,
        );
        check_distribution(
            &mut problems,
            &at,
            "|proper motion| (mas/yr)",
            &proper_motions,
            0.0..=11_000.0,
            0.0..=200.0,
        );
    }
    problems
}

#[test]
fn header_parses_the_spec_layout_and_refuses_an_unknown_type() {
    let title = "Siril Gaia DR3 astrometric extract";
    let mut bytes = [0u8; HEADER_LEN];
    bytes[..title.len()].copy_from_slice(title.as_bytes());
    bytes[48] = 3;
    bytes[49] = 8;
    bytes[50] = 1;
    bytes[51] = 0;
    bytes[52] = 0;
    bytes[53..57].copy_from_slice(&0u32.to_le_bytes());
    bytes[57..61].copy_from_slice(&0u32.to_le_bytes());
    bytes[61..65].copy_from_slice(&786_431u32.to_le_bytes());

    let header = CatHeader::parse(&bytes);

    assert_eq!(header.title, title);
    assert_eq!(
        (header.gaia_version, header.healpix_level, header.cat_type),
        (3, 8, 1)
    );
    assert_eq!((header.chunked, header.chunk_level), (0, 0));
    assert_eq!(
        (
            header.chunk_healpix,
            header.first_healpixel,
            header.last_healpixel
        ),
        (0, 0, 786_431)
    );
    assert_eq!(header.cat_type(), Some(CatType::Astrometric));
    assert_eq!(header.record_len(), Some(16));
    assert_eq!(header.index_entries(), 786_432);
    assert_eq!(header.index_bytes(), 3_145_728);
    assert_eq!(header.to_bytes(), bytes);
    header
        .validate(HEADER_LEN as u64 + 3_145_728)
        .unwrap_or_else(|e| panic!("a spec header validates: {e}"));

    let mut unknown = bytes;
    unknown[50] = 7;
    let message = CatHeader::parse(&unknown)
        .validate(u64::MAX)
        .map_err(|e| e.to_string())
        .unwrap_err();
    assert!(
        message.contains("cat_type") && message.contains('7'),
        "{message}"
    );

    let record_lengths: Vec<Option<usize>> = [1u8, 2, 3, 128, 129]
        .iter()
        .map(|&code| CatType::from_code(code).and_then(CatType::record_len))
        .collect();
    assert_eq!(
        record_lengths,
        vec![Some(16), Some(701), None, Some(6), Some(12)]
    );
}

#[test]
fn header_validation_names_the_offending_field() {
    let base = CatHeader::whole("fields", 4, 1);
    let cases: Vec<(CatHeader, &str)> = vec![
        (
            CatHeader {
                healpix_level: 13,
                ..base.clone()
            },
            "healpix_level",
        ),
        (
            CatHeader {
                chunked: 2,
                ..base.clone()
            },
            "chunked",
        ),
        (
            CatHeader {
                chunked: 1,
                chunk_level: 5,
                ..base.clone()
            },
            "chunk_level",
        ),
        (
            CatHeader {
                first_healpixel: 10,
                last_healpixel: 9,
                ..base.clone()
            },
            "last_healpixel",
        ),
        (
            CatHeader {
                cat_type: 3,
                ..base.clone()
            },
            "cat_type",
        ),
        (
            CatHeader {
                cat_type: 128,
                record_kind: 0,
                epoch_year: 2026.0,
                ..base.clone()
            },
            "record_kind",
        ),
        (
            CatHeader {
                cat_type: 128,
                record_kind: ASTROBURST_RECORD_KIND,
                epoch_year: f32::NAN,
                ..base.clone()
            },
            "epoch_year",
        ),
    ];
    for (header, field) in cases {
        let message = header
            .validate(u64::MAX)
            .map_err(|e| e.to_string())
            .unwrap_err();
        assert!(message.contains(field), "{field}: {message}");
    }
    base.validate(u64::MAX)
        .unwrap_or_else(|e| panic!("the base header is valid: {e}"));
}

#[test]
fn index_pixel_range_is_cumulative_end_exclusive() {
    let header = CatHeader::whole("level 1", 1, 1);
    let pixels = vec![
        (0, stars_in(1, 0, 2, 1)),
        (5, stars_in(1, 5, 3, 2)),
        (47, stars_in(1, 47, 1, 3)),
    ];
    let bytes = written(&header, &pixels);
    assert_eq!(bytes.len(), HEADER_LEN + 48 * 4 + 6 * 16);
    let index: Vec<u32> = bytes[HEADER_LEN..HEADER_LEN + 48 * 4]
        .chunks_exact(4)
        .map(|entry| u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]))
        .collect();
    assert_eq!((index[0], index[4], index[5]), (2, 2, 5));
    assert_eq!((index[46], index[47]), (5, 6));

    let catalog = mem_catalog(bytes);

    assert_eq!(catalog.pixel_range(0).unwrap(), 0..2);
    assert_eq!(catalog.pixel_range(1).unwrap(), 2..2);
    assert_eq!(catalog.pixel_range(5).unwrap(), 2..5);
    assert_eq!(catalog.pixel_range(47).unwrap(), 5..6);
    assert!(refusal(catalog.pixel_range(48)).contains("48"));
    assert_eq!(
        catalog.record_offset(5),
        (HEADER_LEN + 48 * 4 + 5 * 16) as u64
    );
    assert_eq!(catalog.total_records(), 6);
}

#[test]
fn a_chunked_file_sizes_its_index_per_chunk_and_offsets_pixel_ids() {
    let header = CatHeader::chunk("chunk 17", 8, 1, 1, 17);
    assert_eq!(header.first_healpixel, 278_528);
    assert_eq!(header.last_healpixel, 278_528 + 16_383);
    assert_eq!(header.index_entries(), 16_384);
    assert_eq!(288_340 >> 14, 17);

    let bytes = written(
        &header,
        &[
            (288_340, stars_in(8, 288_340, 3, 4)),
            (288_362, stars_in(8, 288_362, 2, 5)),
        ],
    );

    assert_eq!(bytes.len(), HEADER_LEN + 65_536 + 5 * 16);
    let entry = |e: usize| {
        let at = HEADER_LEN + 4 * e;
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    };
    assert_eq!((entry(9_811), entry(9_812)), (0, 3));
    assert_eq!((entry(9_833), entry(9_834)), (3, 5));
    assert_eq!(entry(16_383), 5);

    let catalog = mem_catalog(bytes.clone());
    assert_eq!(catalog.pixel_range(288_340).unwrap(), 0..3);
    assert_eq!(catalog.pixel_range(288_341).unwrap(), 3..3);
    assert_eq!(catalog.pixel_range(288_362).unwrap(), 3..5);
    assert!(refusal(catalog.pixel_range(278_527)).contains("278527"));
    assert!(refusal(catalog.pixel_range(294_912)).contains("294912"));
}

#[test]
fn a_chunked_header_whose_range_does_not_match_the_chunk_is_refused() {
    let one_pixel_too_many = CatHeader {
        last_healpixel: 278_528 + 16_384,
        ..CatHeader::chunk("c", 8, 1, 1, 17)
    };
    let message = one_pixel_too_many
        .validate(u64::MAX)
        .map_err(|e| e.to_string())
        .unwrap_err();
    assert!(
        message.contains("last_healpixel") && message.contains("16384"),
        "{message}"
    );

    let shifted = CatHeader {
        first_healpixel: 278_528 + 16_384,
        last_healpixel: 278_528 + 2 * 16_384 - 1,
        ..CatHeader::chunk("c", 8, 1, 1, 17)
    };
    let message = shifted
        .validate(u64::MAX)
        .map_err(|e| e.to_string())
        .unwrap_err();
    assert!(message.contains("first_healpixel"), "{message}");

    let mut bytes = written(&CatHeader::chunk("c", 8, 1, 1, 17), &[]);
    bytes[..HEADER_LEN].copy_from_slice(&one_pixel_too_many.to_bytes());
    assert!(refusal(HealpixCatalog::new(MemSource::new(bytes))).contains("last_healpixel"));

    let foreign_chunk = CatHeader::chunk("c", 8, 1, 1, 48);
    let message = foreign_chunk
        .validate(u64::MAX)
        .map_err(|e| e.to_string())
        .unwrap_err();
    assert!(message.contains("chunk_healpix"), "{message}");
}

#[test]
fn type1_decodes_ra_dec_scale_pm_teff_mag() {
    assert_eq!(encode_siril_coordinate(180.0), Some(1_073_741_824));
    assert_eq!(encode_siril_coordinate(-30.0), Some(-178_956_971));
    let mut record = Vec::new();
    record.extend_from_slice(&1_073_741_824i32.to_le_bytes());
    record.extend_from_slice(&(-178_956_971i32).to_le_bytes());
    record.extend_from_slice(&(-1234i16).to_le_bytes());
    record.extend_from_slice(&567i16.to_le_bytes());
    record.extend_from_slice(&5778u16.to_le_bytes());
    record.extend_from_slice(&12_345i16.to_le_bytes());
    assert_eq!(record.len(), 16);

    let star = decode_star(CatType::Astrometric, 8, 0, &record).unwrap();

    assert!(
        (star.ra_deg - 180.0).abs() <= SIRIL_QUANTUM_DEG / 2.0 + 1e-12,
        "{}",
        star.ra_deg
    );
    assert!(
        (star.dec_deg + 30.0).abs() <= SIRIL_QUANTUM_DEG / 2.0 + 1e-12,
        "{}",
        star.dec_deg
    );
    assert_eq!(star.pm_ra_masyr, Some(-1234.0));
    assert_eq!(star.pm_dec_masyr, Some(567.0));
    assert_eq!(star.teff_k, Some(5778.0));
    assert_eq!(star.mag, Some(12.345));

    let mut no_teff = record.clone();
    no_teff[12..14].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(
        decode_star(CatType::Astrometric, 8, 0, &no_teff)
            .unwrap()
            .teff_k,
        None
    );

    let pixel = pixel_of(8, 180.0, -30.0);
    let catalog = mem_catalog(written(
        &CatHeader::whole("type 1", 8, 1),
        &[(pixel, vec![Record::Raw(record)])],
    ));
    let rows = catalog.cone(180.0, -30.0, 0.01, None).unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.id, format!("G{pixel}-0"));
    assert_eq!(
        (row.g, row.teff_k, row.pm_ra_masyr, row.pm_dec_masyr),
        (Some(12.345), Some(5778.0), Some(-1234.0), Some(567.0))
    );
    assert_eq!((row.ra_epoch, row.dec_epoch), (row.ra, row.dec));
    assert_eq!(
        (row.bp, row.rp, row.bp_rp, row.parallax_mas),
        (None, None, None, None)
    );

    let propagated = catalog.cone(180.0, -30.0, 0.01, Some(2026.0)).unwrap();
    let expected_dec = row.dec + 567.0 * 10.0 / 3_600_000.0;
    assert!(
        (propagated[0].dec_epoch - expected_dec).abs() < 1e-12,
        "{}",
        propagated[0].dec_epoch
    );
}

#[test]
fn type128_offsets_reconstruct_sky_within_0_05_arcsec() {
    let level = 8;
    for &(ra, dec) in &[(15.0, 0.0), (200.0, 60.0), (10.0, 89.95), (359.99, -45.0)] {
        let pixel = pixel_of(level, ra, dec);
        let (centre_lon, centre_lat) = cdshealpix::nested::center(level, u64::from(pixel));
        let mut targets = vec![(centre_lon.to_degrees(), centre_lat.to_degrees())];
        targets.extend(
            cdshealpix::nested::vertices(level, u64::from(pixel))
                .iter()
                .map(|&(lon, lat)| (lon.to_degrees(), lat.to_degrees())),
        );
        targets.push((ra, dec));
        for (i, &(target_ra, target_dec)) in targets.iter().enumerate() {
            let back = round_trip(
                CatType::Compact6,
                level,
                pixel,
                CatalogStar::at(target_ra, target_dec, Some(9.0)),
            );
            let (xi, eta) = standard_coordinates(
                centre_lon,
                centre_lat,
                target_ra.to_radians(),
                target_dec.to_radians(),
            )
            .unwrap();
            let (back_xi, back_eta) = standard_coordinates(
                centre_lon,
                centre_lat,
                back.ra_deg.to_radians(),
                back.dec_deg.to_radians(),
            )
            .unwrap();
            let error_xi = (xi - back_xi).to_degrees() * ARCSEC_PER_DEGREE;
            let error_eta = (eta - back_eta).to_degrees() * ARCSEC_PER_DEGREE;
            assert!(
                error_xi.abs() <= 0.05 + 1e-9 && error_eta.abs() <= 0.05 + 1e-9,
                "pixel {pixel} target {i}: {error_xi} {error_eta}"
            );
            let separation = angular_separation(target_ra, target_dec, back.ra_deg, back.dec_deg)
                * ARCSEC_PER_DEGREE;
            assert!(separation <= 0.05 * 2f64.sqrt() + 1e-9, "{separation}");
            if i == 0 {
                assert!(separation < 1e-6, "the pixel centre is exact: {separation}");
            }
        }
    }

    let pixel = pixel_of(level, 15.0, 0.0);
    let far = CatalogStar::at(16.0, 0.0, Some(9.0));
    let message = encode_record(
        CatType::Compact6,
        level,
        pixel,
        &Record::Star(far),
        &mut Vec::new(),
    )
    .map_err(|e| e.to_string())
    .unwrap_err();
    assert!(message.contains("offset"), "{message}");
}

#[test]
fn type128_round_trips_magnitudes_from_minus_1_to_16() {
    let level = 8;
    let pixel = pixel_of(level, 120.0, -20.0);
    let (ra, dec) = centre_deg(level, pixel);
    for k in -10..=160 {
        let mag = f64::from(k) / 10.0;
        let back = round_trip(
            CatType::Compact6,
            level,
            pixel,
            CatalogStar::at(ra, dec, Some(mag)),
        );
        assert_eq!(back.mag, Some(mag), "{mag}");
    }
    let no_mag = round_trip(
        CatType::Compact6,
        level,
        pixel,
        CatalogStar::at(ra, dec, None),
    );
    assert_eq!(no_mag.mag, None);

    assert_eq!(compact_mag_code(Some(-1.5)).ok(), Some(0));
    assert_eq!(compact_mag_code(Some(9.0)).ok(), Some(105));
    assert_eq!(compact_mag_code(Some(23.9)).ok(), Some(254));
    assert_eq!(compact_mag_code(None).ok(), Some(COMPACT_NO_MAGNITUDE));
    for outside in [24.0, -1.6, f64::NAN] {
        assert!(compact_mag_code(Some(outside)).is_err(), "{outside}");
    }

    for colour in [-2.54, -1.0, 0.0, 0.62, 2.54] {
        let star = CatalogStar {
            colour: Some(colour),
            ..CatalogStar::at(ra, dec, Some(9.0))
        };
        assert_eq!(
            round_trip(CatType::Compact6, level, pixel, star).colour,
            Some(colour)
        );
    }
    let colourless = round_trip(
        CatType::Compact6,
        level,
        pixel,
        CatalogStar::at(ra, dec, Some(9.0)),
    );
    assert_eq!(colourless.colour, None);
    let too_red = CatalogStar {
        colour: Some(2.56),
        ..CatalogStar::at(ra, dec, Some(9.0))
    };
    let message = encode_record(
        CatType::Compact6,
        level,
        pixel,
        &Record::Star(too_red),
        &mut Vec::new(),
    )
    .map_err(|e| e.to_string())
    .unwrap_err();
    assert!(message.contains("colour"), "{message}");
}

#[test]
fn type129_round_trips() {
    let level = 8;
    let cases = [
        (0.0, -90.0, Some(-1.234), Some(0.567)),
        (359.999_999_9, 89.9, Some(15.5), None),
        (123.456_789, -12.345_678, None, Some(-0.25)),
    ];
    for (ra, dec, mag, colour) in cases {
        let pixel = pixel_of(level, ra, dec);
        let star = CatalogStar {
            colour,
            ..CatalogStar::at(ra, dec, mag)
        };
        let mut bytes = Vec::new();
        encode_record(
            CatType::Compact12,
            level,
            pixel,
            &Record::Star(star),
            &mut bytes,
        )
        .unwrap();
        assert_eq!(bytes.len(), 12);
        assert_eq!(
            i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            encode_siril_coordinate(ra).unwrap()
        );

        let back = decode_star(CatType::Compact12, level, pixel, &bytes).unwrap();

        assert!(ra_difference_deg(back.ra_deg, ra) <= SIRIL_QUANTUM_DEG / 2.0 + 1e-12);
        assert!((back.dec_deg - dec).abs() <= SIRIL_QUANTUM_DEG / 2.0 + 1e-12);
        assert_eq!((back.mag, back.colour), (mag, colour));
        assert_eq!((back.pm_ra_masyr, back.teff_k), (None, None));
    }
}

#[test]
fn type2_records_are_returned_raw_and_cone_refuses_the_type_by_name() {
    let first: Vec<u8> = (0..701u32).map(|i| (i * 7 % 251) as u8).collect();
    let second: Vec<u8> = (0..701u32).map(|i| (i * 13 % 241) as u8).collect();
    let header = CatHeader::whole("xp", 1, 2);
    let catalog = mem_catalog(written(
        &header,
        &[(
            3,
            vec![Record::Raw(first.clone()), Record::Raw(second.clone())],
        )],
    ));
    let both = [first, second].concat();

    assert_eq!(catalog.read_records_raw(3..4).unwrap(), both);
    assert_eq!(catalog.read_records_raw(0..48).unwrap(), both);
    assert!(catalog.read_records_raw(4..4).unwrap().is_empty());
    let (ra, dec) = centre_deg(1, 3);
    let message = refusal(catalog.cone(ra, dec, 1.0, None));
    assert!(
        message.contains("XP sampled") && message.contains("read_records_raw"),
        "{message}"
    );

    let short = write_catalog(
        &header,
        &[(3, vec![Record::Raw(vec![0; 700])])],
        &mut Vec::new(),
    )
    .map_err(|e| e.to_string())
    .unwrap_err();
    assert!(short.contains("701"), "{short}");
}

#[test]
fn cone_returns_exactly_the_records_inside_the_radius() {
    let level = 8;
    let anchor = pixel_of(level, 40.0, 25.0);
    let (vertex_lon, vertex_lat) = cdshealpix::nested::vertices(level, u64::from(anchor))[0];
    let (ra0, dec0) = (vertex_lon.to_degrees(), vertex_lat.to_degrees());
    let step = 1e-4;
    let mut touching: Vec<u32> = [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)]
        .iter()
        .map(|&(d_lon, d_lat)| {
            cdshealpix::nested::hash(level, vertex_lon + d_lon, vertex_lat + d_lat) as u32
        })
        .collect();
    touching.sort_unstable();
    touching.dedup();
    assert!(touching.len() >= 3, "{touching:?}");
    let pixels: Vec<(u32, Vec<Record>)> = touching
        .iter()
        .take(3)
        .enumerate()
        .map(|(i, &pixel)| (pixel, stars_in(level, pixel, 50, i as u64 + 1)))
        .collect();
    let mut by_distance: Vec<(f64, String)> = Vec::new();
    for (pixel, records) in &pixels {
        for (n, record) in records.iter().enumerate() {
            if let Record::Star(star) = record {
                by_distance.push((
                    angular_separation(ra0, dec0, star.ra_deg, star.dec_deg),
                    format!("G{pixel}-{n}"),
                ));
            }
        }
    }
    by_distance.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(by_distance.len(), 150);
    assert!(by_distance[36].0 < by_distance[37].0);
    let radius = (by_distance[36].0 + by_distance[37].0) / 2.0;

    let rows = mem_catalog(written(&CatHeader::whole("cone", level, 1), &pixels))
        .cone(ra0, dec0, radius, None)
        .unwrap();

    assert_eq!(rows.len(), 37);
    let expected: BTreeSet<String> = by_distance[..37].iter().map(|(_, id)| id.clone()).collect();
    assert_eq!(ids_of(&rows), expected);
    let pixels_hit: BTreeSet<&str> = rows
        .iter()
        .filter_map(|row| row.id.split('-').next())
        .collect();
    assert!(pixels_hit.len() >= 2, "{pixels_hit:?}");
    assert!(rows.windows(2).all(|pair| pair[0].g <= pair[1].g));
}

#[test]
fn cone_at_the_pole_and_across_ra_zero_returns_the_same_count() {
    let level = 8;
    let radius = 0.2;
    let mut counts = Vec::new();
    for &(ra0, dec0) in &[(37.0, 90.0), (0.0, 20.0)] {
        let stars = pattern_around(ra0, dec0, 300, 0.3, 11);
        let pixels = grouped(level, &stars);

        let rows = mem_catalog(written(&CatHeader::whole("placement", level, 1), &pixels))
            .cone(ra0, dec0, radius, None)
            .unwrap();

        let ids = ids_of(&rows);
        assert_eq!(ids.len(), rows.len(), "no duplicates at ({ra0}, {dec0})");
        assert_eq!(
            ids,
            expected_ids(&pixels, ra0, dec0, radius),
            "({ra0}, {dec0})"
        );
        counts.push(rows.len());
        if dec0 < 89.0 {
            assert!(rows.iter().any(|row| row.ra > 359.0), "stars west of RA 0");
            assert!(rows.iter().any(|row| row.ra < 1.0), "stars east of RA 0");
        }
    }
    assert_eq!(counts[0], counts[1]);
    assert!(counts[0] > 50, "{counts:?}");
}

#[test]
fn pixel_ranges_group_by_chunk_and_merge_consecutive_pixels() {
    assert_eq!(pixel_of(8, 15.0, 0.0), 288_358);
    assert_eq!(288_358 >> 14, 17);

    let runs = pixel_ranges(8, Some(1), 15.0, 0.0, 0.05).unwrap();

    assert!(runs.iter().all(|(chunk, _)| *chunk == 17), "{runs:?}");
    assert!(
        runs.iter().any(|(_, run)| run.contains(&288_358)),
        "{runs:?}"
    );
    assert!(
        runs.windows(2).all(|pair| pair[0].1.end < pair[1].1.start),
        "{runs:?}"
    );
    let flat: Vec<u32> = runs.iter().flat_map(|(_, run)| run.clone()).collect();
    let reference: Vec<u32> = cdshealpix::nested::cone_coverage_approx_custom(
        8,
        2,
        15f64.to_radians(),
        0.0,
        0.05f64.to_radians(),
    )
    .to_flat_array()
    .iter()
    .map(|&pixel| pixel as u32)
    .collect();
    assert_eq!(flat, reference);
    let whole = pixel_ranges(8, None, 15.0, 0.0, 0.05).unwrap();
    assert_eq!(
        whole.iter().map(|(_, run)| run.clone()).collect::<Vec<_>>(),
        runs.iter().map(|(_, run)| run.clone()).collect::<Vec<_>>()
    );
    assert!(whole.iter().all(|(chunk, _)| *chunk == 0));

    assert_eq!(
        group_by_chunk(8, Some(1), [288_340..288_350, 288_350..288_363]),
        vec![(17, 288_340..288_363)]
    );
    assert_eq!(
        group_by_chunk(8, Some(1), [294_900..294_920]),
        vec![(17, 294_900..294_912), (18, 294_912..294_920)]
    );
    assert_eq!(
        group_by_chunk(8, None, [294_900..294_920]),
        vec![(0, 294_900..294_920)]
    );
}

#[test]
fn cone_on_a_chunked_file_equals_the_cone_on_the_whole_file() {
    let stars = pattern_around(15.0, 0.0, 300, 0.3, 5);
    let pixels = grouped(8, &stars);
    assert!(pixels.iter().all(|(pixel, _)| pixel >> 14 == 17));

    let whole = mem_catalog(written(&CatHeader::whole("whole", 8, 1), &pixels))
        .cone(15.0, 0.0, 0.2, None)
        .unwrap();
    let chunk = mem_catalog(written(&CatHeader::chunk("chunk 17", 8, 1, 1, 17), &pixels))
        .cone(15.0, 0.0, 0.2, None)
        .unwrap();
    let neighbour = mem_catalog(written(&CatHeader::chunk("chunk 18", 8, 1, 1, 18), &[]))
        .cone(15.0, 0.0, 0.2, None)
        .unwrap();

    assert!(whole.len() > 50, "{}", whole.len());
    assert_eq!(whole, chunk);
    assert!(neighbour.is_empty());
}

#[test]
fn rows_without_magnitude_sort_last() {
    let level = 8;
    let pixel = pixel_of(level, 50.0, 10.0);
    let (ra, dec) = centre_deg(level, pixel);
    let header = CatHeader::whole("ASTROBURST-TYC2-V1 VT", level, 128).with_epoch_year(2026.0);
    let records = vec![
        Record::Star(CatalogStar::at(ra, dec, Some(12.0))),
        Record::Star(CatalogStar::at(ra + 0.01, dec, None)),
        Record::Star(CatalogStar::at(ra, dec + 0.01, Some(3.5))),
    ];
    let catalog = mem_catalog(written(&header, &[(pixel, records)]));

    let rows = catalog.cone(ra, dec, 0.1, None).unwrap();

    assert_eq!(
        rows.iter().map(|row| row.g).collect::<Vec<_>>(),
        vec![Some(3.5), Some(12.0), None]
    );
    assert!(rows.iter().all(|row| row.id.starts_with('T')));
    assert!(rows
        .iter()
        .all(|row| row.pm_ra_masyr.is_none() && row.teff_k.is_none()));
    assert_eq!(catalog.header().epoch_year, 2026.0);
    assert_eq!(catalog.header().record_kind, ASTROBURST_RECORD_KIND);
    assert_eq!(catalog.header().title, "ASTROBURST-TYC2-V1 VT");
}

#[test]
fn a_catalogue_is_shared_across_threads_behind_an_arc() {
    fn shareable<T: Send + Sync>() {}
    shareable::<HealpixCatalog<MmapSource>>();
    shareable::<HealpixCatalog<MemSource>>();
    #[cfg(feature = "vizier")]
    shareable::<HealpixCatalog<HttpRangeSource>>();
    let pixels = grouped(8, &pattern_around(15.0, 0.0, 300, 0.3, 5));
    let catalog = std::sync::Arc::new(mem_catalog(written(
        &CatHeader::whole("shared", 8, 1),
        &pixels,
    )));
    let expected = catalog.cone(15.0, 0.0, 0.2, None).unwrap();

    let results: Vec<Vec<CatalogRow>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let catalog = std::sync::Arc::clone(&catalog);
                scope.spawn(move || catalog.cone(15.0, 0.0, 0.2, None).unwrap())
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect()
    });

    assert!(results.iter().all(|rows| *rows == expected));
}

#[test]
fn invalid_cones_are_refused_without_panicking() {
    let catalog = mem_catalog(written(&CatHeader::whole("tiny", 1, 1), &[]));
    let cases = [
        (f64::NAN, 0.0, 1.0),
        (f64::INFINITY, 0.0, 1.0),
        (0.0, 90.5, 1.0),
        (0.0, -91.0, 1.0),
        (0.0, 0.0, 0.0),
        (0.0, 0.0, -1.0),
        (0.0, 0.0, 181.0),
        (0.0, 0.0, f64::NAN),
        (0.0, 0.0, 5e-324),
        (f64::MAX, 45.0, 5e-324),
    ];
    for (ra, dec, radius) in cases {
        let message = refusal(catalog.cone(ra, dec, radius, None));
        assert!(
            message.starts_with("cone"),
            "({ra}, {dec}, {radius}): {message}"
        );
    }
    assert!(pixel_ranges(13, None, 0.0, 0.0, 1.0).is_err());
    assert!(pixel_ranges(8, Some(9), 0.0, 0.0, 1.0).is_err());
    assert!(pixel_ranges(8, None, 0.0, 0.0, 5e-324).is_err());
    assert!(catalog.cone(0.0, 90.0, 180.0, None).unwrap().is_empty());
    assert!(catalog.cone(-725.0, -90.0, 1.0, None).unwrap().is_empty());
}

#[test]
fn a_cone_wider_than_a_hemisphere_returns_every_star_inside_it() {
    let level = 4;
    let pixels = grouped(level, &whole_sky_stars(12_000, 23));
    let catalog = mem_catalog(written(&CatHeader::whole("sky", level, 1), &pixels));

    for (ra0, dec0) in [(0.0, 0.0), (123.4, 56.7)] {
        for radius in [100.0, 135.0, 150.0, 170.0, 179.999_999_99, 180.0] {
            let rows = catalog.cone(ra0, dec0, radius, None).unwrap();

            let expected = expected_ids(&pixels, ra0, dec0, radius);
            let found = ids_of(&rows);
            assert_eq!(
                (rows.len(), found.len()),
                (expected.len(), expected.len()),
                "({ra0}, {dec0}) radius {radius}"
            );
            assert!(found == expected, "({ra0}, {dec0}) radius {radius}");
        }
    }

    let every_chunk: Vec<(u32, Range<u32>)> = (0u32..48)
        .map(|chunk| (chunk, chunk * 16_384..(chunk + 1) * 16_384))
        .collect();
    assert_eq!(
        pixel_ranges(8, Some(1), 0.0, 0.0, 150.0).unwrap(),
        every_chunk
    );
}

#[test]
fn a_record_whose_dec_decodes_beyond_the_pole_is_refused() {
    let level = 8;
    let beyond_the_pole = encode_siril_coordinate(90.05).unwrap().to_le_bytes();
    let near_the_pole = star_at(10.0, 89.95, 12.0);
    for cat_type in [CatType::Astrometric, CatType::Compact12] {
        let mut record = Vec::new();
        encode_record(cat_type, level, 0, &star_at(10.0, -90.0, 12.0), &mut record).unwrap();
        assert!(decode_siril_coordinate(i32::from_le_bytes(take(&record, 4))) < -90.0);
        assert_eq!(
            decode_star(cat_type, level, 0, &record).map(|star| star.dec_deg),
            Some(-90.0),
            "{cat_type:?}: the pole rounds half a count beyond 90 deg and still decodes"
        );
        record[4..8].copy_from_slice(&beyond_the_pole);

        assert_eq!(
            decode_star(cat_type, level, 0, &record),
            None,
            "{cat_type:?}"
        );
    }

    let pixel = pixel_of(level, 10.0, 89.95);
    let mut bytes = written(
        &CatHeader::whole("pole", level, 1),
        &[(pixel, vec![near_the_pole])],
    );
    let dec_at = mem_catalog(bytes.clone()).record_offset(0) as usize + 4;
    bytes[dec_at..dec_at + 4].copy_from_slice(&beyond_the_pole);

    let message = refusal(mem_catalog(bytes).cone(10.0, 89.95, 0.2, None));

    assert!(
        message.contains(&pixel.to_string()) && message.contains("record 0"),
        "{message}"
    );
}

#[test]
fn a_cone_over_more_records_than_the_budget_is_refused_naming_the_count() {
    let pixels = grouped(8, &pattern_around(15.0, 0.0, 300, 0.3, 5));
    let catalog = mem_catalog(written(&CatHeader::whole("budget", 8, 1), &pixels));
    let covered: usize = pixel_ranges(8, None, 15.0, 0.0, 0.2)
        .unwrap()
        .into_iter()
        .flat_map(|(_, run)| run)
        .map(|pixel| catalog.pixel_range(pixel).unwrap().len())
        .sum();
    assert!(covered > 100, "{covered}");

    let within = catalog.cone_within(15.0, 0.0, 0.2, None, covered).unwrap();
    let message = refusal(catalog.cone_within(15.0, 0.0, 0.2, None, covered - 1));

    assert!(!within.is_empty());
    assert_eq!(within, catalog.cone(15.0, 0.0, 0.2, None).unwrap());
    assert!(
        message.starts_with("cone")
            && message.contains(&covered.to_string())
            && message.contains("visit_pixel_runs"),
        "{message}"
    );
    assert_eq!(CONE_RECORD_BUDGET, 2_000_000);
}

#[test]
fn the_siril_extract_check_catches_a_wrong_scale_or_field_order() {
    let good = siril_like_extract();
    let data_start = mem_catalog(good.clone()).record_offset(0) as usize;
    let problems = siril_extract_problems(&mem_catalog(good.clone()));
    assert!(problems.is_empty(), "{problems:#?}");

    let cases: [(&str, fn(&mut [u8])); 4] = [
        ("Dec stored at 180/(2^31-1) deg per count", |record| {
            let dec = i32::from_le_bytes(take(record, 4));
            record[4..8].copy_from_slice(&dec.saturating_mul(2).to_le_bytes());
        }),
        ("RA and Dec swapped", |record| record[0..8].rotate_left(4)),
        ("Teff and G swapped", |record| record[12..16].rotate_left(2)),
        ("proper motion in RA and Teff swapped", |record| {
            let pm_ra: [u8; 2] = take(record, 8);
            let teff: [u8; 2] = take(record, 12);
            record[8..10].copy_from_slice(&teff);
            record[12..14].copy_from_slice(&pm_ra);
        }),
    ];
    for (what, mutate) in cases {
        let mut bytes = good.clone();
        bytes[data_start..].chunks_exact_mut(16).for_each(mutate);

        let problems = siril_extract_problems(&mem_catalog(bytes));

        assert!(!problems.is_empty(), "{what} passes the real-file check");
    }
}

#[test]
fn the_writer_refuses_unsorted_duplicate_or_foreign_pixels() {
    let header = CatHeader::chunk("c", 8, 1, 1, 17);
    let invalid: Vec<Vec<(u32, Vec<Record>)>> = vec![
        vec![(288_341, Vec::new()), (288_340, Vec::new())],
        vec![(288_340, Vec::new()), (288_340, Vec::new())],
        vec![(278_527, Vec::new())],
        vec![(294_912, Vec::new())],
    ];
    for pixels in invalid {
        assert!(
            write_catalog(&header, &pixels, &mut Vec::new()).is_err(),
            "{pixels:?}"
        );
    }
    let message = write_catalog(
        &CatHeader::whole("w", 1, 1),
        &[(0, vec![Record::Star(CatalogStar::at(1.0, 1.0, None))])],
        &mut Vec::new(),
    )
    .map_err(|e| e.to_string())
    .unwrap_err();
    assert!(message.contains("magnitude"), "{message}");
}

#[test]
fn mmap_source_reads_the_same_bytes_as_memory() {
    let stars = pattern_around(15.0, 0.0, 300, 0.3, 5);
    let pixels = grouped(8, &stars);
    let bytes = written(&CatHeader::whole("mmap", 8, 1), &pixels);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("level8.dat");
    std::fs::write(&path, &bytes).unwrap();
    {
        let source = MmapSource::open(&path).unwrap();
        assert_eq!(source.len(), bytes.len() as u64);
        for &(offset, len) in &[
            (0usize, 128usize),
            (127, 1),
            (128 + 4000, 64),
            (bytes.len() - 16, 16),
        ] {
            let mut buffer = vec![0u8; len];
            source.read_at(offset as u64, &mut buffer).unwrap();
            assert_eq!(buffer, bytes[offset..offset + len]);
        }
        let mut past_the_end = [0u8; 2];
        let error = source
            .read_at(bytes.len() as u64 - 1, &mut past_the_end)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);

        let from_file = HealpixCatalog::open_file(&path)
            .unwrap()
            .cone(15.0, 0.0, 0.2, None)
            .unwrap();
        let from_memory = mem_catalog(bytes.clone())
            .cone(15.0, 0.0, 0.2, None)
            .unwrap();
        assert!(!from_file.is_empty());
        assert_eq!(from_file, from_memory);
    }
}

#[test]
fn a_truncated_index_is_refused_naming_the_expected_length() {
    let bytes = written(
        &CatHeader::whole("level 3", 3, 1),
        &[(5, stars_in(3, 5, 4, 3))],
    );
    assert_eq!(bytes.len(), 128 + 768 * 4 + 4 * 16);
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("cut-index.dat", 128 + 1000, "3200"),
        ("cut-records.dat", bytes.len() - 1, "3264"),
        ("cut-header.dat", 100, "128"),
    ];
    for (name, keep, expected) in cases {
        let path = dir.path().join(name);
        std::fs::write(&path, &bytes[..keep]).unwrap();

        let message = refusal(HealpixCatalog::open_file(&path));

        assert!(
            message.contains(expected) && message.contains(&keep.to_string()),
            "{name}: {message}"
        );
        let in_memory = refusal(HealpixCatalog::new(MemSource::new(bytes[..keep].to_vec())));
        assert_eq!(in_memory, message);
    }
}

#[test]
fn siril_astrometric_extract_opens_when_the_env_points_at_it() {
    let path = env_dir(ENV_SIRIL_CATALOG_DIR).join(SIRIL_ASTROMETRIC_FILE);
    if skip_if_absent(&path) {
        return;
    }
    let catalog =
        HealpixCatalog::open_file(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(catalog.header().cat_type, 1);
    assert_eq!(catalog.header().healpix_level, 8);

    let problems = siril_extract_problems(&catalog);

    assert!(problems.is_empty(), "{}: {problems:#?}", path.display());
}

#[test]
#[ignore]
fn cone_over_200_level8_pixels_from_mmap_takes_at_most_10_ms() {
    let (ra0, dec0) = (100.0, 30.0);
    let mut radius = 1.7;
    let mut covered: Vec<u32> = Vec::new();
    for _ in 0..20 {
        covered = pixel_ranges(8, None, ra0, dec0, radius)
            .unwrap()
            .into_iter()
            .flat_map(|(_, run)| run)
            .collect();
        if covered.len() <= 200 {
            break;
        }
        radius *= 0.97;
    }
    assert!((150..=200).contains(&covered.len()), "{}", covered.len());
    let pixels: Vec<(u32, Vec<Record>)> = covered
        .iter()
        .map(|&pixel| (pixel, stars_in(8, pixel, 127, u64::from(pixel))))
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("timing.dat");
    std::fs::write(&path, written(&CatHeader::whole("timing", 8, 1), &pixels)).unwrap();
    let catalog = HealpixCatalog::open_file(&path).unwrap();

    let mut timings = Vec::new();
    let mut rows = 0;
    for _ in 0..7 {
        let started = Instant::now();
        let found = catalog.cone(ra0, dec0, radius, None).unwrap();
        timings.push(started.elapsed());
        rows = found.len();
    }

    timings.sort();
    eprintln!(
        "cone over {} pixels, {rows} rows, radius {radius:.3} deg: {timings:?}",
        covered.len()
    );
    assert!(rows > 20_000, "{rows}");
    assert!(
        timings[3] <= Duration::from_millis(10),
        "median {:?}",
        timings[3]
    );
}

#[cfg(feature = "vizier")]
mod http {
    use super::*;
    use crate::infra::net::test_server::{serve, Resource, LAST_MODIFIED};

    const DATA_START: usize = HEADER_LEN + 192 * 4;

    fn level2_catalogue() -> Vec<u8> {
        let counts = [(9u32, 2usize), (10, 5000), (11, 3), (12, 1), (13, 2)];
        let pixels: Vec<(u32, Vec<Record>)> = counts
            .iter()
            .map(|&(pixel, count)| (pixel, stars_in(2, pixel, count, u64::from(pixel))))
            .collect();
        written(&CatHeader::whole("http", 2, 1), &pixels)
    }

    fn record_byte(record: usize) -> usize {
        DATA_START + record * 16
    }

    fn http_error(error: CatalogReadError) -> HttpRangeError {
        match error {
            CatalogReadError::Http(inner) => inner,
            other => panic!("expected an HTTP range error, got {other}"),
        }
    }

    #[test]
    fn range_source_coalesces_adjacent_runs_and_requires_206() {
        let bytes = level2_catalogue();
        let server = serve(Resource::new(bytes.clone()));

        let catalog = HealpixCatalog::open_http(&server.base, "level2.dat").unwrap();

        let opened = server.requests();
        assert_eq!(opened.len(), 2, "header and one index page: {opened:?}");
        assert_eq!(opened[0].target, "/level2.dat");
        assert_eq!(opened[0].header("range"), Some("bytes=0-127"));
        assert_eq!(opened[1].header("range"), Some("bytes=128-895"));
        assert!(opened.iter().all(|request| request.status == 206));

        let run = catalog.read_records_raw(11..14).unwrap();
        assert_eq!(run, bytes[record_byte(5002)..record_byte(5008)]);
        let after_run = server.requests();
        assert_eq!(after_run.len(), 3, "the index page stays in memory");
        let wanted = format!("bytes={}-{}", record_byte(5002), record_byte(5008) - 1);
        assert_eq!(after_run[2].header("range"), Some(wanted.as_str()));
        assert_eq!(after_run[2].header("if-range"), Some("\"ab-v0\""));

        let mut close = Vec::new();
        catalog
            .visit_pixel_runs(&[11..12, 13..14], |pixel, records| {
                close.push((pixel, records.to_vec()));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            close,
            vec![
                (11, bytes[record_byte(5002)..record_byte(5005)].to_vec()),
                (13, bytes[record_byte(5006)..record_byte(5008)].to_vec()),
            ]
        );
        let requests = server.requests();
        assert_eq!(requests.len(), 4, "two runs 16 bytes apart are one request");
        assert_eq!(requests[3].header("range"), Some(wanted.as_str()));

        let mut far = Vec::new();
        catalog
            .visit_pixel_runs(&[9..10, 11..12], |pixel, records| {
                far.push((pixel, records.len()));
                Ok(())
            })
            .unwrap();
        assert_eq!(far, vec![(9, 32), (11, 48)]);
        assert_eq!(
            server.requests().len(),
            6,
            "runs 80,000 bytes apart are read separately"
        );

        let ignored = serve(Resource::new(bytes.clone()).ignore_range());
        let error = http_error(
            HealpixCatalog::open_http(&ignored.base, "level2.dat")
                .err()
                .unwrap(),
        );
        assert!(
            matches!(error, HttpRangeError::Status { status: 200, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("200"), "{error}");

        let unsatisfiable = serve(Resource::new(bytes).answer_416());
        let error = http_error(
            HealpixCatalog::open_http(&unsatisfiable.base, "level2.dat")
                .err()
                .unwrap(),
        );
        assert!(
            matches!(error, HttpRangeError::NotSatisfiable { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("416"), "{error}");
    }

    #[test]
    fn range_source_refuses_a_header_that_changed_between_requests() {
        let original = level2_catalogue();
        let mut changed = original.clone();
        changed[record_byte(5002)] ^= 0x01;

        let header_then_change =
            serve(Resource::new(original.clone()).change_etag_after(1, changed.clone()));
        let error = http_error(
            HealpixCatalog::open_http(&header_then_change.base, "level2.dat")
                .err()
                .unwrap(),
        );
        assert_eq!(
            error,
            HttpRangeError::Changed {
                url: format!("{}/level2.dat", header_then_change.base),
                what: "ETag",
                before: "\"ab-v0\"".to_owned(),
                after: "\"ab-v1\"".to_owned(),
            }
        );

        let records_then_change =
            serve(Resource::new(original.clone()).change_etag_after(2, changed.clone()));
        let catalog = HealpixCatalog::open_http(&records_then_change.base, "level2.dat").unwrap();
        let error = http_error(catalog.read_records_raw(11..14).unwrap_err());
        assert!(
            matches!(&error, HttpRangeError::Changed { what: "ETag", .. }),
            "{error}"
        );
        assert!(error.to_string().contains("ab-v1"), "{error}");

        let dated = serve(
            Resource::new(original)
                .no_etag()
                .change_etag_after(2, changed),
        );
        let catalog = HealpixCatalog::open_http(&dated.base, "level2.dat").unwrap();
        let error = http_error(catalog.read_records_raw(11..14).unwrap_err());
        assert_eq!(
            error,
            HttpRangeError::Changed {
                url: format!("{}/level2.dat", dated.base),
                what: "Last-Modified",
                before: LAST_MODIFIED[0].to_owned(),
                after: LAST_MODIFIED[1].to_owned(),
            }
        );
        assert_eq!(
            dated.requests()[2].header("if-range"),
            Some(LAST_MODIFIED[0])
        );
    }

    #[test]
    fn range_reads_split_at_the_read_limit_and_runs_do_not_coalesce_past_it() {
        let xp_len = 701;
        let big = MAX_READ_BYTES / xp_len + 100;
        let raw = |count: usize, salt: usize| -> Vec<Record> {
            (0..count)
                .map(|i| Record::Raw((0..xp_len).map(|b| ((b + i + salt) % 251) as u8).collect()))
                .collect()
        };
        let bytes = written(
            &CatHeader::whole("xp", 2, 2),
            &[(10, raw(big, 0)), (11, raw(3, 7))],
        );
        let server = serve(Resource::new(bytes.clone()));
        let catalog = HealpixCatalog::open_http(&server.base, "xp.dat").unwrap();
        let pixel10 = DATA_START..DATA_START + big * xp_len;
        let pixel11 = pixel10.end..pixel10.end + 3 * xp_len;
        let pixel10_requests = [
            format!(
                "bytes={}-{}",
                pixel10.start,
                pixel10.start + MAX_READ_BYTES - 1
            ),
            format!(
                "bytes={}-{}",
                pixel10.start + MAX_READ_BYTES,
                pixel10.end - 1
            ),
        ];
        let ranges_after = |opened: usize| -> Vec<String> {
            server
                .requests()
                .iter()
                .skip(opened)
                .map(|request| request.header("range").unwrap_or_default().to_owned())
                .collect()
        };

        let raw_read = catalog.read_records_raw(10..11).unwrap();

        assert!(raw_read == bytes[pixel10.clone()]);
        assert_eq!(ranges_after(2), pixel10_requests.to_vec());

        let mut seen = Vec::new();
        catalog
            .visit_pixel_runs(&[10..11, 11..12], |pixel, records| {
                let written_bytes = match pixel {
                    10 => &bytes[pixel10.clone()],
                    _ => &bytes[pixel11.clone()],
                };
                seen.push((pixel, records.len(), records == written_bytes));
                Ok(())
            })
            .unwrap();

        assert_eq!(
            seen,
            vec![(10, pixel10.len(), true), (11, pixel11.len(), true)]
        );
        let mut expected = pixel10_requests.to_vec();
        expected.push(format!("bytes={}-{}", pixel11.start, pixel11.end - 1));
        assert_eq!(ranges_after(4), expected);
    }
    fn answer_once(response: Vec<u8>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 2048];
                let _ = std::io::Read::read(&mut stream, &mut request);
                let _ = std::io::Write::write_all(&mut stream, &response);
            }
        });
        format!("http://{address}/oversized.dat")
    }

    #[test]
    fn a_header_answer_wider_than_the_request_is_refused_before_its_body_is_read() {
        let body = vec![7u8; 256];
        let mut response = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-255/1000\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(&body);
        let url = answer_once(response);

        let error = match HttpRangeSource::open(&url) {
            Ok(_) => panic!("a 256-byte answer to a 128-byte header request was accepted"),
            Err(error) => error,
        };
        assert!(
            matches!(error, HttpRangeError::ContentRange { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("0-255"), "{error}");
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_range_catalogue_opened_and_read_on_an_async_worker_neither_panics_nor_blocks_it() {
        let bytes = level2_catalogue();
        let server = serve(Resource::new(bytes.clone()));
        let catalog = HealpixCatalog::open_http(&server.base, "level2.dat").unwrap();
        let run = catalog.read_records_raw(11..14).unwrap();
        assert_eq!(run, bytes[record_byte(5002)..record_byte(5008)]);
        drop(catalog);
        assert_eq!(server.requests().len(), 3);
    }
}
