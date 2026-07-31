// FITS tile-compression support (Rice/GZIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
pub mod gzip;
pub mod rice;
pub mod tiles;

pub use tiles::{decode_compressed_image, is_compressed_image_hdu, read_compressed_shape};
