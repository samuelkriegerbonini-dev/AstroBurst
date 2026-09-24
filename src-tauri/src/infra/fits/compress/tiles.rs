// FITS tile-compression support (Rice/GZIP) — contributed by Jae-Joon Lee <https://github.com/leejjoon>
//! FITS Tiled Image Compression Convention: BINTABLE column/heap layout,
//! tile geometry, and per-tile decode orchestration.
//!
//! Reference: https://fits.gsfc.nasa.gov/registry/tilecompression.html
//!
//! Scope: 2D (ZNAXIS=2) compressed images, and plane-aligned 3D (ZNAXIS=3,
//! ZTILE3=1) compressed cubes, ZCMPTYPE in {RICE_1, GZIP_1, GZIP_2,
//! NOCOMPRESS}, global or per-tile (ZSCALE/ZZERO) float requantization.
//! Not yet supported: non-plane-aligned 3D tiling (ZTILE3>1), PLIO_1,
//! HCOMPRESS_1, NULL_PIXEL_MASK (per-pixel null bitmap), per-tile ZBLANK
//! column (only a scalar header BLANK/ZBLANK is honored).

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::types::HduHeader;

use super::gzip::gzip_decode;
use super::rice::{rice_decode, RiceParams};

pub fn is_compressed_image_hdu(header: &HduHeader) -> bool {
    let is_bintable = header
        .get("XTENSION")
        .map(|x| x.trim().eq_ignore_ascii_case("BINTABLE"))
        .unwrap_or(false);
    let is_zimage = header
        .get("ZIMAGE")
        .map(|v| v.trim().eq_ignore_ascii_case("T"))
        .unwrap_or(false);
    is_bintable && is_zimage
}

/// Decompressed-image shape/type as advertised by the Z-prefixed keywords,
/// used by the caller (reader.rs) to populate HduInfo without decoding.
pub struct CompressedImageShape {
    pub znaxis: i64,
    pub znaxis1: i64,
    pub znaxis2: i64,
    pub znaxis3: i64,
    pub zbitpix: i64,
}

pub fn read_compressed_shape(header: &HduHeader) -> CompressedImageShape {
    CompressedImageShape {
        znaxis: header.get_i64("ZNAXIS").unwrap_or(0),
        znaxis1: header.get_i64("ZNAXIS1").unwrap_or(0),
        znaxis2: header.get_i64("ZNAXIS2").unwrap_or(0),
        znaxis3: header.get_i64("ZNAXIS3").unwrap_or(0),
        zbitpix: header.get_i64("ZBITPIX").unwrap_or(0),
    }
}

#[derive(Debug, Clone, Copy)]
enum ColumnSpan {
    Fixed {
        offset: usize,
        type_code: char,
    },
    VarArray {
        offset: usize,
        elem_bytes: usize,
        wide: bool,
    },
}

struct BintableLayout {
    row_width: usize,
    n_rows: usize,
    heap_base: usize, // absolute mmap offset where the variable-length heap begins
    columns: HashMap<String, ColumnSpan>,
}

fn tform_elem_bytes(type_code: char) -> Result<usize> {
    Ok(match type_code {
        'L' | 'B' | 'A' => 1,
        'I' => 2,
        'J' | 'E' => 4,
        'K' | 'D' | 'C' => 8,
        'M' => 16,
        other => bail!("Unsupported BINTABLE column type code '{other}'"),
    })
}

enum TformKind {
    Fixed { repeat: usize, type_code: char },
    VarArray { elem_type: char, wide: bool },
}

fn parse_tform(tform: &str) -> Result<TformKind> {
    let tform = tform.trim();
    let digit_end = tform
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(tform.len());
    let repeat: usize = if digit_end == 0 {
        1
    } else {
        tform[..digit_end].parse().unwrap_or(1)
    };
    let mut chars = tform[digit_end..].chars();
    let type_code = chars.next().context("empty TFORM type code")?;
    if type_code == 'P' || type_code == 'Q' {
        let elem_type = chars
            .next()
            .context("variable-length TFORM missing element type code")?;
        Ok(TformKind::VarArray {
            elem_type,
            wide: type_code == 'Q',
        })
    } else {
        Ok(TformKind::Fixed { repeat, type_code })
    }
}

