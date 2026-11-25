use ndarray::Array2;

use crate::core::mask::{blend, MaskError};
use crate::core::ops::dispatch::dispatch;
use crate::core::ops::error::OpError;
use crate::core::ops::op_io::{OpInput, OpOutput};
use crate::core::ops::spec::OpSpec;
use crate::core::ops::stretch::{arcsinh_blend_base, ghs_blend_base};
use crate::core::ops::tone::tone_blend_base;
use crate::infra::progress::ProgressHandle;

fn check_planes(spec: &OpSpec, input: &OpInput) -> Result<(), OpError> {
    let planes = input.planes.len();
    let allowed = if spec.runs_on_one_plane_only() { planes == 1 } else { planes == 1 || planes == 3 };
    if !allowed {
        return Err(OpError::PlaneCount { op: spec.name(), planes });
    }
    let dims: Vec<(usize, usize)> = input.planes.iter().map(|p| p.dim()).collect();
    if dims.iter().any(|d| *d != dims[0]) {
        return Err(OpError::PlaneDims { op: spec.name(), dims });
    }
    Ok(())
}

pub fn run_op(spec: &OpSpec, input: &OpInput, progress: Option<&ProgressHandle>) -> Result<OpOutput, OpError> {
    check_planes(spec, input)?;
    let Some(mask) = input.mask else {
        return dispatch(spec, input, progress);
    };
    check_mask(spec, input, mask)?;
    if matches!(spec, OpSpec::Pixelmath { .. }) {
        return dispatch(spec, input, progress);
    }
    let base = blend_base(spec, input)?;
    let mut out = dispatch(spec, input, progress)?;
    out.planes = base
        .iter()
        .zip(&out.planes)
        .map(|(base, processed)| blend(base, processed, mask))
        .collect::<Result<_, _>>()?;
    Ok(out)
}

pub fn refuse_mask(spec: &OpSpec) -> Result<(), OpError> {
    if matches!(spec, OpSpec::Background { .. } | OpSpec::MaskedStretch { .. }) {
        return Err(OpError::MaskNotSupported(spec.name()));
    }
    Ok(())
}

fn check_mask(spec: &OpSpec, input: &OpInput, mask: &Array2<f32>) -> Result<(), OpError> {
    refuse_mask(spec)?;
    let image = input.planes[0].dim();
    if mask.dim() != image {
        return Err(MaskError::BlendDims { base: image, processed: image, mask: mask.dim() }.into());
    }
    Ok(())
}

pub fn blend_base(spec: &OpSpec, input: &OpInput) -> Result<Vec<Array2<f32>>, OpError> {
    refuse_mask(spec)?;
    check_planes(spec, input)?;
    let planes = &input.planes;
    Ok(match spec {
        OpSpec::Arcsinh { .. } => arcsinh_blend_base(planes),
        OpSpec::Ghs { .. } => ghs_blend_base(planes),
        OpSpec::Tone { .. } => tone_blend_base(planes, input.display_referred()),
        _ => planes.iter().map(|p| (*p).clone()).collect(),
    })
}
