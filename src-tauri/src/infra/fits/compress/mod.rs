// FITS tile-compression support (Rice/GZIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
pub mod tiles;

#[cfg(test)]
pub use tiles::decode_compressed_image;
pub use tiles::{decode_compressed_planes, is_compressed_image_hdu, read_compressed_shape};