fn build_bintable_layout(header: &HduHeader, data_start: usize) -> Result<BintableLayout> {
    let naxis1 = header.get_i64("NAXIS1").context("Missing NAXIS1")? as usize;
    let naxis2 = header.get_i64("NAXIS2").context("Missing NAXIS2")? as usize;
    let tfields = header.get_i64("TFIELDS").unwrap_or(0) as usize;

    let mut offset = 0usize;
    let mut columns = HashMap::new();

    for i in 1..=tfields {
        let tform = header
            .get(&format!("TFORM{i}"))
            .with_context(|| format!("Missing TFORM{i}"))?;
        let ttype = header
            .get(&format!("TTYPE{i}"))
            .map(|s| s.trim().to_uppercase());

        let (span, width) = match parse_tform(tform)? {
            TformKind::VarArray { elem_type, wide } => {
                let elem_bytes = tform_elem_bytes(elem_type)?;
                let width = if wide { 16 } else { 8 };
                (
                    ColumnSpan::VarArray {
                        offset,
                        elem_bytes,
                        wide,
                    },
                    width,
                )
            }
            TformKind::Fixed { repeat, type_code } => {
                let width = if type_code == 'X' {
                    repeat.div_ceil(8)
                } else {
                    repeat * tform_elem_bytes(type_code)?
                };
                (ColumnSpan::Fixed { offset, type_code }, width)
            }
        };

        if let Some(name) = ttype {
            columns.insert(name, span);
        }
        offset += width;
    }

    if offset != naxis1 {
        bail!(
            "BINTABLE row width mismatch: TFORM columns sum to {} bytes, NAXIS1={}",
            offset,
            naxis1
        );
    }

    let theap = header.get_i64("THEAP").unwrap_or((naxis1 * naxis2) as i64) as usize;
    let heap_base = data_start + theap;

    Ok(BintableLayout {
        row_width: naxis1,
        n_rows: naxis2,
        heap_base,
        columns,
    })
}

fn read_var_descriptor(row: &[u8], span: &ColumnSpan) -> Result<(usize, usize)> {
    match *span {
        ColumnSpan::VarArray {
            offset,
            elem_bytes,
            wide,
        } => {
            let (nelem, rel) = if wide {
                let nelem = i64::from_be_bytes(row[offset..offset + 8].try_into().unwrap());
                let rel = i64::from_be_bytes(row[offset + 8..offset + 16].try_into().unwrap());
                (nelem as usize, rel as usize)
            } else {
                let nelem = i32::from_be_bytes(row[offset..offset + 4].try_into().unwrap());
                let rel = i32::from_be_bytes(row[offset + 4..offset + 8].try_into().unwrap());
                (nelem as usize, rel as usize)
            };
            Ok((nelem * elem_bytes, rel * elem_bytes))
        }
        _ => bail!("column is not a variable-length array"),
    }
}

fn read_fixed_f64(row: &[u8], span: &ColumnSpan) -> Result<f64> {
    match *span {
        ColumnSpan::Fixed {
            offset,
            type_code: 'D',
            ..
        } => Ok(f64::from_be_bytes(
            row[offset..offset + 8].try_into().unwrap(),
        )),
        ColumnSpan::Fixed {
            offset,
            type_code: 'E',
            ..
        } => Ok(f32::from_be_bytes(row[offset..offset + 4].try_into().unwrap()) as f64),
        _ => bail!("column is not a fixed D/E scalar"),
    }
}

enum QuantSource {
    None,
    Columns { zscale: ColumnSpan, zzero: ColumnSpan },
    Keywords { zscale: f64, zzero: f64 },
}

fn quantization_source(header: &HduHeader, layout: &BintableLayout, zquantiz: &str, zbitpix: i64) -> QuantSource {
    if zquantiz == "NONE" {
        return QuantSource::None;
    }
    if let (Some(&zscale), Some(&zzero)) = (layout.columns.get("ZSCALE"), layout.columns.get("ZZERO")) {
        return QuantSource::Columns { zscale, zzero };
    }
    match (header.get_f64("ZSCALE"), header.get_f64("ZZERO")) {
        (Some(zscale), Some(zzero)) if zbitpix < 0 && zscale.is_finite() && zzero.is_finite() => {
            QuantSource::Keywords { zscale, zzero }
        }
        _ => QuantSource::None,
    }
}

fn find_zval(header: &HduHeader, name: &str, default: i64) -> i64 {
    for i in 1..=99 {
        let key = format!("ZNAME{i}");
        match header.get(&key) {
            Some(v) if v.trim().eq_ignore_ascii_case(name) => {
                return header.get_i64(&format!("ZVAL{i}")).unwrap_or(default);
            }
            Some(_) => continue,
            None => break,
        }
    }
    default
}

struct TileGeometry {
    znaxis1: usize,
    znaxis2: usize,
    znaxis3: usize,
    ztile1: usize,
    ztile2: usize,
    #[allow(dead_code)]
    ztile3: usize,
    tiles_x: usize,
    tiles_y: usize,
    tiles_z: usize,
    zbitpix: i64,
}

