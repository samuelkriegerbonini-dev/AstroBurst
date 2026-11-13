use ndarray::Array2;

use crate::types::header::HduHeader;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerRole {
    Science,
    Sigma,
    Variance,
    InverseVariance,
    Mask,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LayerGuess {
    pub role: LayerRole,
    pub basis: String,
    pub assumed: bool,
}

pub const NO_DATA_SIGMA: f32 = 1.0e15;

const EXTNAME_ROLES: &[(&str, LayerRole, bool)] = &[
    ("SCI", LayerRole::Science, false),
    ("IMAGE", LayerRole::Science, false),
    ("DATA", LayerRole::Science, false),
    ("ERR", LayerRole::Sigma, false),
    ("RMS", LayerRole::Sigma, false),
    ("VARIANCE", LayerRole::Variance, false),
    ("VAR", LayerRole::Variance, false),
    ("IVAR", LayerRole::InverseVariance, false),
    ("INVVAR", LayerRole::InverseVariance, false),
    ("WGT", LayerRole::InverseVariance, true),
    ("WEIGHT", LayerRole::InverseVariance, true),
    ("DQ", LayerRole::Mask, false),
    ("MASK", LayerRole::Mask, false),
    ("MSK", LayerRole::Mask, false),
    ("FLAG", LayerRole::Mask, false),
    ("MASKBITS", LayerRole::Mask, false),
];

const CARD_ROLES: &[(&str, &str, LayerRole, bool)] = &[
    ("IMAGETYP", "INVVAR", LayerRole::InverseVariance, false),
    ("IMAGETYP", "MASKBITS", LayerRole::Mask, false),
    ("IMAGETYP", "IMAGE", LayerRole::Science, false),
    ("IMTYPE", "INVVAR", LayerRole::InverseVariance, false),
    ("IMTYPE", "MASKBITS", LayerRole::Mask, false),
    ("IMTYPE", "IMAGE", LayerRole::Science, false),
    ("DES_EXT", "WEIGHT", LayerRole::InverseVariance, true),
    ("DES_EXT", "MASK", LayerRole::Mask, false),
    ("DES_EXT", "IMAGE", LayerRole::Science, false),
];

const PS1_VARIANCE_CARD: &str = "PS_COVAR";
const PS1_VARIANCE_BASIS: &str = "PS_COVAR card (PS1 variance layer)";
const PS1_MASK_CARD: &str = "MSKNAM00";
const PS1_MASK_BASIS: &str = "MSKNAM cards (PS1 mask layer)";
const NO_NAME_BASIS: &str = "no layer name or card";
const COMPRESSION_SUFFIXES: &[&str] = &[".gz", ".fz", ".bz2"];
const EUCLID_MOSAIC_PREFIX: &str = "euc_mer_mosaic-";

impl LayerRole {
    pub fn is_error(self) -> bool {
        matches!(self, LayerRole::Sigma | LayerRole::Variance | LayerRole::InverseVariance)
    }

    pub fn name(self) -> &'static str {
        match self {
            LayerRole::Science => "science",
            LayerRole::Sigma => "sigma",
            LayerRole::Variance => "variance",
            LayerRole::InverseVariance => "inverse_variance",
            LayerRole::Mask => "mask",
            LayerRole::Other => "other",
        }
    }

    pub fn conversion(self) -> &'static str {
        match self {
            LayerRole::Sigma => "as is",
            LayerRole::Variance => "square root",
            LayerRole::InverseVariance => "1 / square root",
            LayerRole::Science | LayerRole::Mask | LayerRole::Other => "",
        }
    }
}

fn guess(role: LayerRole, basis: impl Into<String>, assumed: bool) -> LayerGuess {
    LayerGuess { role, basis: basis.into(), assumed }
}

fn named_by(header: &HduHeader, card: &'static str) -> Option<(&'static str, String)> {
    let value = header.get(card)?.trim().to_uppercase();
    let segment = value.rsplit('.').next().unwrap_or(&value);
    (!segment.is_empty()).then(|| (card, segment.to_string()))
}

fn layer_name(header: &HduHeader) -> Option<(&'static str, String)> {
    named_by(header, "EXTNAME").or_else(|| named_by(header, "EXTTYPE"))
}

