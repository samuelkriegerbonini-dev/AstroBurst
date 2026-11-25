mod error;
pub mod fk5;
pub mod inverse;
pub mod parse;
pub mod pipeline;
pub mod sip_fit;
pub mod source;
pub mod transform;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod test_support;

pub use error::GwcsError;
pub use inverse::{NewtonOptions, GWCS_INVERSE_ACCEPT_PX};
pub use parse::{
    classify_wcs_node, find_wcs_node, is_fitswcs_imaging, missing_wcs_node, parse_gwcs, parse_node,
    tag_family, wcs_node_at, wcsinfo_sip_residuals, ArrayResolver, WcsNodeClass, WcsNodeKey,
    WCS_NODE_SEARCH_ORDER,
};
pub use pipeline::{BoundingBox, FrameKind, GwcsFrame, GwcsPipeline, GwcsStep, InverseResult};
pub use sip_fit::{fit_pipeline_tan_sip, fit_tan_sip, SipFit, SipFitOptions, SIP_FIT_MAX_DEGREE};
pub use source::{GwcsOrigin, GwcsSource};
pub use transform::{Axis, Kind, Node, Vals, MAX_LANES};