fn parse_tile_geometry(header: &HduHeader) -> Result<TileGeometry> {
    let znaxis = header.get_i64("ZNAXIS").unwrap_or(0);
    if znaxis != 2 && znaxis != 3 {
        bail!("Compressed images with ZNAXIS={znaxis} are not supported (only 2D or 3D)");
    }
    let znaxis1 = header.get_i64("ZNAXIS1").context("Missing ZNAXIS1")? as usize;
    let znaxis2 = header.get_i64("ZNAXIS2").context("Missing ZNAXIS2")? as usize;
    let ztile1 = header.get_i64("ZTILE1").unwrap_or(znaxis1 as i64).max(1) as usize;
    let ztile2 = header.get_i64("ZTILE2").unwrap_or(1).max(1) as usize;
    let zbitpix = header.get_i64("ZBITPIX").context("Missing ZBITPIX")?;

    let (znaxis3, ztile3) = if znaxis == 3 {
        let znaxis3 = header.get_i64("ZNAXIS3").context("Missing ZNAXIS3")? as usize;
        let ztile3 = header.get_i64("ZTILE3").unwrap_or(1).max(1) as usize;
        if ztile3 != 1 {
            bail!(
                "Compressed 3D cubes with ZTILE3={ztile3} are not supported \
                 (only plane-aligned ZTILE3=1 cubes)"
            );
        }
        (znaxis3, ztile3)
    } else {
        (1, 1)
    };

    Ok(TileGeometry {
        znaxis1,
        znaxis2,
        znaxis3,
        ztile1,
        ztile2,
        ztile3,
        tiles_x: znaxis1.div_ceil(ztile1),
        tiles_y: znaxis2.div_ceil(ztile2),
        tiles_z: znaxis3.div_ceil(ztile3),
        zbitpix,
    })
}

fn scale_ints(values: &[i64], bscale: f64, bzero: f64, blank: Option<i64>) -> Vec<f32> {
    values
        .iter()
        .map(|&v| {
            if blank == Some(v) {
                f32::NAN
            } else {
                (v as f64 * bscale + bzero) as f32
            }
        })
        .collect()
}

struct TileCodecCtx<'a> {
    zcmptype: &'a str,
    zbitpix: i64,
    blocksize: u32,
    rice_bytepix: u32,
    /// BSCALE/BZERO from the primary header -- the physical scaling that
    /// applies to any tile stored in its native (non-quantized) binary form:
    /// UNCOMPRESSED_DATA, GZIP_COMPRESSED_DATA, NOCOMPRESS, or plain integer
    /// RICE_1/GZIP_1/GZIP_2 with no ZQUANTIZ in play.
    global_bscale: f64,
    global_bzero: f64,
    /// Per-tile ZSCALE/ZZERO, present only when this HDU quantizes floats.
    /// Applies *only* to values decoded from COMPRESSED_DATA under a lossy
    /// quantizing codec -- never to the raw-storage fallback columns above,
    /// which already hold physical values (verified against real astropy
    /// output: a fully-uniform quantized tile falls back to
    /// GZIP_COMPRESSED_DATA holding the literal physical float bytes, with
    /// ZSCALE/ZZERO for that row left at a meaningless 0.0/0.0).
    tile_quant: Option<(f64, f64)>,
    dither: Option<(i32, usize)>,
    blank: Option<i64>,
}

fn scale_ints_dither(
    values: &[i64],
    bscale: f64,
    bzero: f64,
    blank: Option<i64>,
    seed: i32,
    tile_index: usize,
) -> Vec<f32> {
    let mut dither = super::quantize::TileDither::new(seed, tile_index);
    values
        .iter()
        .map(|&v| {
            let r = dither.next();
            match blank {
                Some(b) if v == b => f32::NAN,
                _ => ((v as f64 - r + 0.5) * bscale + bzero) as f32,
            }
        })
        .collect()
}

fn scale_quantized_ints(
    bytes: &[u8],
    nx: usize,
    bscale: f64,
    bzero: f64,
    ctx: &TileCodecCtx,
) -> Result<Vec<f32>> {
    if nx == 0 || !bytes.len().is_multiple_of(nx) {
        bail!("quantized tile holds {} bytes for {nx} pixels", bytes.len());
    }
    let width = bytes.len() / nx;
    let ints: Vec<i64> = match width {
        1 => bytes.iter().map(|&b| b as i64).collect(),
        2 => bytes
            .chunks_exact(2)
            .map(|c| i16::from_be_bytes([c[0], c[1]]) as i64)
            .collect(),
        4 => bytes
            .chunks_exact(4)
            .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]) as i64)
            .collect(),
        8 => bytes
            .chunks_exact(8)
            .map(|c| i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect(),
        other => bail!("unsupported quantized tile element width {other}"),
    };
    Ok(match ctx.dither {
        Some((seed, tile_index)) => {
            scale_ints_dither(&ints, bscale, bzero, ctx.blank, seed, tile_index)
        }
        None => scale_ints(&ints, bscale, bzero, ctx.blank),
    })
}