fn role_from_cards(header: &HduHeader) -> Option<LayerGuess> {
    for (card, value, role, assumed) in CARD_ROLES {
        if header.get(card).is_some_and(|found| found.trim().eq_ignore_ascii_case(value)) {
            return Some(guess(*role, format!("{card} {value}"), *assumed));
        }
    }
    if header.get(PS1_VARIANCE_CARD).is_some() {
        return Some(guess(LayerRole::Variance, PS1_VARIANCE_BASIS, true));
    }
    header.get(PS1_MASK_CARD).map(|_| guess(LayerRole::Mask, PS1_MASK_BASIS, true))
}

fn role_from_file_name(file_name: &str) -> Option<LayerGuess> {
    let lower = file_name.to_lowercase();
    let name = COMPRESSION_SUFFIXES.iter().find_map(|suffix| lower.strip_suffix(suffix)).unwrap_or(&lower);
    let euclid_mosaic = name.starts_with(EUCLID_MOSAIC_PREFIX);
    let (role, basis) = if name.contains("-invvar-") {
        (LayerRole::InverseVariance, "file name -invvar-")
    } else if name.contains("-maskbits") {
        (LayerRole::Mask, "file name -maskbits")
    } else if euclid_mosaic && name.contains("-rms_") {
        (LayerRole::Sigma, "file name -RMS")
    } else if euclid_mosaic && name.contains("-flag_") {
        (LayerRole::Mask, "file name -FLAG")
    } else if name.ends_with(".wt.fits") {
        (LayerRole::Variance, "file name .wt")
    } else if name.ends_with(".mask.fits") {
        (LayerRole::Mask, "file name .mask")
    } else {
        return None;
    };
    Some(guess(role, basis, false))
}

pub fn classify_layer(header: &HduHeader, file_name: &str) -> LayerGuess {
    let name = layer_name(header);
    if let Some((card, name)) = &name {
        if let Some((_, role, assumed)) = EXTNAME_ROLES.iter().find(|(known, _, _)| known == name) {
            return guess(*role, format!("{card} {name}"), *assumed);
        }
    }
    role_from_cards(header)
        .or_else(|| role_from_file_name(file_name))
        .unwrap_or_else(|| match name {
            Some((card, name)) => guess(LayerRole::Other, format!("{card} {name}"), false),
            None => guess(LayerRole::Science, NO_NAME_BASIS, false),
        })
}

fn one_sigma(role: LayerRole, value: f32) -> f32 {
    if !value.is_finite() {
        return f32::NAN;
    }
    let value = f64::from(value);
    let sigma = match role {
        LayerRole::Sigma => value,
        LayerRole::Variance if value >= 0.0 => value.sqrt(),
        LayerRole::InverseVariance if value > 0.0 => 1.0 / value.sqrt(),
        _ => return f32::NAN,
    };
    if sigma < f64::from(NO_DATA_SIGMA) {
        sigma as f32
    } else {
        f32::NAN
    }
}

pub fn sigma_with_no_data(role: LayerRole, values: &Array2<f32>) -> Option<(Array2<f32>, u64)> {
    if !role.is_error() {
        return None;
    }
    let sigma = values.mapv(|value| one_sigma(role, value));
    let no_data = sigma.iter().filter(|value| value.is_nan()).count() as u64;
    Some((sigma, no_data))
}

