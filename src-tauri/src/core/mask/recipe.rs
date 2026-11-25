use ndarray::Array2;
use serde::{Deserialize, Serialize};

use crate::core::history::id::Fnv1a;
use crate::core::imaging::region::RegionShape;
use crate::core::imaging::star_mask::StarMaskConfig;

use super::error::MaskError;
use super::grid::WcsSummary;
use super::raster::rasterize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkMode {
    #[default]
    NonZero,
    Zero,
    Nan,
    NonNan,
    Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MaskMods {
    pub feather_px: f32,
    pub invert: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskSource {
    #[default]
    OpInput,
    Node(String),
    Composite,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "StarMaskConfig", default, deny_unknown_fields)]
struct StarParams {
    growth_factor: f64,
    softness: f64,
    #[serde(skip)]
    detection_sigma: f64,
    min_fwhm: f64,
    max_fwhm: f64,
    max_eccentricity: f64,
    luminance_protect: bool,
    luminance_ceiling: f64,
}

impl Default for StarParams {
    fn default() -> Self {
        let StarMaskConfig {
            growth_factor,
            softness,
            detection_sigma,
            min_fwhm,
            max_fwhm,
            max_eccentricity,
            luminance_protect,
            luminance_ceiling,
        } = StarMaskConfig::default();
        Self {
            growth_factor,
            softness,
            detection_sigma,
            min_fwhm,
            max_fwhm,
            max_eccentricity,
            luminance_protect,
            luminance_ceiling,
        }
    }
}

fn default_star_sigma() -> f32 {
    StarMaskConfig::default().detection_sigma as f32
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MaskRecipe {
    Luminance {
        #[serde(default)]
        auto_stf: bool,
    },
    Range {
        lo: f32,
        hi: f32,
        #[serde(default)]
        fuzz: f32,
    },
    Stars {
        #[serde(default, with = "StarParams")]
        config: StarMaskConfig,
        #[serde(default = "default_star_sigma")]
        sigma: f32,
    },
    Regions {
        shapes: Vec<RegionShape>,
        #[serde(default)]
        exclude_shapes: Vec<RegionShape>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_wcs: Option<WcsSummary>,
    },
    File {
        path: String,
        #[serde(default)]
        mark: MarkMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lo: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hi: Option<f32>,
    },
}

impl MaskRecipe {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Luminance { .. } => "luminance",
            Self::Range { .. } => "range",
            Self::Stars { .. } => "stars",
            Self::Regions { .. } => "regions",
            Self::File { .. } => "file",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaskSpec {
    #[serde(flatten)]
    pub mods: MaskMods,
    #[serde(flatten)]
    pub recipe: MaskRecipe,
}

impl MaskSpec {
    pub fn rasterize(&self, planes: &[&Array2<f32>]) -> Result<Array2<f32>, MaskError> {
        rasterize(&self.recipe, &self.mods, planes)
    }
}

pub fn recipe_hash(recipe: &MaskRecipe, mods: &MaskMods) -> u64 {
    let canonical = serde_json::to_string(&(recipe, mods)).expect("a mask recipe has no map keys, so it always serialises");
    Fnv1a::default().update(canonical.as_bytes()).finish()
}
