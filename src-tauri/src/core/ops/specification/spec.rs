use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::core::imaging::hdr::HdrConfig;
use crate::core::imaging::local_contrast::LheConfig;
use crate::core::imaging::masked_stretch::MaskedStretchConfig;
use crate::core::imaging::stretch::GhsParams;
use crate::core::imaging::wavelet::WaveletConfig;
use crate::core::ops::background::BackgroundModel;
use crate::core::ops::pixelmath::PixelMathSlot;
use crate::core::ops::psf::PsfSpec;
use crate::core::ops::stretch::{clamped_arcsinh_factor, clamped_ghs, clamped_masked_stretch};
use crate::core::ops::tone::{ToneCurveInput, ToneLevelsInput};
use crate::types::stacking::RLConfig;

pub const OPSPEC_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputValues {
    Linear,
    Rescaled,
    DisplayReferred,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpVersion;

impl Serialize for OpVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(OPSPEC_VERSION)
    }
}

impl<'de> Deserialize<'de> for OpVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = u64::deserialize(deserializer)?;
        if v == u64::from(OPSPEC_VERSION) {
            Ok(OpVersion)
        } else {
            Err(D::Error::custom(format!(
                "unsupported OpSpec version {v}; this build reads version {OPSPEC_VERSION}"
            )))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpSpec {
    Background {
        v: OpVersion,
        #[serde(flatten)]
        model: BackgroundModel,
    },
    WaveletDenoise {
        v: OpVersion,
        #[serde(flatten)]
        config: WaveletConfig,
    },
    RlDeconv {
        v: OpVersion,
        #[serde(flatten)]
        config: RLConfig,
        psf: PsfSpec,
    },
    Arcsinh {
        v: OpVersion,
        factor: f64,
    },
    Ghs {
        v: OpVersion,
        #[serde(flatten)]
        params: GhsParams,
    },
    MaskedStretch {
        v: OpVersion,
        #[serde(flatten)]
        config: MaskedStretchConfig,
    },
    Lhe {
        v: OpVersion,
        #[serde(flatten)]
        config: LheConfig,
    },
    Hdrmt {
        v: OpVersion,
        #[serde(flatten)]
        config: HdrConfig,
    },
    Tone {
        v: OpVersion,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        levels: Option<ToneLevelsInput>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        curve: Option<ToneCurveInput>,
    },
    Pixelmath {
        v: OpVersion,
        expression: String,
        slots: Vec<PixelMathSlot>,
        truncate: bool,
        rescale: bool,
    },
}

impl OpSpec {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Background { .. } => "background",
            Self::WaveletDenoise { .. } => "wavelet_denoise",
            Self::RlDeconv { .. } => "rl_deconv",
            Self::Arcsinh { .. } => "arcsinh",
            Self::Ghs { .. } => "ghs",
            Self::MaskedStretch { .. } => "masked_stretch",
            Self::Lhe { .. } => "lhe",
            Self::Hdrmt { .. } => "hdrmt",
            Self::Tone { .. } => "tone",
            Self::Pixelmath { .. } => "pixelmath",
        }
    }

    pub fn normalised(self) -> Self {
        match self {
            Self::Arcsinh { v, factor } => Self::Arcsinh { v, factor: clamped_arcsinh_factor(factor) },
            Self::Ghs { v, params } => Self::Ghs { v, params: clamped_ghs(&params) },
            Self::MaskedStretch { v, config } => Self::MaskedStretch { v, config: clamped_masked_stretch(&config) },
            other => other,
        }
    }

    pub(in crate::core::ops) fn runs_on_one_plane_only(&self) -> bool {
        matches!(
            self,
            Self::Background { .. } | Self::WaveletDenoise { .. } | Self::RlDeconv { .. } | Self::MaskedStretch { .. }
        )
    }
}