pub fn sigma_from_layer(role: LayerRole, values: &Array2<f32>) -> Option<Array2<f32>> {
    sigma_with_no_data(role, values).map(|(sigma, _)| sigma)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;

    const LEGACY_INVVAR_FILE: &str = "legacysurvey-0532m280-invvar-r.fits.fz";

    fn header_with(pairs: &[(&str, &str)]) -> HduHeader {
        let mut h = HduHeader::empty();
        for (k, v) in pairs {
            h.set(k, v.to_string());
        }
        h
    }

    fn bits(values: &Array2<f32>) -> Vec<u32> {
        values.iter().map(|v| v.to_bits()).collect()
    }

    #[test]
    fn extnames_map_to_their_roles_and_a_weight_is_an_assumed_inverse_variance() {
        let rows: &[(&str, LayerRole, bool, &str)] = &[
            ("SCI", LayerRole::Science, false, "EXTNAME SCI"),
            ("IMAGE", LayerRole::Science, false, "EXTNAME IMAGE"),
            ("DATA", LayerRole::Science, false, "EXTNAME DATA"),
            ("ERR", LayerRole::Sigma, false, "EXTNAME ERR"),
            ("RMS", LayerRole::Sigma, false, "EXTNAME RMS"),
            ("VARIANCE", LayerRole::Variance, false, "EXTNAME VARIANCE"),
            ("VAR", LayerRole::Variance, false, "EXTNAME VAR"),
            ("IVAR", LayerRole::InverseVariance, false, "EXTNAME IVAR"),
            ("INVVAR", LayerRole::InverseVariance, false, "EXTNAME INVVAR"),
            ("WGT", LayerRole::InverseVariance, true, "EXTNAME WGT"),
            ("WEIGHT", LayerRole::InverseVariance, true, "EXTNAME WEIGHT"),
            ("DQ", LayerRole::Mask, false, "EXTNAME DQ"),
            ("MASK", LayerRole::Mask, false, "EXTNAME MASK"),
            ("MSK", LayerRole::Mask, false, "EXTNAME MSK"),
            ("FLAG", LayerRole::Mask, false, "EXTNAME FLAG"),
            ("MASKBITS", LayerRole::Mask, false, "EXTNAME MASKBITS"),
            (" wgt ", LayerRole::InverseVariance, true, "EXTNAME WGT"),
            ("variance", LayerRole::Variance, false, "EXTNAME VARIANCE"),
            ("roman.err", LayerRole::Sigma, false, "EXTNAME ERR"),
            ("roman.dq", LayerRole::Mask, false, "EXTNAME DQ"),
            ("roman.data", LayerRole::Science, false, "EXTNAME DATA"),
        ];
        for (extname, role, assumed, basis) in rows {
            assert_eq!(
                classify_layer(&header_with(&[("EXTNAME", extname)]), "image.fits"),
                guess(*role, *basis, *assumed),
                "EXTNAME {extname:?}"
            );
        }
    }

    #[test]
    fn drizzle_weights_and_variance_terms_are_never_an_error_layer() {
        for extname in ["WHT", "CTX", "CON", "VAR_POISSON", "VAR_RNOISE", "VAR_FLAT", "AREA", "WAVELENGTH", "ERROR", "DQX"] {
            let found = classify_layer(&header_with(&[("EXTNAME", extname)]), "jw_i2d.fits");
            assert_eq!(found, guess(LayerRole::Other, format!("EXTNAME {extname}"), false));
            assert!(!found.role.is_error(), "{extname}");
        }
    }

    #[test]
    fn cards_name_a_layer_that_has_no_extname() {
        let rows: &[(&[(&str, &str)], LayerRole, bool, &str)] = &[
            (&[("IMAGETYP", "INVVAR")], LayerRole::InverseVariance, false, "IMAGETYP INVVAR"),
            (&[("IMTYPE", "invvar")], LayerRole::InverseVariance, false, "IMTYPE INVVAR"),
            (&[("IMAGETYP", "MASKBITS")], LayerRole::Mask, false, "IMAGETYP MASKBITS"),
            (&[("IMTYPE", "maskbits")], LayerRole::Mask, false, "IMTYPE MASKBITS"),
            (&[("IMAGETYP", "IMAGE")], LayerRole::Science, false, "IMAGETYP IMAGE"),
            (&[("DES_EXT", "WEIGHT")], LayerRole::InverseVariance, true, "DES_EXT WEIGHT"),
            (&[("DES_EXT", "MASK")], LayerRole::Mask, false, "DES_EXT MASK"),
            (&[("DES_EXT", "IMAGE")], LayerRole::Science, false, "DES_EXT IMAGE"),
            (&[("PS_COVAR", "T"), ("MSKNAM00", "DETECTOR")], LayerRole::Variance, true, "PS_COVAR card (PS1 variance layer)"),
            (&[("MSKNAM00", "DETECTOR")], LayerRole::Mask, true, "MSKNAM cards (PS1 mask layer)"),
            (&[("IMAGETYP", "Light Frame")], LayerRole::Science, false, "no layer name or card"),
            (&[("IMAGETYP", "FLAT")], LayerRole::Science, false, "no layer name or card"),
            (&[("IMAGETYP", "object"), ("IMTYPE", "invvar")], LayerRole::InverseVariance, false, "IMTYPE INVVAR"),
        ];
        for (cards, role, assumed, basis) in rows {
            assert_eq!(classify_layer(&header_with(cards), "cutout.fits"), guess(*role, *basis, *assumed), "{cards:?}");
        }
    }

    #[test]
    fn the_extname_wins_over_cards_and_the_file_name_comes_last() {
        let none = HduHeader::empty();
        assert_eq!(
            classify_layer(&header_with(&[("EXTNAME", "SCI"), ("IMTYPE", "invvar")]), LEGACY_INVVAR_FILE),
            guess(LayerRole::Science, "EXTNAME SCI", false)
        );
        assert_eq!(classify_layer(&none, LEGACY_INVVAR_FILE), guess(LayerRole::InverseVariance, "file name -invvar-", false));
        assert_eq!(
            classify_layer(&none, "legacysurvey-0532m280-maskbits.fits.fz"),
            guess(LayerRole::Mask, "file name -maskbits", false)
        );
        assert_eq!(
            classify_layer(&none, "EUC_MER_MOSAIC-VIS-RMS_TILE102044185-8BF2FC_20241021T010718.207983Z_00.00.fits"),
            guess(LayerRole::Sigma, "file name -RMS", false)
        );
        assert_eq!(
            classify_layer(&none, "EUC_MER_MOSAIC-VIS-FLAG_TILE102044185-A61C42_x.fits.gz"),
            guess(LayerRole::Mask, "file name -FLAG", false)
        );
        assert_eq!(
            classify_layer(&none, "rings.v3.skycell.0647.098.stk.r.unconv.wt.fits"),
            guess(LayerRole::Variance, "file name .wt", false)
        );
        assert_eq!(
            classify_layer(&none, "rings.v3.skycell.0647.098.stk.r.unconv.mask.fits"),
            guess(LayerRole::Mask, "file name .mask", false)
        );
        assert_eq!(
            classify_layer(&none, "rings.v3.skycell.0647.098.stk.r.unconv.wt.fits.bz2"),
            guess(LayerRole::Variance, "file name .wt", false)
        );
        for science in ["EUC_MER_BGSUB-MOSAIC-VIS_TILE102044185-3EA2F5_x.fits", "ps1_A_r_stack_480px.fits"] {
            assert_eq!(classify_layer(&none, science), guess(LayerRole::Science, "no layer name or card", false), "{science}");
        }

        assert_eq!(
            classify_layer(&header_with(&[("EXTNAME", "N4"), ("IMAGETYP", "INVVAR")]), "cutout.fits"),
            guess(LayerRole::InverseVariance, "IMAGETYP INVVAR", false),
            "an extension name outside the table goes on to the cards"
        );
        assert_eq!(
            classify_layer(&header_with(&[("EXTNAME", "COMPRESSED_IMAGE")]), LEGACY_INVVAR_FILE),
            guess(LayerRole::InverseVariance, "file name -invvar-", false),
            "an extension name outside the table goes on to the file name"
        );
        assert_eq!(
            classify_layer(&header_with(&[("EXTNAME", "  ")]), "cutout.fits"),
            guess(LayerRole::Science, "no layer name or card", false)
        );
    }

    #[test]
    fn exttype_names_a_layer_that_has_no_extname() {
        for (exttype, role, basis) in [
            ("IMAGE   ", LayerRole::Science, "EXTTYPE IMAGE"),
            ("MASK", LayerRole::Mask, "EXTTYPE MASK"),
            ("VARIANCE", LayerRole::Variance, "EXTTYPE VARIANCE"),
            ("ARCHIVE_DATA", LayerRole::Other, "EXTTYPE ARCHIVE_DATA"),
        ] {
            assert_eq!(classify_layer(&header_with(&[("EXTTYPE", exttype)]), "medexp.fits"), guess(role, basis, false), "{exttype}");
        }
        assert_eq!(
            classify_layer(&header_with(&[("EXTNAME", "IMAGE"), ("EXTTYPE", "MASK")]), "visit_image.fits"),
            guess(LayerRole::Science, "EXTNAME IMAGE", false)
        );
        assert_eq!(
            classify_layer(&header_with(&[("EXTNAME", "N4"), ("EXTTYPE", "VARIANCE")]), "cutout.fits"),
            guess(LayerRole::Other, "EXTNAME N4", false),
            "an extension name outside the table is not replaced by EXTTYPE"
        );
        assert_eq!(
            classify_layer(&header_with(&[("EXTTYPE", " "), ("IMAGETYP", "INVVAR")]), "cutout.fits"),
            guess(LayerRole::InverseVariance, "IMAGETYP INVVAR", false)
        );
    }

    #[test]
    fn a_variance_becomes_its_square_root_and_a_negative_variance_is_no_data() {
        let variance = array![[4.0_f32, 0.0, -1.0], [f32::NAN, 2.25, 1.0e32]];
        let (sigma, no_data) = sigma_with_no_data(LayerRole::Variance, &variance).unwrap();
        assert_eq!(sigma[[0, 0]], 2.0);
        assert_eq!(sigma[[0, 1]], 0.0);
        assert!(sigma[[0, 2]].is_nan());
        assert!(sigma[[1, 0]].is_nan());
        assert_eq!(sigma[[1, 1]], 1.5);
        assert!(sigma[[1, 2]].is_nan(), "sqrt(1e32) = 1e16 is at or above the no-data sentinel");
        assert_eq!(no_data, 3);
    }

    #[test]
    fn an_inverse_variance_becomes_one_over_its_square_root_and_zero_weight_is_no_data() {
        let weight = array![[4.0_f32, 0.25, 0.0], [-3.0, 1941938.0, f32::INFINITY]];
        let (sigma, no_data) = sigma_with_no_data(LayerRole::InverseVariance, &weight).unwrap();
        assert_eq!(sigma[[0, 0]], 0.5);
        assert_eq!(sigma[[0, 1]], 2.0);
        assert!(sigma[[0, 2]].is_nan());
        assert!(sigma[[1, 0]].is_nan());
        let golden = 0.0007175998180759543_f64;
        assert!(
            (f64::from(sigma[[1, 1]]) - golden).abs() <= 1e-6 * golden,
            "1 / sqrt(1941938) = {} against {golden}",
            sigma[[1, 1]]
        );
        assert!(sigma[[1, 2]].is_nan());
        assert_eq!(no_data, 3);
    }

    #[test]
    fn a_sigma_stays_as_it_is_and_the_euclid_sentinel_is_no_data() {
        let rms = array![[0.0041878815_f32, 1.0000000272564224e16, 0.0], [-1.0, f32::NAN, 9.9e14]];
        let (sigma, no_data) = sigma_with_no_data(LayerRole::Sigma, &rms).unwrap();
        for at in [[0, 0], [0, 2], [1, 0], [1, 2]] {
            assert_eq!(sigma[at].to_bits(), rms[at].to_bits(), "{at:?}");
        }
        assert!(sigma[[0, 1]].is_nan());
        assert!(sigma[[1, 1]].is_nan());
        assert_eq!(no_data, 2);
        assert!(f64::from(rms[[1, 2]]) < f64::from(NO_DATA_SIGMA));
    }

    #[test]
    fn science_mask_and_other_layers_have_no_sigma() {
        let values = array![[4.0_f32, 0.0], [-1.0, f32::NAN]];
        for role in [LayerRole::Science, LayerRole::Mask, LayerRole::Other] {
            assert!(sigma_with_no_data(role, &values).is_none(), "{role:?}");
            assert!(sigma_from_layer(role, &values).is_none(), "{role:?}");
            assert!(!role.is_error());
            assert_eq!(role.conversion(), "");
        }
        for (role, name, conversion) in [
            (LayerRole::Sigma, "sigma", "as is"),
            (LayerRole::Variance, "variance", "square root"),
            (LayerRole::InverseVariance, "inverse_variance", "1 / square root"),
        ] {
            assert!(role.is_error());
            assert_eq!(role.name(), name);
            assert_eq!(role.conversion(), conversion);
            let (with_count, _) = sigma_with_no_data(role, &values).unwrap();
            assert_eq!(bits(&sigma_from_layer(role, &values).unwrap()), bits(&with_count), "{role:?}");

            let (empty, none) = sigma_with_no_data(role, &Array2::<f32>::zeros((0, 0))).unwrap();
            assert_eq!(empty.dim(), (0, 0));
            assert_eq!(none, 0);

            let (single, none) = sigma_with_no_data(role, &array![[4.0_f32]]).unwrap();
            assert_eq!(single.dim(), (1, 1));
            assert!(single[[0, 0]] > 0.0);
            assert_eq!(none, 0);
        }
        for role in [LayerRole::Science, LayerRole::Sigma, LayerRole::Variance, LayerRole::InverseVariance, LayerRole::Mask, LayerRole::Other] {
            assert_eq!(serde_json::to_value(role).unwrap(), serde_json::Value::String(role.name().to_string()));
        }
    }
}
