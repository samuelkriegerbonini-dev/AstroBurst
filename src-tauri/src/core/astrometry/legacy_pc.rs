use std::borrow::Cow;

use crate::types::header::HduHeader;

pub const LEGACY_PC_CARDS: [(&str, &str); 4] =
    [("PC001001", "PC1_1"), ("PC001002", "PC1_2"), ("PC002001", "PC2_1"), ("PC002002", "PC2_2")];
pub const MODERN_MATRIX_CARDS: [&str; 8] = ["PC1_1", "PC1_2", "PC2_1", "PC2_2", "CD1_1", "CD1_2", "CD2_1", "CD2_2"];

pub fn has_legacy_matrix(header: &HduHeader) -> bool {
    MODERN_MATRIX_CARDS.iter().all(|key| header.get(key).is_none())
        && LEGACY_PC_CARDS.iter().any(|(old, _)| header.get(old).is_some())
}

pub fn with_modern_matrix_cards(header: &HduHeader) -> Cow<'_, HduHeader> {
    if !has_legacy_matrix(header) {
        return Cow::Borrowed(header);
    }
    let mut modern = header.clone();
    for (old, new) in LEGACY_PC_CARDS {
        if let Some(value) = header.get(old) {
            modern.set(new, value.to_string());
        }
    }
    Cow::Owned(modern)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use crate::core::astrometry::wcs::WcsTransform;
    use crate::core::imaging::region::test_support::make_header;
    use crate::infra::wcs_source::{header_wcs, load_wcs};

    const PS1_CARDS: [(&str, &str); 16] = [
        ("NAXIS", "2"), ("NAXIS1", "480"), ("NAXIS2", "480"),
        ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"),
        ("CRVAL1", "54.6835441588982"), ("CRVAL2", "-29.999999999999"),
        ("CRPIX1", "-19507.5"), ("CRPIX2", "-27002.5"),
        ("CDELT1", "6.94444461259988E-05"), ("CDELT2", "6.94444461259988E-05"),
        ("PC001001", "-1."), ("PC001002", "0."), ("PC002001", "0."), ("PC002002", "1."),
        ("RADESYS", "FK5"),
    ];
    const STAR: (f64, f64) = (53.13872873150106, -28.085398991615556);
    const STAR_PIXEL: (f64, f64) = (131.42345394406948, 452.29437563565443);

    const ROTATED_CARDS: [(&str, &str); 14] = [
        ("NAXIS1", "200"), ("NAXIS2", "200"),
        ("CTYPE1", "RA---TAN"), ("CTYPE2", "DEC--TAN"),
        ("CRVAL1", "150.0"), ("CRVAL2", "2.0"),
        ("CRPIX1", "100.5"), ("CRPIX2", "100.5"),
        ("CDELT1", "-2.0E-05"), ("CDELT2", "2.0E-05"),
        ("PC001001", "0.8660254037844386"), ("PC001002", "-0.5"), ("PC002001", "0.5"), ("PC002002", "0.8660254037844386"),
    ];

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        make_header(cards)
    }

    fn is_legacy(key: &str) -> bool {
        LEGACY_PC_CARDS.iter().any(|(old, _)| *old == key)
    }

    fn ps1_without(dropped: &[&str]) -> HduHeader {
        let kept: Vec<(&str, &str)> = PS1_CARDS.iter().copied().filter(|(key, _)| !dropped.contains(key)).collect();
        header(&kept)
    }

    fn ps1_with(extra: (&'static str, &'static str)) -> HduHeader {
        let mut cards = PS1_CARDS.to_vec();
        cards.push(extra);
        header(&cards)
    }

    fn is_borrowed_from(cow: &Cow<'_, HduHeader>, input: &HduHeader) -> bool {
        matches!(cow, Cow::Borrowed(returned) if std::ptr::eq(*returned, input))
    }

    fn assert_within(actual: (f64, f64), expected: (f64, f64), tolerance: f64, what: &str) {
        assert!(
            (actual.0 - expected.0).abs() < tolerance && (actual.1 - expected.1).abs() < tolerance,
            "{what}: {actual:?} vs {expected:?} (tolerance {tolerance})"
        );
    }

    fn matrix_bits(matrix: [[f64; 2]; 2]) -> [u64; 4] {
        [matrix[0][0].to_bits(), matrix[0][1].to_bits(), matrix[1][0].to_bits(), matrix[1][1].to_bits()]
    }

    #[test]
    fn old_style_pc_cards_are_copied_to_the_modern_names() {
        let input = header(&PS1_CARDS);
        assert!(has_legacy_matrix(&input));
        let modern = with_modern_matrix_cards(&input);
        assert!(matches!(modern, Cow::Owned(_)));
        assert_eq!(modern.get("PC1_1"), Some("-1."));
        assert_eq!(modern.get("PC1_2"), Some("0."));
        assert_eq!(modern.get("PC2_1"), Some("0."));
        assert_eq!(modern.get("PC2_2"), Some("1."));
        assert_eq!(modern.get("PC001001"), Some("-1."));
        assert_eq!(modern.get("CDELT1"), Some("6.94444461259988E-05"));
        assert_eq!(modern.get("CRPIX1"), Some("-19507.5"));
        assert_eq!(input.get("PC1_1"), None);
    }

    #[test]
    fn a_header_with_a_modern_matrix_is_returned_as_it_is() {
        for extra in [("PC1_1", "1."), ("CD2_1", "0.")] {
            let input = ps1_with(extra);
            assert!(!has_legacy_matrix(&input), "{extra:?}");
            let modern = with_modern_matrix_cards(&input);
            assert!(is_borrowed_from(&modern, &input), "{extra:?}");
        }
    }

    #[test]
    fn a_header_without_old_style_cards_is_borrowed() {
        let plain: Vec<(&str, &str)> = PS1_CARDS.iter().copied().filter(|(key, _)| !is_legacy(key)).collect();
        assert_eq!(plain.len(), 12);
        let input = header(&plain);
        assert!(!has_legacy_matrix(&input));
        assert!(is_borrowed_from(&with_modern_matrix_cards(&input), &input));
        let empty = HduHeader::empty();
        assert!(is_borrowed_from(&with_modern_matrix_cards(&empty), &empty));
    }

    #[test]
    fn missing_old_style_cards_are_not_invented() {
        let input = ps1_without(&["PC001002", "PC002001"]);
        let modern = with_modern_matrix_cards(&input);
        assert_eq!(modern.get("PC1_1"), Some("-1."));
        assert_eq!(modern.get("PC2_2"), Some("1."));
        assert_eq!(modern.get("PC1_2"), None);
        assert_eq!(modern.get("PC2_1"), None);

        let only_first = ps1_without(&["PC001002", "PC002001", "PC002002"]);
        let modern = with_modern_matrix_cards(&only_first);
        assert_eq!(modern.get("PC1_1"), Some("-1."));
        assert_eq!(modern.cards.len(), only_first.cards.len() + 1);
        assert_eq!(modern.index.len(), only_first.index.len() + 1);
    }

    #[test]
    fn a_pan_starrs_cutout_header_places_the_catalogue_star_where_astropy_does() {
        let wcs = header_wcs(&header(&PS1_CARDS)).unwrap();
        let (x, y) = wcs.world_to_pixel(STAR.0, STAR.1);
        assert!((x - STAR_PIXEL.0).abs() < 1e-3, "x = {x}, astropy {}", STAR_PIXEL.0);
        assert!((y - STAR_PIXEL.1).abs() < 1e-3, "y = {y}, astropy {}", STAR_PIXEL.1);
    }

    #[test]
    fn the_matrix_of_the_cutout_is_cdelt_times_the_old_style_pc() {
        let wcs = header_wcs(&header(&PS1_CARDS)).unwrap();
        assert_eq!(wcs.raw_params().4, [[-6.94444461259988e-05, 0.0], [0.0, 6.94444461259988e-05]]);
        assert!((wcs.pixel_scale_arcsec() - 0.2500000060535957).abs() < 1e-12, "{}", wcs.pixel_scale_arcsec());
    }

    #[test]
    fn pixels_of_the_cutout_map_to_the_sky_positions_of_the_oracle() {
        let wcs = header_wcs(&header(&PS1_CARDS)).unwrap();
        let centre = wcs.pixel_to_world(239.5, 239.5);
        assert_within((centre.ra, centre.dec), (53.130005058601405, -28.10005914658242), 1e-9, "centre");
        let origin = wcs.pixel_to_world(0.0, 0.0);
        assert_within((origin.ra, origin.dec), (53.148584702742546, -28.11687793605545), 1e-9, "origin");
        assert!(wcs.pixel_to_world(479.0, 0.0).ra < origin.ra, "RA decreases with x");
    }

    #[test]
    fn load_wcs_reads_the_old_style_matrix_of_a_file_and_agrees_with_the_modern_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ps1_cutout_16px.fits");
        let mut cards: Vec<(&'static str, String)> =
            vec![("SIMPLE", "T".into()), ("BITPIX", "-32".into()), ("NAXIS", "2".into()), ("NAXIS1", "16".into()), ("NAXIS2", "16".into())];
        for &(key, value) in PS1_CARDS.iter().skip_while(|(key, _)| *key != "CTYPE1") {
            let text = match key {
                "CTYPE1" | "CTYPE2" | "RADESYS" => format!("'{value}'"),
                _ => value.to_string(),
            };
            cards.push((key, text));
        }
        assert_eq!(cards.len(), 18);
        crate::infra::fits::reader::test_fixtures::write_raw_hdus(&path, &[(cards, vec![0u8; 16 * 16 * 4])]);
        let path = path.to_string_lossy().into_owned();
        let read = crate::infra::fits::reader::read_primary_header(&path).unwrap();
        let from_file = load_wcs(&path, &read).unwrap().world_to_pixel(STAR.0, STAR.1);
        assert_within(from_file, STAR_PIXEL, 1e-3, "load_wcs of the written file");

        let modern_cards: Vec<(&str, &str)> = PS1_CARDS
            .iter()
            .map(|&(key, value)| {
                let renamed = LEGACY_PC_CARDS.iter().find(|(old, _)| *old == key).map_or(key, |(_, new)| *new);
                (renamed, value)
            })
            .collect();
        let old_style = header_wcs(&header(&PS1_CARDS)).unwrap();
        let modern = header_wcs(&header(&modern_cards)).unwrap();
        assert_eq!(matrix_bits(old_style.raw_params().4), matrix_bits(modern.raw_params().4));
        let (old_x, old_y) = old_style.world_to_pixel(STAR.0, STAR.1);
        let (new_x, new_y) = modern.world_to_pixel(STAR.0, STAR.1);
        assert_eq!((old_x.to_bits(), old_y.to_bits()), (new_x.to_bits(), new_y.to_bits()));
    }

    #[test]
    fn a_rotated_old_style_matrix_is_read_like_its_modern_spelling() {
        let wcs = header_wcs(&header(&ROTATED_CARDS)).unwrap();
        let expected = [[-1.7320508075688774e-05, 1.0e-05], [1.0e-05, 1.7320508075688774e-05]];
        let cd = wcs.raw_params().4;
        for row in 0..2 {
            for col in 0..2 {
                assert!((cd[row][col] - expected[row][col]).abs() < 1e-18, "{cd:?} vs {expected:?}");
            }
        }
        let sky = wcs.pixel_to_world(150.0, 60.0);
        assert_within((sky.ra, sky.dec), (149.99872954055147, 1.999820839439788), 1e-9, "pixel (150, 60)");
    }

    #[test]
    fn a_header_that_reads_through_cdelt_and_crota_today_is_not_touched() {
        let mut cards: Vec<(&str, &str)> = ROTATED_CARDS.iter().copied().filter(|(key, _)| !is_legacy(key)).collect();
        cards.push(("CROTA2", "30.0"));
        let h = header(&cards);
        assert!(is_borrowed_from(&with_modern_matrix_cards(&h), &h));
        let hooked = header_wcs(&h).unwrap().raw_params().4;
        let direct = WcsTransform::from_header(&h).unwrap().raw_params().4;
        assert_eq!(matrix_bits(hooked), matrix_bits(direct));
    }

    #[test]
    #[ignore = "needs ASTROBURST_SCIENCE_DATA_DIR: survey/P1/ps1_A_{g,r,i}_stack_480px.fits"]
    fn real_ps1_p1_stack_cutouts_place_the_catalogue_star_on_its_pixel() {
        let folder = crate::core::astrometry::gwcs::test_support::env_dir("ASTROBURST_SCIENCE_DATA_DIR").join("survey").join("P1");
        for band in ["g", "r", "i"] {
            let path = folder.join(format!("ps1_A_{band}_stack_480px.fits"));
            let loaded = crate::infra::fits::reader::extract_image_mmap(&std::fs::File::open(&path).unwrap()).unwrap();
            assert!(has_legacy_matrix(&loaded.header), "{band}: the reader no longer carries the PC001001 cards");
            let wcs = load_wcs(&path.to_string_lossy(), &loaded.header).unwrap();
            let star = wcs.world_to_pixel(STAR.0, STAR.1);
            println!("{band}: star at ({:.6}, {:.6}) px, pixel scale {:.13} arcsec", star.0, star.1, wcs.pixel_scale_arcsec());
            assert_within(star, STAR_PIXEL, 0.01, band);
            assert!((wcs.pixel_scale_arcsec() - 0.2500000060535957).abs() < 1e-9, "{band}: {}", wcs.pixel_scale_arcsec());
            if band == "r" {
                let (peak_row, peak_col, peak) = loaded
                    .image
                    .indexed_iter()
                    .filter(|(_, value)| value.is_finite())
                    .fold((0, 0, f32::NEG_INFINITY), |best, ((row, col), &value)| if value > best.2 { (row, col, value) } else { best });
                let distance = (peak_col as f64 - star.0).hypot(peak_row as f64 - star.1);
                println!("r: peak {peak} at row {peak_row}, column {peak_col}, {distance:.3} px from the star");
                assert_eq!((peak_row, peak_col), (453, 133));
                assert!((peak as f64 - 14411.896).abs() < 0.01, "{peak}");
                assert!(distance < 2.0, "{distance}");
            }
        }
    }
}
