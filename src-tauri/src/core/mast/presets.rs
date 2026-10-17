use super::errors::bad_input;
use super::types::{AppliedEcho, MastCmdError, MastMissionId, MastPreset, MastPresetCondition};

#[derive(Debug, PartialEq, Eq)]
pub struct PresetSpec {
    pub id: &'static str,
    pub mission: MastMissionId,
    pub label: &'static str,
    pub description: &'static str,
    pub conditions: &'static [(&'static str, &'static str)],
    pub products: &'static [&'static str],
}

const JWST_ONLY: &str = "Presets are available for JWST";

pub static PRESETS: [PresetSpec; 7] = [
    PresetSpec {
        id: "tso",
        mission: MastMissionId::Jwst,
        label: "TSO",
        description: "Time-series observations: NIRSpec bright object, NIRISS SOSS, MIRI slitless LRS, NIRCam grism",
        conditions: &[
            ("tsovisit", "t"),
            ("exp_type", "NRS_BRIGHTOBJ,NIS_SOSS,MIR_LRS-SLITLESS,NRC_TSGRISM"),
        ],
        products: &["x1dints", "calints"],
    },
    PresetSpec {
        id: "ifu_cubes",
        mission: MastMissionId::Jwst,
        label: "IFU cubes",
        description: "NIRSpec IFU and MIRI MRS combined cubes",
        conditions: &[("exp_type", "NRS_IFU,MIR_MRS"), ("productLevel", "3")],
        products: &["s3d", "x1d"],
    },
    PresetSpec {
        id: "slit_spectra",
        mission: MastMissionId::Jwst,
        label: "Slit spectra",
        description: "NIRSpec fixed slit and MOS, MIRI LRS slit",
        conditions: &[("exp_type", "NRS_FIXEDSLIT,NRS_MSASPEC,MIR_LRS-FIXEDSLIT")],
        products: &["s2d", "x1d"],
    },
    PresetSpec {
        id: "spectra_1d",
        mission: MastMissionId::Jwst,
        label: "1D spectra",
        description: "Extracted 1D spectra of any mode",
        conditions: &[],
        products: &["x1d"],
    },
    PresetSpec {
        id: "psf_calibration",
        mission: MastMissionId::Jwst,
        label: "PSF calibration",
        description: "Observations flagged as PSF references",
        conditions: &[("is_psf", "t")],
        products: &["cal", "i2d"],
    },
    PresetSpec {
        id: "pure_parallel",
        mission: MastMissionId::Jwst,
        label: "Pure parallel",
        description: "Pure-parallel visits",
        conditions: &[("visitype", "PARALLEL_PURE")],
        products: &["cal"],
    },
    PresetSpec {
        id: "pipeline_catalogs",
        mission: MastMissionId::Jwst,
        label: "Pipeline catalogues",
        description: "Source catalogues and segmentation maps of combined products",
        conditions: &[("productLevel", "3")],
        products: &["cat", "segm"],
    },
];

impl PresetSpec {
    fn to_contract(&self) -> MastPreset {
        MastPreset {
            id: self.id.to_owned(),
            mission: self.mission,
            label: self.label.to_owned(),
            description: self.description.to_owned(),
            conditions: self
                .conditions
                .iter()
                .map(|(column, value)| MastPresetCondition {
                    column: (*column).to_owned(),
                    value: (*value).to_owned(),
                })
                .collect(),
            products: self
                .products
                .iter()
                .map(|suffix| (*suffix).to_owned())
                .collect(),
        }
    }
}

pub fn table() -> Vec<MastPreset> {
    PRESETS.iter().map(PresetSpec::to_contract).collect()
}

fn known_ids(mission: MastMissionId) -> String {
    let ids: Vec<&str> = PRESETS
        .iter()
        .filter(|preset| preset.mission == mission)
        .map(|preset| preset.id)
        .collect();
    match ids.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_owned(),
        None => String::new(),
    }
}

pub fn find(mission: MastMissionId, id: &str) -> Result<&'static PresetSpec, MastCmdError> {
    if !PRESETS.iter().any(|preset| preset.mission == mission) {
        return Err(bad_input("preset", JWST_ONLY));
    }
    PRESETS
        .iter()
        .find(|preset| preset.mission == mission && preset.id == id)
        .ok_or_else(|| {
            bad_input(
                "preset",
                &format!("unknown preset; use {}", known_ids(mission)),
            )
        })
}

fn echoed_values(applied: &AppliedEcho, column: &str) -> Vec<String> {
    applied
        .conditions
        .iter()
        .filter_map(|condition| condition.get(column))
        .flat_map(|text| text.split(','))
        .map(|part| part.trim().to_ascii_lowercase())
        .collect()
}

