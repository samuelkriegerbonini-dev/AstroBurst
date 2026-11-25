use ndarray::Array2;

use crate::types::header::HduHeader;

pub const ASINH_MAGNITUDE_SCALE: f64 = 2.5 / std::f64::consts::LN_10;
pub const BSOFTEN_CARD: &str = "BSOFTEN";
pub const BOFFSET_CARD: &str = "BOFFSET";
pub const LINEARISED_SOFTEN_CARD: &str = "ABSOFTEN";
pub const LINEARISED_OFFSET_CARD: &str = "ABOFFSET";
pub const ASINH_LINEARISED_WARNING: &str = "PS1 asinh-scaled pixels were linearised (BOFFSET, BSOFTEN)";

const REFUSAL_CONSEQUENCE: &str =
    "asinh-scaled pixels cannot be linearised, so the image is refused instead of being read as linear";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsinhScale {
    pub boffset: f64,
    pub bsoften: f64,
}

fn finite_card(header: &HduHeader, key: &str) -> Option<f64> {
    header.get_f64(key).filter(|value| value.is_finite())
}

pub fn asinh_scale(header: &HduHeader) -> Option<AsinhScale> {
    let bsoften = finite_card(header, BSOFTEN_CARD)?;
    let boffset = finite_card(header, BOFFSET_CARD)?;
    (bsoften != 0.0).then_some(AsinhScale { boffset, bsoften })
}

pub fn asinh_refusal(header: &HduHeader) -> Option<String> {
    if asinh_scale(header).is_some() {
        return None;
    }
    let problem = match (header.get(BSOFTEN_CARD), header.get(BOFFSET_CARD)) {
        (None, None) => return None,
        (Some(_), None) => format!("the header has {BSOFTEN_CARD} but no {BOFFSET_CARD}"),
        (None, Some(_)) => format!("the header has {BOFFSET_CARD} but no {BSOFTEN_CARD}"),
        (Some(soften), Some(offset)) => {
            let not_finite = [(BSOFTEN_CARD, soften), (BOFFSET_CARD, offset)]
                .into_iter()
                .find(|(key, _)| finite_card(header, key).is_none());
            match not_finite {
                Some((key, raw)) => format!("{key} = '{}' is not a finite number", raw.trim()),
                None => format!("{BSOFTEN_CARD} = 0"),
            }
        }
    };
    Some(format!("{problem}: {REFUSAL_CONSEQUENCE}"))
}

pub fn linearise(value: f32, scale: &AsinhScale) -> f32 {
    let exponent = f64::from(value) / ASINH_MAGNITUDE_SCALE;
    (scale.boffset + scale.bsoften * (exponent.exp() - (-exponent).exp())) as f32
}

pub fn linearise_plane(plane: &mut Array2<f32>, scale: &AsinhScale) {
    plane.par_mapv_inplace(|value| linearise(value, scale));
}

pub fn mark_linearised(header: &mut HduHeader, scale: &AsinhScale) {
    header.remove(BSOFTEN_CARD);
    header.remove(BOFFSET_CARD);
    header.set_f64(LINEARISED_SOFTEN_CARD, scale.bsoften);
    header.set_f64(LINEARISED_OFFSET_CARD, scale.boffset);
}