fn decode_one_tile(
    mmap: &[u8],
    row_bytes: &[u8],
    layout: &BintableLayout,
    nx: usize,
    ctx: &TileCodecCtx,
) -> Result<Vec<f32>> {
    let read_column = |name: &str| -> Result<Option<&[u8]>> {
        match layout.columns.get(name) {
            Some(span) => {
                let (nbytes, rel) = read_var_descriptor(row_bytes, span)?;
                if nbytes == 0 {
                    return Ok(None);
                }
                let start = layout
                    .heap_base
                    .checked_add(rel)
                    .filter(|&s| s.checked_add(nbytes).is_some_and(|end| end <= mmap.len()))
                    .with_context(|| {
                        format!(
                            "Compressed tile heap range [{}+{}, +{}) exceeds file size {} (truncated file?)",
                            layout.heap_base,
                            rel,
                            nbytes,
                            mmap.len()
                        )
                    })?;
                Ok(Some(&mmap[start..start + nbytes]))
            }
            None => Ok(None),
        }
    };

    // A tile that gained nothing from compression may be stored raw instead,
    // with an empty (nelem=0) COMPRESSED_DATA entry for that row: either
    // UNCOMPRESSED_DATA (native BITPIX bytes, integer or float) or, for
    // quantized-float HDUs, GZIP_COMPRESSED_DATA (native float bytes,
    // gzip-wrapped, no byte-plane shuffle). Both hold physical values
    // already, so they use the header's global BSCALE/BZERO, never the
    // per-tile ZSCALE/ZZERO.
    if let Some(raw) = read_column("UNCOMPRESSED_DATA")? {
        return Ok(super::super::reader::decode_pixels_blank(
            raw,
            ctx.zbitpix,
            ctx.global_bscale,
            ctx.global_bzero,
            ctx.blank,
        ));
    }
    if let Some(raw) = read_column("GZIP_COMPRESSED_DATA")? {
        let decompressed =
            gzip_decode(raw, (ctx.zbitpix.unsigned_abs() / 8).max(1) as usize, false)?;
        return Ok(super::super::reader::decode_pixels_blank(
            &decompressed,
            ctx.zbitpix,
            ctx.global_bscale,
            ctx.global_bzero,
            ctx.blank,
        ));
    }

    let raw = read_column("COMPRESSED_DATA")?
        .context("Compressed image HDU missing COMPRESSED_DATA column")?;

    // Quantized floats apply per-tile ZSCALE/ZZERO to the decoded integers;
    // everything else (plain integer Rice/GZIP) uses the header's BSCALE/BZERO.
    let (bscale, bzero) = ctx
        .tile_quant
        .unwrap_or((ctx.global_bscale, ctx.global_bzero));

    match ctx.zcmptype {
        "RICE_1" => {
            if ctx.zbitpix < 0 && ctx.tile_quant.is_none() {
                bail!(
                    "RICE_1 compressed floating-point image (ZBITPIX={}) has no ZSCALE/ZZERO \
                     columns or keywords, so its integer codes cannot be turned back into values",
                    ctx.zbitpix
                );
            }
            let signed = ctx.zbitpix.unsigned_abs() != 8;
            let params = RiceParams {
                blocksize: ctx.blocksize,
                bytepix: ctx.rice_bytepix,
                signed,
            };
            let ints = rice_decode(raw, nx, &params)?;
            match ctx.dither {
                Some((seed, tile_index)) if ctx.tile_quant.is_some() => {
                    Ok(scale_ints_dither(&ints, bscale, bzero, ctx.blank, seed, tile_index))
                }
                _ => Ok(scale_ints(&ints, bscale, bzero, ctx.blank)),
            }
        }
        "GZIP_1" | "GZIP_2" => {
            if ctx.tile_quant.is_some() {
                let decompressed = gzip_decode(raw, 1, false)?;
                let width = decompressed.len().checked_div(nx).unwrap_or(0);
                let bytes = if ctx.zcmptype == "GZIP_2" && width > 1 && decompressed.len() % nx == 0 {
                    super::gzip::unshuffle_bytes(&decompressed, width)
                } else {
                    decompressed
                };
                return scale_quantized_ints(&bytes, nx, bscale, bzero, ctx);
            }
            let bytepix = (ctx.zbitpix.unsigned_abs() / 8) as usize;
            let decompressed = gzip_decode(raw, bytepix, ctx.zcmptype == "GZIP_2")?;
            Ok(super::super::reader::decode_pixels_blank(
                &decompressed,
                ctx.zbitpix,
                bscale,
                bzero,
                ctx.blank,
            ))
        }
        "NOCOMPRESS" => {
            if ctx.tile_quant.is_some() {
                return scale_quantized_ints(raw, nx, bscale, bzero, ctx);
            }
            Ok(super::super::reader::decode_pixels_blank(
                raw,
                ctx.zbitpix,
                bscale,
                bzero,
                ctx.blank,
            ))
        }
        other => {
            bail!("Unsupported ZCMPTYPE '{other}' (only RICE_1/GZIP_1/GZIP_2/NOCOMPRESS supported)")
        }
    }
}

