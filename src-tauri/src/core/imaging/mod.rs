use crate::types::header::HduHeader;

pub mod background;
pub mod boundary;
pub mod calibration_pipeline;
pub mod colormap;
pub mod contour;
pub mod cosmetic;
pub mod curves;
pub mod dbe;
pub mod debayer;
pub mod dq_flags;
pub mod geometry;
pub mod masked_stretch;
pub mod pixel_probe;
pub mod psf_estimation;
pub mod region;
pub mod region_file;
pub mod resample;
pub mod sampling;
pub mod scale;
pub mod scnr;
pub mod star_mask;
pub mod star_removal;
pub mod stats;
pub mod stf;
pub mod stretch;
pub mod wavelet;
pub mod zscale;
pub mod statistics;
pub mod hdr;
pub mod local_contrast;
pub mod luminance;
pub mod cutout;

pub const HISTORY_TEXT_BYTES: usize = 72;

pub fn wrap_history(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut rest = word;
        while rest.len() > HISTORY_TEXT_BYTES {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut cut = HISTORY_TEXT_BYTES;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            lines.push(rest[..cut].to_string());
            rest = &rest[cut..];
        }
        if rest.is_empty() {
            continue;
        }
        if current.is_empty() {
            current.push_str(rest);
        } else if current.len() + 1 + rest.len() <= HISTORY_TEXT_BYTES {
            current.push(' ');
            current.push_str(rest);
        } else {
            lines.push(std::mem::replace(&mut current, rest.to_string()));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

pub fn push_history(header: &mut HduHeader, text: &str) {
    for line in wrap_history(text) {
        header.cards.push(("HISTORY".to_string(), line));
    }
}
