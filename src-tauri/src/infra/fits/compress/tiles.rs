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
            let signed = ctx.zbitpix.unsigned_abs() != 8;
            let params = RiceParams {
                blocksize: ctx.blocksize,
                bytepix: ctx.rice_bytepix,
                signed,
            };
            let ints = rice_decode(raw, nx, &params);
            match ctx.dither {
                Some((seed, tile_index)) if ctx.tile_quant.is_some() => {
                    Ok(scale_ints_dither(&ints, bscale, bzero, ctx.blank, seed, tile_index))
                }
                _ => Ok(scale_ints(&ints, bscale, bzero, ctx.blank)),
            }
        }
        "GZIP_1" | "GZIP_2" => {
            if ctx.dither.is_some() && ctx.tile_quant.is_some() {
                bail!("SUBTRACTIVE_DITHER_1 with GZIP-compressed tiles is not supported");
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
            if ctx.dither.is_some() && ctx.tile_quant.is_some() {
                bail!("SUBTRACTIVE_DITHER_1 with NOCOMPRESS tiles is not supported");
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

/// Decode a full 2D compressed-image BINTABLE HDU into a plain pixel array.
/// `data_start` is the HDU's data-unit start offset (same field already
/// computed by `parse_header_at` for any HDU type).
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
    let is_quantized = header.get("ZQUANTIZ").is_some()
        && layout.columns.contains_key("ZSCALE")
        && layout.columns.contains_key("ZZERO");

    let global_bzero = header.get_f64("BZERO").unwrap_or(0.0);
    let global_bscale = header.get_f64("BSCALE").unwrap_or(1.0);
    let global_blank = header.get_i64("ZBLANK").or_else(|| header.get_i64("BLANK"));

    let dither_seed: Option<i32> = if is_quantized {
        let zq = header
            .get("ZQUANTIZ")
            .map(|s| s.trim().to_uppercase())
            .unwrap_or_default();
        if zq.contains("SUBTRACTIVE_DITHER_2") {
            bail!("ZQUANTIZ SUBTRACTIVE_DITHER_2 is not supported");
        } else if zq.contains("SUBTRACTIVE_DITHER_1") {
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
        32 => 4,
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

    let zscale_col = layout.columns.get("ZSCALE").copied();
    let zzero_col = layout.columns.get("ZZERO").copied();

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

            let tile_quant = if is_quantized {
                Some((
                    read_fixed_f64(row_bytes, zscale_col.as_ref().unwrap())?,
                    read_fixed_f64(row_bytes, zzero_col.as_ref().unwrap())?,
                ))
            } else {
                None
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