#[cfg(test)]
pub fn decode_compressed_image(
    mmap: &[u8],
    header: &HduHeader,
    data_start: usize,
) -> Result<Array2<f32>> {
    let mut planes = decode_compressed_planes(mmap, header, data_start)?;
    if planes.len() != 1 {
        bail!(
            "decode_compressed_image expects a single-plane (ZNAXIS=2) compressed \
             image, found {} planes -- use decode_compressed_planes for cubes",
            planes.len()
        );
    }
    Ok(planes.remove(0))
}

pub fn decode_compressed_planes(
    mmap: &[u8],
    header: &HduHeader,
    data_start: usize,
) -> Result<Vec<Array2<f32>>> {
    let geom = parse_tile_geometry(header)?;
    let layout = build_bintable_layout(header, data_start)?;

    let zcmptype = header
        .get("ZCMPTYPE")
        .context("Missing ZCMPTYPE")?
        .trim()
        .to_uppercase();
    let zquantiz = header
        .get("ZQUANTIZ")
        .map(|s| s.trim().to_uppercase())
        .unwrap_or_default();
    let quant_source = quantization_source(header, &layout, &zquantiz, geom.zbitpix);
    let is_quantized = !matches!(quant_source, QuantSource::None);

    let global_bzero = header.get_f64("BZERO").unwrap_or(0.0);
    let global_bscale = header.get_f64("BSCALE").unwrap_or(1.0);
    let global_blank = header.get_i64("ZBLANK").or_else(|| header.get_i64("BLANK"));

    let dither_seed: Option<i32> = if is_quantized {
        if zquantiz.contains("SUBTRACTIVE_DITHER_2") {
            bail!("ZQUANTIZ SUBTRACTIVE_DITHER_2 is not supported");
        } else if zquantiz.contains("SUBTRACTIVE_DITHER_1") {
            Some(
                header
                    .get_i64("ZDITHER0")
                    .context("Missing ZDITHER0 for SUBTRACTIVE_DITHER_1")? as i32,
            )
        } else {
            None
        }
    } else {
        None
    };

    let default_bytepix = match geom.zbitpix.unsigned_abs() {
        8 => 1,
        16 => 2,
        32 | 64 => 4,
        other => bail!("Unsupported ZBITPIX {other}"),
    };
    let blocksize = find_zval(header, "BLOCKSIZE", 32).max(1) as u32;
    let rice_bytepix = find_zval(header, "BYTEPIX", default_bytepix).max(1) as u32;

    let n_tiles = layout.n_rows;
    let plane_tiles = geom.tiles_x * geom.tiles_y;
    if n_tiles != plane_tiles * geom.tiles_z {
        bail!(
            "BINTABLE row count {} does not match ZTILE geometry {}x{}x{} tiles",
            n_tiles,
            geom.tiles_x,
            geom.tiles_y,
            geom.tiles_z
        );
    }

    let tiles: Vec<Result<DecodedTile>> = (0..n_tiles)
        .into_par_iter()
        .map(|row| -> Result<DecodedTile> {
            let tz = row / plane_tiles;
            let rem = row % plane_tiles;
            let tx = rem % geom.tiles_x;
            let ty = rem / geom.tiles_x;
            let x0 = tx * geom.ztile1;
            let y0 = ty * geom.ztile2;
            let tile_w = geom.ztile1.min(geom.znaxis1 - x0);
            let tile_h = geom.ztile2.min(geom.znaxis2 - y0);
            let nx = tile_w * tile_h;

            let row_start = data_start + row * layout.row_width;
            let row_end = row_start
                .checked_add(layout.row_width)
                .filter(|&end| end <= mmap.len())
                .with_context(|| {
                    format!(
                        "Compressed tile row {} at [{}, +{}) exceeds file size {} (truncated file?)",
                        row,
                        row_start,
                        layout.row_width,
                        mmap.len()
                    )
                })?;
            let row_bytes = &mmap[row_start..row_end];

            let tile_quant = match &quant_source {
                QuantSource::None => None,
                QuantSource::Columns { zscale, zzero } => {
                    Some((read_fixed_f64(row_bytes, zscale)?, read_fixed_f64(row_bytes, zzero)?))
                }
                QuantSource::Keywords { zscale, zzero } => Some((*zscale, *zzero)),
            };

            let ctx = TileCodecCtx {
                zcmptype: &zcmptype,
                zbitpix: geom.zbitpix,
                blocksize,
                rice_bytepix,
                global_bscale,
                global_bzero,
                tile_quant,
                dither: dither_seed.map(|s| (s, row)),
                blank: global_blank,
            };
            let pixels = decode_one_tile(mmap, row_bytes, &layout, nx, &ctx)?;

            if pixels.len() != nx {
                bail!("tile {row} decoded {} pixels, expected {nx}", pixels.len());
            }

            Ok(DecodedTile { tz, x0, y0, w: tile_w, h: tile_h, pixels })
        })
        .collect();

    let mut images: Vec<Array2<f32>> = (0..geom.znaxis3)
        .map(|_| Array2::<f32>::from_elem((geom.znaxis2, geom.znaxis1), f32::NAN))
        .collect();
    for tile in tiles {
        let tile = tile?;
        let image = &mut images[tile.tz];
        for yy in 0..tile.h {
            let row_offset = yy * tile.w;
            for xx in 0..tile.w {
                image[[tile.y0 + yy, tile.x0 + xx]] = tile.pixels[row_offset + xx];
            }
        }
    }

    Ok(images)
}

