use crate::core::imaging::region::RegionError;

#[derive(Debug, thiserror::Error)]
pub enum MaskError {
    #[error("mask {id} is {}×{}, image is {}×{}", .mask.1, .mask.0, .image.1, .image.0)]
    Dims { id: String, mask: (usize, usize), image: (usize, usize) },
    #[error("mask {0} is not in this image's history")]
    Unknown(String),
    #[error("mask {0} was deleted; pick another mask or create it again")]
    Deleted(String),
    #[error("a mask is built from one or three planes; got {0}")]
    PlaneCount(usize),
    #[error("a mask needs planes of one size; got {0:?}")]
    PlaneDims(Vec<(usize, usize)>),
    #[error("{role} {index} ({kind}): {source}")]
    Region { role: &'static str, index: usize, kind: &'static str, source: RegionError },
    #[error("{0}")]
    Invalid(String),
    #[error("only a regions mask has a label image; this mask is {0}")]
    NotRegions(&'static str),
    #[error("mask file {path} cannot be read: {reason}")]
    File { path: String, reason: String },
    #[error("star mask: {0}")]
    Stars(String),
    #[error("mask file {mask} was not rotated or flipped like {image} ({detail}); redraw or re-export the mask")]
    FileGrid { mask: String, image: String, detail: String },
    #[error("region mask was drawn on another grid than {image} ({detail}); redraw the regions on {image}")]
    RegionsGrid { image: String, detail: String },
    #[error(
        "a blend needs base, processed and mask of one size; got base {}×{}, processed {}×{}, mask {}×{}",
        .base.1, .base.0, .processed.1, .processed.0, .mask.1, .mask.0
    )]
    BlendDims { base: (usize, usize), processed: (usize, usize), mask: (usize, usize) },
}