pub fn dropped_conditions(preset: &PresetSpec, applied: &AppliedEcho) -> Vec<String> {
    preset
        .conditions
        .iter()
        .filter(|(column, value)| {
            let echoed = echoed_values(applied, column);
            !value
                .split(',')
                .all(|part| echoed.contains(&part.trim().to_ascii_lowercase()))
        })
        .map(|(column, _)| (*column).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::MastErrorCode;
    use serde_json::json;
    use std::collections::BTreeMap;

    const TSO_TYPES: &str = "NRS_BRIGHTOBJ,NIS_SOSS,MIR_LRS-SLITLESS,NRC_TSGRISM";

    fn echo(conditions: &[(&str, &str)]) -> AppliedEcho {
        AppliedEcho {
            conditions: conditions
                .iter()
                .map(|(column, value)| {
                    BTreeMap::from([((*column).to_owned(), (*value).to_owned())])
                })
                .collect(),
            sort_by: Vec::new(),
            sort_desc: Vec::new(),
        }
    }

    #[test]
    fn the_table_holds_seven_jwst_presets_in_the_contract_shape() {
        let table = serde_json::to_value(table()).unwrap();
        assert_eq!(
            table,
            json!([
                {
                    "id": "tso",
                    "mission": "jwst",
                    "label": "TSO",
                    "description": "Time-series observations: NIRSpec bright object, NIRISS SOSS, MIRI slitless LRS, NIRCam grism",
                    "conditions": [{"column": "tsovisit", "value": "t"}, {"column": "exp_type", "value": TSO_TYPES}],
                    "products": ["x1dints", "calints"]
                },
                {
                    "id": "ifu_cubes",
                    "mission": "jwst",
                    "label": "IFU cubes",
                    "description": "NIRSpec IFU and MIRI MRS combined cubes",
                    "conditions": [{"column": "exp_type", "value": "NRS_IFU,MIR_MRS"}, {"column": "productLevel", "value": "3"}],
                    "products": ["s3d", "x1d"]
                },
                {
                    "id": "slit_spectra",
                    "mission": "jwst",
                    "label": "Slit spectra",
                    "description": "NIRSpec fixed slit and MOS, MIRI LRS slit",
                    "conditions": [{"column": "exp_type", "value": "NRS_FIXEDSLIT,NRS_MSASPEC,MIR_LRS-FIXEDSLIT"}],
                    "products": ["s2d", "x1d"]
                },
                {
                    "id": "spectra_1d",
                    "mission": "jwst",
                    "label": "1D spectra",
                    "description": "Extracted 1D spectra of any mode",
                    "conditions": [],
                    "products": ["x1d"]
                },
                {
                    "id": "psf_calibration",
                    "mission": "jwst",
                    "label": "PSF calibration",
                    "description": "Observations flagged as PSF references",
                    "conditions": [{"column": "is_psf", "value": "t"}],
                    "products": ["cal", "i2d"]
                },
                {
                    "id": "pure_parallel",
                    "mission": "jwst",
                    "label": "Pure parallel",
                    "description": "Pure-parallel visits",
                    "conditions": [{"column": "visitype", "value": "PARALLEL_PURE"}],
                    "products": ["cal"]
                },
                {
                    "id": "pipeline_catalogs",
                    "mission": "jwst",
                    "label": "Pipeline catalogues",
                    "description": "Source catalogues and segmentation maps of combined products",
                    "conditions": [{"column": "productLevel", "value": "3"}],
                    "products": ["cat", "segm"]
                }
            ])
        );
        let ids: Vec<&str> = PRESETS.iter().map(|preset| preset.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
        assert!(ids.iter().all(|id| id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')));
    }

    #[test]
    fn presets_resolve_for_their_mission_only() {
        assert_eq!(find(MastMissionId::Jwst, "tso").unwrap().label, "TSO");
        assert_eq!(
            find(MastMissionId::Jwst, "pipeline_catalogs")
                .unwrap()
                .products,
            ["cat", "segm"]
        );
        for (mission, id, reason) in [
            (MastMissionId::Jwst, "TSO", "preset: unknown preset; use tso, ifu_cubes, slit_spectra, spectra_1d, psf_calibration, pure_parallel or pipeline_catalogs"),
            (MastMissionId::Jwst, "../tso", "preset: unknown preset; use tso, ifu_cubes, slit_spectra, spectra_1d, psf_calibration, pure_parallel or pipeline_catalogs"),
            (MastMissionId::Hst, "tso", "preset: Presets are available for JWST"),
            (MastMissionId::Roman, "ifu_cubes", "preset: Presets are available for JWST"),
        ] {
            let error = find(mission, id).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{id}");
            assert_eq!(error.message, reason, "{mission:?} {id}");
        }
    }

    #[test]
    fn the_echo_check_names_conditions_mast_dropped() {
        let tso = find(MastMissionId::Jwst, "tso").unwrap();
        let full = echo(&[
            ("productLevel", "3"),
            ("tsovisit", "t"),
            ("exp_type", TSO_TYPES),
        ]);
        assert_eq!(dropped_conditions(tso, &full), Vec::<String>::new());
        let upper = echo(&[
            ("tsovisit", "T"),
            ("exp_type", &TSO_TYPES.to_ascii_lowercase()),
        ]);
        assert_eq!(dropped_conditions(tso, &upper), Vec::<String>::new());
        let without_flag = echo(&[("productLevel", "3"), ("exp_type", TSO_TYPES)]);
        assert_eq!(dropped_conditions(tso, &without_flag), ["tsovisit"]);
        let short_list = echo(&[("tsovisit", "t"), ("exp_type", "NRS_BRIGHTOBJ,NIS_SOSS")]);
        assert_eq!(dropped_conditions(tso, &short_list), ["exp_type"]);
        let merged = echo(&[
            ("tsovisit", "t"),
            ("exp_type", &format!("NRC_IMAGE,{TSO_TYPES}")),
        ]);
        assert_eq!(dropped_conditions(tso, &merged), Vec::<String>::new());
        assert_eq!(
            dropped_conditions(tso, &echo(&[])),
            ["tsovisit", "exp_type"]
        );
        let ifu = find(MastMissionId::Jwst, "ifu_cubes").unwrap();
        let level_merged = echo(&[("productLevel", "3,3"), ("exp_type", "NRS_IFU,MIR_MRS")]);
        assert_eq!(dropped_conditions(ifu, &level_merged), Vec::<String>::new());
        let one_d = find(MastMissionId::Jwst, "spectra_1d").unwrap();
        assert_eq!(dropped_conditions(one_d, &echo(&[])), Vec::<String>::new());
    }
}
