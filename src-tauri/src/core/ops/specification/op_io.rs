use ndarray::Array2;
use serde_json::Value;

use crate::core::ops::domain::is_display_referred;
use crate::core::ops::pixelmath::SlotLoader;
use crate::core::ops::spec::OutputValues;
use crate::types::header::HduHeader;

#[derive(Clone, Copy, Default)]
pub struct OpCompanions<'a> {
    pub psf_kernel: Option<&'a Array2<f32>>,
    pub load_slot: Option<SlotLoader<'a>>,
}

pub struct OpInput<'a> {
    pub planes: Vec<&'a Array2<f32>>,
    pub header: Option<&'a HduHeader>,
    pub companions: OpCompanions<'a>,
    pub mask: Option<&'a Array2<f32>>,
}

impl<'a> OpInput<'a> {
    pub fn mono(plane: &'a Array2<f32>, header: Option<&'a HduHeader>) -> Self {
        Self { planes: vec![plane], header, companions: OpCompanions::default(), mask: None }
    }

    pub fn rgb(planes: [&'a Array2<f32>; 3], header: Option<&'a HduHeader>) -> Self {
        Self { planes: planes.to_vec(), header, companions: OpCompanions::default(), mask: None }
    }

    pub fn with_companions(self, companions: OpCompanions<'a>) -> Self {
        Self { companions, ..self }
    }

    pub fn with_mask(self, mask: Option<&'a Array2<f32>>) -> Self {
        Self { mask, ..self }
    }

    pub(in crate::core::ops) fn display_referred(&self) -> bool {
        is_display_referred(self.header)
    }
}

#[derive(Debug)]
pub struct OpOutput {
    pub planes: Vec<Array2<f32>>,
    pub model: Option<Array2<f32>>,
    pub history_lines: Vec<String>,
    pub values: OutputValues,
    pub extras: Value,
}

impl OpOutput {
    pub(in crate::core::ops) fn new(planes: Vec<Array2<f32>>, values: OutputValues, extras: Value) -> Self {
        Self { planes, model: None, history_lines: Vec::new(), values, extras }
    }
}