struct DecodedTile {
    tz: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    pixels: Vec<f32>,
}

#[cfg(test)]
mod tests {
    use super::super::gzip::gzip2_encode;
    use super::super::quantize::NULL_VALUE;
    use super::super::rice_encode::rice_encode;
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn make_header(pairs: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for &(k, v) in pairs {
            header.index.insert(k.to_string(), v.to_string());
            header.cards.push((k.to_string(), v.to_string()));
        }
        header
    }

    fn gzip1(raw: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(raw).unwrap();
        enc.finish().unwrap()
    }

    fn be_i32(values: &[i32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn be_i16(values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    struct SingleTile<'a> {
        zcmptype: &'a str,
        zbitpix: i64,
        npix: usize,
        heap: Vec<u8>,
        quant: Option<(f64, f64, &'a str)>,
    }

    fn single_tile_hdu(t: &SingleTile) -> (Vec<u8>, HduHeader) {
        let row_width: usize = if t.quant.is_some() { 24 } else { 8 };
        let mut mmap = Vec::new();
        mmap.extend_from_slice(&(t.heap.len() as i32).to_be_bytes());
        mmap.extend_from_slice(&0i32.to_be_bytes());
        if let Some((zscale, zzero, _)) = t.quant {
            mmap.extend_from_slice(&zscale.to_be_bytes());
            mmap.extend_from_slice(&zzero.to_be_bytes());
        }
        assert_eq!(mmap.len(), row_width);
        mmap.extend_from_slice(&t.heap);

        let npix = t.npix.to_string();
        let zbitpix = t.zbitpix.to_string();
        let naxis1 = row_width.to_string();
        let tfields = if t.quant.is_some() { "3" } else { "1" };
        let mut pairs: Vec<(&str, &str)> = vec![
            ("XTENSION", "BINTABLE"),
            ("BITPIX", "8"),
            ("NAXIS", "2"),
            ("NAXIS1", &naxis1),
            ("NAXIS2", "1"),
            ("TFIELDS", tfields),
            ("TTYPE1", "COMPRESSED_DATA"),
            ("TFORM1", "1PB(0)"),
            ("ZIMAGE", "T"),
            ("ZCMPTYPE", t.zcmptype),
            ("ZBITPIX", &zbitpix),
            ("ZNAXIS", "2"),
            ("ZNAXIS1", &npix),
            ("ZNAXIS2", "1"),
            ("ZTILE1", &npix),
            ("ZTILE2", "1"),
        ];
        if let Some((_, _, zquantiz)) = t.quant {
            pairs.extend([
                ("TTYPE2", "ZSCALE"),
                ("TFORM2", "1D"),
                ("TTYPE3", "ZZERO"),
                ("TFORM3", "1D"),
                ("ZBLANK", "-2147483647"),
            ]);
            if !zquantiz.is_empty() {
                pairs.extend([("ZQUANTIZ", zquantiz), ("ZDITHER0", "1")]);
            }
        }
        (mmap, make_header(&pairs))
    }

    fn rice_i32(values: &[i32]) -> Vec<u8> {
        let ints: Vec<i64> = values.iter().map(|&v| v as i64).collect();
        rice_encode(&ints, &RiceParams { blocksize: 32, bytepix: 4, signed: true })
    }

    #[test]
    fn quantized_columns_without_zquantiz_decode_as_no_dither() {
        for (zcmptype, heap) in [
            ("RICE_1", rice_i32(&QUANT_INTS)),
            ("GZIP_1", gzip1(&be_i32(&QUANT_INTS))),
            ("NOCOMPRESS", be_i32(&QUANT_INTS)),
        ] {
            let pixels = decode_single_tile(&SingleTile {
                zcmptype,
                zbitpix: -32,
                npix: QUANT_INTS.len(),
                heap,
                quant: Some((0.5, 100.0, "")),
            })
            .unwrap();
            assert_quantized_no_dither(&pixels);
        }
    }

    #[test]
    fn zscale_and_zzero_header_keywords_scale_every_tile() {
        let (mmap, mut header) = single_tile_hdu(&SingleTile {
            zcmptype: "RICE_1",
            zbitpix: -32,
            npix: QUANT_INTS.len(),
            heap: rice_i32(&QUANT_INTS),
            quant: None,
        });
        header.set("ZSCALE", "0.5".to_string());
        header.set("ZZERO", "100.0".to_string());
        header.set("ZBLANK", NULL_VALUE.to_string());
        let pixels: Vec<f32> = decode_compressed_image(&mmap, &header, 0).unwrap().iter().copied().collect();
        assert_quantized_no_dither(&pixels);
    }

    #[test]
    fn a_rice_float_image_without_any_scale_is_refused_instead_of_returning_codes() {
        let err = decode_single_tile(&SingleTile {
            zcmptype: "RICE_1",
            zbitpix: -32,
            npix: QUANT_INTS.len(),
            heap: rice_i32(&QUANT_INTS),
            quant: None,
        })
        .expect_err("integer codes must not be passed off as float pixels");
        assert!(err.to_string().contains("ZSCALE/ZZERO"), "{err}");
    }

    #[test]
    fn zquantiz_none_keeps_a_lossless_float_tile_unscaled() {
        let values = [1.5f32, -2.25, 1e10, 0.0, 3.0];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "GZIP_2",
            zbitpix: -32,
            npix: values.len(),
            heap: gzip2_encode(&raw, 4),
            quant: Some((0.5, 100.0, "NONE")),
        })
        .unwrap();
        assert_eq!(pixels, values.to_vec());
    }

    fn decode_single_tile(t: &SingleTile) -> Result<Vec<f32>> {
        let (mmap, header) = single_tile_hdu(t);
        let image = decode_compressed_image(&mmap, &header, 0)?;
        Ok(image.iter().copied().collect())
    }

    const QUANT_INTS: [i32; 6] = [0, 1, 2, -3, 40, NULL_VALUE as i32];
    const QUANT_EXPECTED: [f32; 5] = [100.0, 100.5, 101.0, 98.5, 120.0];

    fn assert_quantized_no_dither(pixels: &[f32]) {
        assert_eq!(pixels.len(), QUANT_INTS.len());
        assert_eq!(&pixels[..5], &QUANT_EXPECTED[..]);
        assert!(pixels[5].is_nan(), "ZBLANK pixel must decode to NaN, got {}", pixels[5]);
    }

    #[test]
    fn gzip2_lossless_float64_tile_decodes() {
        let values = [1.5f64, -2.25, 1e10, 0.0, 3.0];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "GZIP_2",
            zbitpix: -64,
            npix: values.len(),
            heap: gzip2_encode(&raw, 8),
            quant: None,
        })
        .unwrap();
        let expected: Vec<f32> = values.iter().map(|&v| v as f32).collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn gzip1_lossless_int64_tile_decodes() {
        let values = [7i64, -8, 1 << 40, 0];
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "GZIP_1",
            zbitpix: 64,
            npix: values.len(),
            heap: gzip1(&raw),
            quant: None,
        })
        .unwrap();
        let expected: Vec<f32> = values.iter().map(|&v| v as f32).collect();
        assert_eq!(pixels, expected);
    }

    #[test]
    fn gzip1_quantized_tile_payload_is_int32() {
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "GZIP_1",
            zbitpix: -32,
            npix: QUANT_INTS.len(),
            heap: gzip1(&be_i32(&QUANT_INTS)),
            quant: Some((0.5, 100.0, "NO_DITHER")),
        })
        .unwrap();
        assert_quantized_no_dither(&pixels);
    }

    #[test]
    fn nocompress_quantized_tile_payload_is_int32() {
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "NOCOMPRESS",
            zbitpix: -32,
            npix: QUANT_INTS.len(),
            heap: be_i32(&QUANT_INTS),
            quant: Some((0.5, 100.0, "NO_DITHER")),
        })
        .unwrap();
        assert_quantized_no_dither(&pixels);
    }

    const QUANT_INTS_I16: [i16; 5] = [0, 1, 2, -3, 40];

    #[test]
    fn gzip1_quantized_tile_int16_payload() {
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "GZIP_1",
            zbitpix: -32,
            npix: QUANT_INTS_I16.len(),
            heap: gzip1(&be_i16(&QUANT_INTS_I16)),
            quant: Some((0.5, 100.0, "NO_DITHER")),
        })
        .unwrap();
        assert_eq!(pixels, QUANT_EXPECTED.to_vec());
    }

    #[test]
    fn gzip2_quantized_tile_int16_payload() {
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "GZIP_2",
            zbitpix: -32,
            npix: QUANT_INTS_I16.len(),
            heap: gzip2_encode(&be_i16(&QUANT_INTS_I16), 2),
            quant: Some((0.5, 100.0, "NO_DITHER")),
        })
        .unwrap();
        assert_eq!(pixels, QUANT_EXPECTED.to_vec());
    }

    #[test]
    fn quantized_tile_with_bad_byte_count_is_an_error() {
        let err = decode_single_tile(&SingleTile {
            zcmptype: "NOCOMPRESS",
            zbitpix: -32,
            npix: 6,
            heap: vec![1u8; 7],
            quant: Some((0.5, 100.0, "NO_DITHER")),
        })
        .expect_err("7 bytes for 6 pixels must be reported as an error");
        assert!(err.to_string().contains("quantized tile holds 7 bytes for 6 pixels"), "{err}");
    }

    #[test]
    fn gzip2_quantized_tile_applies_subtractive_dither() {
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "GZIP_2",
            zbitpix: -64,
            npix: QUANT_INTS.len(),
            heap: gzip2_encode(&be_i32(&QUANT_INTS), 4),
            quant: Some((0.5, 100.0, "SUBTRACTIVE_DITHER_1")),
        })
        .unwrap();
        let ints: Vec<i64> = QUANT_INTS.iter().map(|&v| v as i64).collect();
        let expected = scale_ints_dither(&ints, 0.5, 100.0, Some(NULL_VALUE), 1, 0);
        assert_eq!(pixels.len(), expected.len());
        for (i, (got, want)) in pixels.iter().zip(expected.iter()).enumerate() {
            if want.is_nan() {
                assert!(got.is_nan(), "pixel {i}: expected NaN, got {got}");
            } else {
                assert_eq!(got, want, "pixel {i}");
                assert!((got - QUANT_EXPECTED[i]).abs() <= 0.25 + 1e-6, "pixel {i}: {got}");
            }
        }
    }

    #[test]
    fn rice_quantized_float64_tile_defaults_to_bytepix_4() {
        let ints: Vec<i64> = QUANT_INTS.iter().map(|&v| v as i64).collect();
        let params = RiceParams { blocksize: 32, bytepix: 4, signed: true };
        let pixels = decode_single_tile(&SingleTile {
            zcmptype: "RICE_1",
            zbitpix: -64,
            npix: QUANT_INTS.len(),
            heap: rice_encode(&ints, &params),
            quant: Some((0.5, 100.0, "NO_DITHER")),
        })
        .unwrap();
        assert_quantized_no_dither(&pixels);
    }

    #[test]
    fn truncated_rice_tile_is_an_error_not_a_panic() {
        let ints: Vec<i64> = (0..64).map(|i| (i * 37 % 101) - 50).collect();
        let params = RiceParams { blocksize: 32, bytepix: 2, signed: true };
        let encoded = rice_encode(&ints, &params);
        let err = decode_single_tile(&SingleTile {
            zcmptype: "RICE_1",
            zbitpix: 16,
            npix: ints.len(),
            heap: encoded[..encoded.len() / 2].to_vec(),
            quant: None,
        })
        .expect_err("half a Rice stream must be reported as an error");
        assert!(err.to_string().contains("truncated"), "{err}");
    }
}