pub fn linearised_warning(header: &HduHeader) -> Option<&'static str> {
    let marked = header.get(LINEARISED_SOFTEN_CARD).is_some() && header.get(LINEARISED_OFFSET_CARD).is_some();
    marked.then_some(ASINH_LINEARISED_WARNING)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFFSET_AND_SOFTENING: AsinhScale = AsinhScale { boffset: 3.5, bsoften: 0.25 };
    const FORMULA_TOLERANCE: f64 = 1e-6;
    const ROUND_TRIP_TOLERANCE: f64 = 5e-7;

    fn header(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (key, value) in cards {
            header.set(key, value.to_string());
        }
        header
    }

    fn relative_difference(actual: f64, expected: f64) -> f64 {
        ((actual - expected) / expected).abs()
    }

    #[test]
    fn linearise_follows_the_ps1_formula() {
        assert!((ASINH_MAGNITUDE_SCALE - 1.0857362047581294).abs() < 1e-15);
        let expected = [
            (0.0f32, 3.5f64),
            (2.5, 5.975),
            (5.0, 28.4975),
            (-5.0, -21.4975),
            (10.0, 2503.499975),
            (1.0, 4.028444815239021),
        ];
        for (stored, linear) in expected {
            let found = f64::from(linearise(stored, &OFFSET_AND_SOFTENING));
            assert!(
                relative_difference(found, linear) < FORMULA_TOLERANCE,
                "stored {stored}: {found} against {linear}"
            );
        }
    }

    #[test]
    fn linearise_is_odd_about_the_offset_and_keeps_nan() {
        let offset = OFFSET_AND_SOFTENING.boffset;
        for stored in [0.125f32, 1.0, 7.25] {
            let above = f64::from(linearise(stored, &OFFSET_AND_SOFTENING)) - offset;
            let below = f64::from(linearise(-stored, &OFFSET_AND_SOFTENING)) - offset;
            assert!(above > 0.0, "stored {stored}: {above}");
            assert!(
                relative_difference(-below, above) < FORMULA_TOLERANCE,
                "stored {stored}: {above} against {below}"
            );
        }
        assert!(linearise(f32::NAN, &OFFSET_AND_SOFTENING).is_nan());
        assert_eq!(linearise(1000.0, &OFFSET_AND_SOFTENING), f32::INFINITY);
    }

    #[test]
    fn the_inverse_transform_round_trips() {
        let scale = AsinhScale { boffset: 2.0, bsoften: 100.0 };
        let oracle = [
            (2.0f64, 0.0f64),
            (102.0, 0.5224691006249468),
            (5000.0, 4.247425010840046),
            (-300.0, -1.3032063969279994),
        ];
        for (linear, oracle_stored) in oracle {
            let stored = ASINH_MAGNITUDE_SCALE * ((linear - scale.boffset) / (2.0 * scale.bsoften)).asinh();
            assert!((stored - oracle_stored).abs() < 1e-12, "linear {linear}: stored {stored}");
            let restored = f64::from(linearise(stored as f32, &scale));
            if linear == scale.boffset {
                assert!((restored - linear).abs() < 1e-6, "linear {linear}: {restored}");
            } else {
                assert!(
                    relative_difference(restored, linear) < ROUND_TRIP_TOLERANCE,
                    "linear {linear}: {restored}"
                );
            }
        }
    }

    #[test]
    fn linearise_plane_applies_the_formula_to_every_pixel() {
        let mut plane = Array2::from_shape_vec((2, 2), vec![0.0f32, 2.5, 5.0, f32::NAN]).unwrap();
        linearise_plane(&mut plane, &OFFSET_AND_SOFTENING);
        let expected = [((0, 0), 3.5f64), ((0, 1), 5.975), ((1, 0), 28.4975)];
        for (pixel, linear) in expected {
            let found = f64::from(plane[pixel]);
            assert!(
                relative_difference(found, linear) < FORMULA_TOLERANCE,
                "pixel {pixel:?}: {found} against {linear}"
            );
        }
        assert!(plane[[1, 1]].is_nan());
    }

    #[test]
    fn the_scale_needs_both_cards_as_finite_numbers() {
        let both = header(&[("BSOFTEN", "0.25"), ("BOFFSET", "3.5")]);
        assert_eq!(asinh_scale(&both), Some(AsinhScale { boffset: 3.5, bsoften: 0.25 }));
        assert_eq!(asinh_refusal(&both), None);

        let neither = header(&[("OBJECT", "M51")]);
        assert_eq!(asinh_scale(&neither), None);
        assert_eq!(asinh_refusal(&neither), None);

        let fortran_exponent = header(&[("BSOFTEN", "2.5D-01"), ("BOFFSET", "3.5")]);
        assert_eq!(asinh_scale(&fortran_exponent).map(|scale| scale.bsoften), Some(0.25));
        assert_eq!(asinh_refusal(&fortran_exponent), None);
    }

    #[test]
    fn an_unusable_pair_is_a_refusal_not_a_linear_image() {
        let cases = [
            (header(&[("BSOFTEN", "0.25")]), "the header has BSOFTEN but no BOFFSET"),
            (header(&[("BOFFSET", "3.5")]), "the header has BOFFSET but no BSOFTEN"),
            (
                header(&[("BSOFTEN", "abc"), ("BOFFSET", "3.5")]),
                "BSOFTEN = 'abc' is not a finite number",
            ),
            (
                header(&[("BSOFTEN", "0.25"), ("BOFFSET", "inf")]),
                "BOFFSET = 'inf' is not a finite number",
            ),
            (header(&[("BSOFTEN", "0"), ("BOFFSET", "3.5")]), "BSOFTEN = 0"),
        ];
        for (unusable, problem) in cases {
            assert_eq!(asinh_scale(&unusable), None, "{problem}");
            assert_eq!(
                asinh_refusal(&unusable),
                Some(format!(
                    "{problem}: asinh-scaled pixels cannot be linearised, so the image is refused instead of being read as linear"
                ))
            );
        }
    }

    #[test]
    fn marking_removes_the_cards_and_records_what_was_applied() {
        let mut marked = header(&[("BSOFTEN", "0.25"), ("OBJECT", "M51"), ("BOFFSET", "3.5")]);
        let scale = asinh_scale(&marked).unwrap();
        assert_eq!(linearised_warning(&marked), None);
        mark_linearised(&mut marked, &scale);

        assert_eq!(marked.get("BSOFTEN"), None);
        assert_eq!(marked.get("BOFFSET"), None);
        assert!(marked.cards.iter().all(|(key, _)| key != "BSOFTEN" && key != "BOFFSET"));
        assert_eq!(marked.get_f64("ABSOFTEN"), Some(0.25));
        assert_eq!(marked.get_f64("ABOFFSET"), Some(3.5));
        assert_eq!(marked.get("OBJECT"), Some("M51"));
        assert_eq!(asinh_scale(&marked), None);
        assert_eq!(asinh_refusal(&marked), None);
        assert_eq!(
            linearised_warning(&marked),
            Some("PS1 asinh-scaled pixels were linearised (BOFFSET, BSOFTEN)")
        );

        assert_eq!(linearised_warning(&header(&[("OBJECT", "M51")])), None);
    }
}
