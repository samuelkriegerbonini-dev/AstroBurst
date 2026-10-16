use std::borrow::Cow;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use rayon::prelude::*;

use crate::core::stacking::memory::{
    effective_memory, format_bytes, MemoryInfo, MEMORY_BUDGET_ENV,
};
use crate::types::header::MAX_FITS_AXES;
use crate::types::HduHeader;

use super::compress;

const MAX_FIELDS: i64 = 999;
const F32_BYTES: u64 = 4;
const RICE_BYTEPIX: [i64; 4] = [1, 2, 4, 8];

const DECODER_KEYS: [&str; 18] = [
    "XTENSION", "BITPIX", "NAXIS", "PCOUNT", "GCOUNT", "TFIELDS", "ZIMAGE", "ZBITPIX", "ZNAXIS",
    "ZCMPTYPE", "ZQUANTIZ", "ZDITHER0", "ZSCALE", "ZZERO", "ZBLANK", "BLANK", "BSCALE", "BZERO",
];
const DECODER_INDEXED_KEYS: [&str; 7] = [
    "NAXIS", "TTYPE", "TFORM", "ZNAXIS", "ZTILE", "ZNAME", "ZVAL",
];

pub fn decode_planes(
    mmap: &[u8],
    header: &HduHeader,
    data_start: usize,
) -> Result<Vec<Array2<f32>>> {
    reject_non_ascii(header)?;
    validate_layout(mmap, header, data_start, effective_memory())?;
    let decoder_header = with_zblank_as_blank(header)?;
    compress::decode_compressed_planes(mmap, &decoder_header, data_start)
}

fn with_zblank_as_blank(header: &HduHeader) -> Result<Cow<'_, HduHeader>> {
    let integer_image = required(header, "ZBITPIX")? > 0;
    let has_blank = header.cards.iter().any(|(key, _)| key == "BLANK");
    if !integer_image || has_blank {
        return Ok(Cow::Borrowed(header));
    }
    Ok(match integer(header, "ZBLANK")? {
        Some(zblank) => {
            let mut view = header.clone();
            view.set("BLANK", zblank.to_string());
            Cow::Owned(view)
        }
        None => Cow::Borrowed(header),
    })
}

fn validate_layout(
    mmap: &[u8],
    header: &HduHeader,
    data_start: usize,
    memory: Option<MemoryInfo>,
) -> Result<()> {
    let table = table_shape(header)?;
    let axes = image_axes(header)?;
    let tiles = tile_shape(header, &axes)?;
    check_tile_count(&axes, &tiles, table.naxis2)?;
    if is_rice(header)? {
        let tile_pixels = axes
            .iter()
            .zip(&tiles)
            .try_fold(1u64, |n, (&axis, &tile)| n.checked_mul(axis.min(tile)))
            .context("compressed image header: the tile size overflows")?;
        check_rice(header, tile_pixels)?;
    }
    let data = mmap.get(data_start..).unwrap_or_default();
    let pcount = check_data_size(header, table.naxis1, table.naxis2, data.len() as u64)?;
    check_memory_budget(header, &axes, memory)?;
    check_descriptors(data, &table, pcount)?;
    check_tiles(header, data, &table, &axes, &tiles, pcount)
}

fn single<'a>(header: &'a HduHeader, key: &str) -> Result<Option<&'a str>> {
    let mut values = header
        .cards
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str());
    let first = values.next();
    if values.next().is_some() {
        bail!("compressed image header: {key} appears more than once");
    }
    Ok(first)
}

fn integer(header: &HduHeader, key: &str) -> Result<Option<i64>> {
    let value = single(header, key)?;
    if header.is_string_value(key) == Some(true) {
        return Ok(None);
    }
    Ok(value.and_then(|v| v.trim().parse().ok()))
}

fn required(header: &HduHeader, key: &str) -> Result<i64> {
    integer(header, key)?
        .with_context(|| format!("compressed image header: {key} is missing or not an integer"))
}

fn at_least_one(key: &str, value: i64) -> Result<u64> {
    match u64::try_from(value) {
        Ok(n) if n >= 1 => Ok(n),
        _ => bail!("compressed image header: {key} = {value} must be at least 1"),
    }
}

fn non_negative(key: &str, value: i64) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| anyhow::anyhow!("compressed image header: {key} = {value} is negative"))
}

fn element_bytes(code: u8) -> Option<u64> {
    match code {
        b'L' | b'B' | b'A' => Some(1),
        b'I' => Some(2),
        b'J' | b'E' => Some(4),
        b'K' | b'D' | b'C' => Some(8),
        b'M' => Some(16),
        _ => None,
    }
}

fn element_bits(code: u8) -> Option<u64> {
    match code {
        b'X' => Some(1),
        _ => element_bytes(code).map(|bytes| bytes * 8),
    }
}

struct Descriptor {
    bytes: usize,
    element_bits: u64,
}

struct ColumnFormat {
    width: u64,
    descriptor: Option<Descriptor>,
}

fn column_format(tform: &str) -> Option<ColumnFormat> {
    let spec = tform.trim();
    let spec = spec.split_once('(').map_or(spec, |(head, _)| head);
    let digits = spec.bytes().take_while(u8::is_ascii_digit).count();
    let repeat: u64 = if digits == 0 {
        1
    } else {
        spec[..digits].parse().ok()?
    };
    let fixed = |width: Option<u64>| {
        Some(ColumnFormat {
            width: width?,
            descriptor: None,
        })
    };
    match spec.as_bytes()[digits..] {
        [kind @ (b'P' | b'Q'), code] => {
            let bytes: usize = if kind == b'P' { 8 } else { 16 };
            Some(ColumnFormat {
                width: repeat.checked_mul(bytes as u64)?,
                descriptor: Some(Descriptor {
                    bytes,
                    element_bits: element_bits(code)?,
                }),
            })
        }
        [b'X'] => fixed(Some(repeat.div_ceil(8))),
        [code] => fixed(repeat.checked_mul(element_bytes(code)?)),
        _ => None,
    }
}

fn quantization_column_width(name: &str) -> Option<u64> {
    match name {
        "ZSCALE" | "ZZERO" => Some(8),
        "ZBLANK" => Some(4),
        _ => None,
    }
}

struct HeapColumn {
    field: i64,
    name: String,
    offset: u64,
    descriptor: Descriptor,
}

struct TableShape {
    naxis1: u64,
    naxis2: u64,
    heap_columns: Vec<HeapColumn>,
}

fn table_shape(header: &HduHeader) -> Result<TableShape> {
    let naxis = required(header, "NAXIS")?;
    if naxis != 2 {
        bail!(
            "compressed image header: NAXIS = {naxis}, a tile-compressed image table has NAXIS = 2"
        );
    }
    let tfields = required(header, "TFIELDS")?;
    if !(1..=MAX_FIELDS).contains(&tfields) {
        bail!("compressed image header: TFIELDS = {tfields} is outside 1..={MAX_FIELDS}");
    }
    let naxis1 = non_negative("NAXIS1", required(header, "NAXIS1")?)?;
    let naxis2 = non_negative("NAXIS2", required(header, "NAXIS2")?)?;
    let mut heap_columns = Vec::new();
    let mut row_width = 0u64;
    for n in 1..=tfields {
        let key = format!("TFORM{n}");
        let tform = single(header, &key)?
            .with_context(|| format!("compressed image header: {key} is missing"))?;
        let format = column_format(tform).with_context(|| {
            format!(
                "compressed image header: {key} = {tform:?} is not a binary table column format"
            )
        })?;
        let name = single(header, &format!("TTYPE{n}"))?
            .unwrap_or_default()
            .trim()
            .to_string();
        if let Some(expected) = quantization_column_width(&name) {
            if format.width != expected {
                bail!(
                    "compressed image header: TTYPE{n} = {name} needs a {expected}-byte column but {key} = {tform:?} is {} bytes wide",
                    format.width
                );
            }
        }
        if let Some(descriptor) = format.descriptor {
            heap_columns.push(HeapColumn {
                field: n,
                name,
                offset: row_width,
                descriptor,
            });
        }
        row_width = row_width
            .checked_add(format.width)
            .context("compressed image header: the table row width overflows")?;
    }
    if row_width != naxis1 {
        bail!("compressed image header: the TFORMn columns are {row_width} bytes wide but NAXIS1 = {naxis1}");
    }
    Ok(TableShape {
        naxis1,
        naxis2,
        heap_columns,
    })
}

fn image_axes(header: &HduHeader) -> Result<Vec<u64>> {
    let znaxis = required(header, "ZNAXIS")?;
    if !(1..=MAX_FITS_AXES).contains(&znaxis) {
        bail!("compressed image header: ZNAXIS = {znaxis} is outside 1..={MAX_FITS_AXES}");
    }
    (1..=znaxis)
        .map(|n| {
            let key = format!("ZNAXIS{n}");
            at_least_one(&key, required(header, &key)?)
        })
        .collect()
}

fn tile_shape(header: &HduHeader, axes: &[u64]) -> Result<Vec<u64>> {
    axes.iter()
        .enumerate()
        .map(|(index, &axis)| {
            let key = format!("ZTILE{}", index + 1);
            match integer(header, &key)? {
                Some(tile) => at_least_one(&key, tile),
                None if index == 0 => Ok(axis),
                None => Ok(1),
            }
        })
        .collect()
}

fn check_tile_count(axes: &[u64], tiles: &[u64], naxis2: u64) -> Result<()> {
    let count = axes
        .iter()
        .zip(tiles)
        .try_fold(1u64, |count, (&axis, &tile)| {
            count.checked_mul(axis.div_ceil(tile))
        })
        .context("compressed image header: the tile count overflows")?;
    if count != naxis2 {
        bail!("compressed image header: NAXIS2 = {naxis2} rows but ZNAXISn/ZTILEn describe {count} tiles");
    }
    Ok(())
}

fn is_rice(header: &HduHeader) -> Result<bool> {
    Ok(
        single(header, "ZCMPTYPE")?
            .is_some_and(|name| matches!(name.trim(), "RICE_1" | "RICE_ONE")),
    )
}

fn check_rice(header: &HduHeader, tile_pixels: u64) -> Result<()> {
    let zval1 = integer(header, "ZVAL1")?.unwrap_or(32);
    let zval2 = integer(header, "ZVAL2")?.unwrap_or(4);
    let swapped = (0..16).contains(&zval1) && !(0..=8).contains(&zval2);
    let ((blocksize_key, blocksize), (bytepix_key, bytepix)) = if swapped {
        (("ZVAL2", zval2), ("ZVAL1", zval1))
    } else {
        (("ZVAL1", zval1), ("ZVAL2", zval2))
    };
    if blocksize < 1 {
        bail!("compressed image header: Rice BLOCKSIZE ({blocksize_key}) = {blocksize} must be at least 1");
    }
    if !RICE_BYTEPIX.contains(&bytepix) {
        bail!(
            "compressed image header: Rice BYTEPIX ({bytepix_key}) = {bytepix} is not 1, 2, 4 or 8"
        );
    }
    let fits_the_decoder = tile_pixels
        .checked_add(blocksize as u64)
        .is_some_and(|n| n <= i32::MAX as u64);
    if !fits_the_decoder {
        bail!("compressed image header: Rice tiles of {tile_pixels} pixels in blocks of {blocksize} exceed the 32-bit Rice decoder");
    }
    Ok(())
}

fn check_data_size(header: &HduHeader, naxis1: u64, naxis2: u64, available: u64) -> Result<u64> {
    let pcount = non_negative("PCOUNT", integer(header, "PCOUNT")?.unwrap_or(0))?;
    let needed = naxis1
        .checked_mul(naxis2)
        .and_then(|table| table.checked_add(pcount))
        .context("compressed image header: NAXIS1 x NAXIS2 + PCOUNT overflows")?;
    if needed > available {
        bail!(
            "compressed image header: NAXIS1 x NAXIS2 + PCOUNT = {needed} bytes but the file has {available} bytes after the header"
        );
    }
    Ok(pcount)
}

fn read_descriptor(entry: &[u8]) -> (u64, u64) {
    let (count, offset) = entry.split_at(entry.len() / 2);
    let unsigned = |bytes: &[u8]| bytes.iter().fold(0u64, |n, &b| (n << 8) | u64::from(b));
    (unsigned(count), unsigned(offset))
}

impl HeapColumn {
    fn label(&self) -> String {
        match self.name.as_str() {
            "" => format!("column {}", self.field),
            name => format!("column {} ({name})", self.field),
        }
    }
}

impl TableShape {
    fn heap_column(&self, name: &str) -> Option<&HeapColumn> {
        self.heap_columns.iter().find(|column| column.name == name)
    }
}

fn heap_range(
    data: &[u8],
    table: &TableShape,
    column: &HeapColumn,
    row: u64,
    pcount: u64,
) -> Result<(u64, u64)> {
    let at = row * table.naxis1 + column.offset;
    let entry = usize::try_from(at)
        .ok()
        .and_then(|at| data.get(at..)?.get(..column.descriptor.bytes))
        .with_context(|| {
            format!(
                "compressed image table: row {} of {} runs past the end of the file",
                row + 1,
                column.label()
            )
        })?;
    let (count, offset) = read_descriptor(entry);
    let end = count
        .checked_mul(column.descriptor.element_bits)
        .and_then(|bits| bits.div_ceil(8).checked_add(offset));
    match end {
        Some(end) if end <= pcount => Ok((offset, end)),
        _ => bail!(
            "compressed image table: row {} of {} points past the {pcount}-byte heap (PCOUNT)",
            row + 1,
            column.label()
        ),
    }
}

fn check_descriptors(data: &[u8], table: &TableShape, pcount: u64) -> Result<()> {
    for column in &table.heap_columns {
        for row in 0..table.naxis2 {
            heap_range(data, table, column, row, pcount)?;
        }
    }
    Ok(())
}

fn tile_bytes<'a>(
    data: &'a [u8],
    table: &TableShape,
    column: &HeapColumn,
    row: u64,
    pcount: u64,
) -> Result<&'a [u8]> {
    let (start, end) = heap_range(data, table, column, row, pcount)?;
    let heap = table.naxis1 * table.naxis2;
    usize::try_from(heap + start)
        .ok()
        .zip(usize::try_from(heap + end).ok())
        .and_then(|(start, end)| data.get(start..end))
        .with_context(|| {
            format!(
                "compressed image table: row {} of {} runs past the end of the file",
                row + 1,
                column.label()
            )
        })
}

fn tile_pixels(axes: &[u64], tiles: &[u64], row: u64) -> u64 {
    let mut rest = row;
    axes.iter()
        .zip(tiles)
        .map(|(&axis, &tile)| {
            let count = axis.div_ceil(tile);
            let start = rest % count * tile;
            rest /= count;
            tile.min(axis - start)
        })
        .product()
}

const WIDEST_TILE_INTEGER: u64 = 4;
const INFLATE_SCRATCH: usize = 64 * 1024;

struct Inflated {
    bytes: u64,
    consumed: usize,
    complete: bool,
}

fn inflate_counting(stream: &[u8], zlib: bool, limit: u64) -> Inflated {
    let mut state = flate2::Decompress::new(zlib);
    let mut scratch = vec![0u8; limit.saturating_add(1).min(INFLATE_SCRATCH as u64) as usize];
    loop {
        let (before_in, before_out) = (state.total_in(), state.total_out());
        let input = stream.get(before_in as usize..).unwrap_or_default();
        let status = state.decompress(input, &mut scratch, flate2::FlushDecompress::None);
        let inflated = Inflated {
            bytes: state.total_out(),
            consumed: state.total_in() as usize,
            complete: matches!(status, Ok(flate2::Status::StreamEnd)),
        };
        let progressed = (state.total_in(), state.total_out()) != (before_in, before_out);
        if inflated.complete || status.is_err() || !progressed || inflated.bytes > limit {
            return inflated;
        }
    }
}

fn gzip_member_body(member: &[u8]) -> Option<usize> {
    if member.len() < 18 || member[..3] != [0x1f, 0x8b, 8] {
        return None;
    }
    let flags = member[3];
    let mut at = 10usize;
    if flags & 0x04 != 0 {
        let extra = member.get(at..at + 2)?;
        at += 2 + usize::from(u16::from_le_bytes([extra[0], extra[1]]));
    }
    for name_or_comment in [0x08, 0x10] {
        if flags & name_or_comment != 0 {
            at += member.get(at..)?.iter().position(|&b| b == 0)? + 1;
        }
    }
    if flags & 0x02 != 0 {
        at += 2;
    }
    (at < member.len()).then_some(at)
}

fn gzip_inflated_bytes(stream: &[u8], limit: u64) -> u64 {
    let mut total = 0u64;
    let mut rest = stream;
    while let Some(body) = gzip_member_body(rest) {
        let member = inflate_counting(&rest[body..], false, limit - total);
        total += member.bytes;
        let next = body + member.consumed + 8;
        if total > limit || !member.complete || next > rest.len() {
            break;
        }
        rest = &rest[next..];
        if rest.iter().all(|&b| b == 0) {
            break;
        }
    }
    total
}

fn inflated_bytes(stream: &[u8], limit: u64) -> u64 {
    if stream.starts_with(&[0x1f, 0x8b]) {
        return gzip_inflated_bytes(stream, limit);
    }
    inflate_counting(stream, true, limit)
        .bytes
        .max(inflate_counting(stream, false, limit).bytes)
}

fn check_inflation(tile: &[u8], limit: u64, row: u64, column: &HeapColumn) -> Result<()> {
    if inflated_bytes(tile, limit) > limit {
        bail!(
            "compressed image table: row {} of {} inflates past the {limit} bytes its tile can hold",
            row + 1,
            column.label()
        );
    }
    Ok(())
}

const HCOMPRESS_LONGEST_SAFE_LINE: i32 = 4;

fn check_hcompress_shape(tile: &[u8], row: u64) -> Result<()> {
    let mut head = [0u8; 10];
    let known = tile.len().min(head.len());
    head[..known].copy_from_slice(&tile[..known]);
    let rows = i32::from_be_bytes([head[2], head[3], head[4], head[5]]);
    let columns = i32::from_be_bytes([head[6], head[7], head[8], head[9]]);
    if rows.min(columns) == 1 && rows.max(columns) > HCOMPRESS_LONGEST_SAFE_LINE {
        bail!(
            "compressed image table: row {} holds a {rows} x {columns} HCOMPRESS tile; the decoder cannot expand a tile one pixel wide and more than {HCOMPRESS_LONGEST_SAFE_LINE} long",
            row + 1
        );
    }
    Ok(())
}

fn check_tiles(
    header: &HduHeader,
    data: &[u8],
    table: &TableShape,
    axes: &[u64],
    tiles: &[u64],
    pcount: u64,
) -> Result<()> {
    let codec = single(header, "ZCMPTYPE")?.unwrap_or_default().trim();
    let widest = stored_bytes(required(header, "ZBITPIX")?)?.max(WIDEST_TILE_INTEGER);
    let compressed = table.heap_column("COMPRESSED_DATA");
    let gzip_fallback = table.heap_column("GZIP_COMPRESSED_DATA");
    let check_row = |row: u64| -> Result<()> {
        let limit = tile_pixels(axes, tiles, row).saturating_mul(widest);
        if let Some(column) = compressed {
            let tile = tile_bytes(data, table, column, row, pcount)?;
            match codec {
                "GZIP_1" | "GZIP_2" => check_inflation(tile, limit, row, column)?,
                "HCOMPRESS_1" => check_hcompress_shape(tile, row)?,
                _ => {}
            }
        }
        if let Some(column) = gzip_fallback {
            check_inflation(
                tile_bytes(data, table, column, row, pcount)?,
                limit,
                row,
                column,
            )?;
        }
        Ok(())
    };
    match (0..table.naxis2)
        .into_par_iter()
        .find_map_first(|row| check_row(row).err())
    {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

fn stored_bytes(zbitpix: i64) -> Result<u64> {
    match zbitpix {
        8 => Ok(1),
        16 => Ok(2),
        32 | -32 => Ok(4),
        64 | -64 => Ok(8),
        other => bail!("compressed image header: ZBITPIX = {other} is not a FITS pixel type"),
    }
}

fn check_memory_budget(header: &HduHeader, axes: &[u64], memory: Option<MemoryInfo>) -> Result<()> {
    let stored = stored_bytes(required(header, "ZBITPIX")?)?;
    let pixels = axes
        .iter()
        .try_fold(1u64, |total, &axis| total.checked_mul(axis))
        .context("compressed image header: the image size overflows")?;
    let planes: u64 = axes.iter().skip(2).product();
    let per_pixel = F32_BYTES + stored + if planes > 1 { F32_BYTES } else { 0 };
    let needed = pixels.saturating_mul(per_pixel);
    let available = memory.map_or(isize::MAX as u64, |info| info.commit_available);
    if needed > available {
        bail!(
            "compressed image header: decoding {pixels} pixels needs about {} of memory but only {} is available. If the system can page this much, set {MEMORY_BUDGET_ENV} to the memory to allow (in MB) and restart.",
            format_bytes(needed),
            format_bytes(available)
        );
    }
    Ok(())
}

fn is_printable_ascii(text: &str) -> bool {
    text.bytes().all(|b| (b' '..=b'~').contains(&b))
}

fn is_decoder_key(key: &str) -> bool {
    DECODER_KEYS.contains(&key)
        || DECODER_INDEXED_KEYS.iter().any(|prefix| {
            key.strip_prefix(prefix)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
}

fn reject_non_ascii(header: &HduHeader) -> Result<()> {
    for (key, value) in &header.cards {
        if !is_printable_ascii(key) {
            let readable: String = key.chars().filter(char::is_ascii_graphic).collect();
            if is_decoder_key(&readable) {
                bail!("compressed image header: the keyword {key:?} has bytes outside printable ASCII");
            }
        } else if is_decoder_key(key) && !is_printable_ascii(value) {
            bail!("compressed image header: {key} = {value:?} has bytes outside printable ASCII");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::Path;

    use super::*;
    use crate::infra::fits::reader::{load_fits_image, parse_header_at, try_extract_rgb_mmap};
    use crate::infra::fits::writer::{write_fits_mono_rice, write_fits_rgb_rice};

    const CARD: usize = 80;

    fn ramp(rows: usize, cols: usize) -> Array2<f32> {
        Array2::from_shape_fn((rows, cols), |(y, x)| (y * cols + x) as f32)
    }

    fn rice_mono_file(dir: &Path) -> Vec<u8> {
        let path = dir.join("rice_mono.fits");
        write_fits_mono_rice(path.to_str().unwrap(), &ramp(16, 16), None, 16, 0.0).unwrap();
        std::fs::read(path).unwrap()
    }

    fn rice_rgb_file(dir: &Path) -> Vec<u8> {
        let path = dir.join("rice_rgb.fits");
        let plane = ramp(16, 16);
        write_fits_rgb_rice(
            path.to_str().unwrap(),
            &plane,
            &plane,
            &plane,
            None,
            16,
            0.0,
        )
        .unwrap();
        std::fs::read(path).unwrap()
    }

    fn compressed_header_start(bytes: &[u8]) -> usize {
        parse_header_at(bytes, 0).unwrap().next_hdu_offset
    }

    fn card_position(bytes: &[u8], start: usize, key: &str) -> Option<usize> {
        let name = format!("{key:<8}");
        bytes[start..]
            .chunks_exact(CARD)
            .take_while(|card| !card.starts_with(b"END     "))
            .position(|card| card[..8] == *name.as_bytes())
            .map(|index| start + index * CARD)
    }

    fn end_position(bytes: &[u8], start: usize) -> usize {
        start
            + bytes[start..]
                .chunks_exact(CARD)
                .position(|card| card.starts_with(b"END     "))
                .unwrap()
                * CARD
    }

    fn value_card(key: &str, value: &[u8]) -> Vec<u8> {
        let mut card = format!("{key:<8}= ").into_bytes();
        card.extend_from_slice(value);
        card.resize(CARD, b' ');
        card
    }

    fn insert_before_end(bytes: &mut [u8], start: usize) -> usize {
        let end = end_position(bytes, start);
        assert!(
            (end + CARD - start) % 2880 != 0,
            "no room for another card in the header block"
        );
        bytes.copy_within(end..end + CARD, end + CARD);
        end
    }

    fn with_card(mut bytes: Vec<u8>, key: &str, value: &[u8]) -> Vec<u8> {
        let start = compressed_header_start(&bytes);
        let at = match card_position(&bytes, start, key) {
            Some(at) => at,
            None => insert_before_end(&mut bytes, start),
        };
        bytes[at..at + CARD].copy_from_slice(&value_card(key, value));
        bytes
    }

    fn with_extra_card(mut bytes: Vec<u8>, key: &str, value: &[u8]) -> Vec<u8> {
        let start = compressed_header_start(&bytes);
        let at = insert_before_end(&mut bytes, start);
        bytes[at..at + CARD].copy_from_slice(&value_card(key, value));
        bytes
    }

    fn with_cards(bytes: Vec<u8>, cards: &[(&str, &str)]) -> Vec<u8> {
        cards.iter().fold(bytes, |bytes, (key, value)| {
            with_card(bytes, key, value.as_bytes())
        })
    }

    fn assert_refused(cards: &[(&str, &str)], needles: &[&str]) {
        let dir = tempfile::tempdir().unwrap();
        let bytes = with_cards(rice_mono_file(dir.path()), cards);
        let err = load_error(dir.path(), &bytes);
        for needle in needles {
            assert!(err.contains(needle), "{needle:?} missing from {err}");
        }
    }

    fn compressed_hdu(bytes: &[u8]) -> crate::infra::fits::reader::ParsedHdu {
        parse_header_at(bytes, compressed_header_start(bytes)).unwrap()
    }

    fn memory(bytes: u64) -> Option<MemoryInfo> {
        Some(MemoryInfo {
            physical_available: bytes,
            commit_available: bytes,
        })
    }

    fn load(dir: &Path, bytes: &[u8]) -> Result<Array2<f32>> {
        let path = dir.join("patched.fits");
        std::fs::write(&path, bytes).unwrap();
        load_fits_image(path.to_str().unwrap())
    }

    fn load_error(dir: &Path, bytes: &[u8]) -> String {
        match load(dir, bytes) {
            Ok(image) => panic!(
                "the patched file must be refused, it decoded to {:?}",
                image.dim()
            ),
            Err(err) => format!("{err:#}"),
        }
    }

    #[test]
    fn a_non_ascii_tform_byte_is_refused_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = with_card(rice_mono_file(dir.path()), "TFORM1", b"'1B\xDA'");
        let err = load_error(dir.path(), &bytes);
        assert!(
            err.contains("TFORM1") && err.contains("printable ASCII"),
            "{err}"
        );
    }

    #[test]
    fn a_non_ascii_byte_in_a_compressed_rgb_cube_is_refused_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = with_card(rice_rgb_file(dir.path()), "TFORM1", b"'1P\xC3\x89'");
        let path = dir.path().join("patched_rgb.fits");
        std::fs::write(&path, &bytes).unwrap();
        let err = match try_extract_rgb_mmap(&File::open(&path).unwrap()) {
            Ok(_) => panic!("the patched cube must be refused"),
            Err(err) => format!("{err:#}"),
        };
        assert!(
            err.contains("TFORM1") && err.contains("printable ASCII"),
            "{err}"
        );
    }

    #[test]
    fn a_control_byte_in_a_compression_keyword_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = with_card(rice_mono_file(dir.path()), "ZCMPTYPE", b"'RICE_1\x07'");
        let err = load_error(dir.path(), &bytes);
        assert!(
            err.contains("ZCMPTYPE") && err.contains("printable ASCII"),
            "{err}"
        );
    }

    #[test]
    fn a_non_ascii_descriptive_value_does_not_block_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = with_card(
            rice_mono_file(dir.path()),
            "OBJECT",
            "'M31 \u{2013} Andromeda'".as_bytes(),
        );
        let image = load(dir.path(), &bytes).unwrap();
        assert_eq!(image.dim(), (16, 16));
        for (got, want) in image.iter().zip(ramp(16, 16).iter()) {
            assert!((got - want).abs() < 0.01, "{got} vs {want}");
        }
    }

    #[test]
    fn oversized_znaxis_with_a_mismatched_tile_grid_is_refused_before_allocating() {
        for size in ["200000", "40000"] {
            assert_refused(
                &[("ZNAXIS1", size), ("ZNAXIS2", size)],
                &["NAXIS2 = 16", "tiles"],
            );
        }
    }

    #[test]
    fn a_tile_grid_that_does_not_match_the_row_count_is_refused() {
        assert_refused(&[("ZTILE2", "2")], &["NAXIS2 = 16", "describe 8 tiles"]);
    }

    #[test]
    fn a_table_that_is_not_two_dimensional_is_refused() {
        assert_refused(&[("NAXIS", "3"), ("NAXIS3", "1")], &["NAXIS = 3"]);
    }

    #[test]
    fn a_field_count_outside_the_fits_range_is_refused() {
        for tfields in ["0", "1000"] {
            assert_refused(&[("TFIELDS", tfields)], &[&format!("TFIELDS = {tfields}")]);
        }
    }

    #[test]
    fn column_widths_must_add_up_to_naxis1() {
        assert_refused(&[("TFORM1", "'2PB(30)'")], &["16 bytes wide", "NAXIS1 = 8"]);
    }

    #[test]
    fn an_unparseable_column_format_is_refused_instead_of_panicking() {
        assert_refused(&[("TFORM1", "'(30)'")], &["TFORM1", "column format"]);
    }

    #[test]
    fn a_tile_size_below_one_is_refused() {
        for ztile in ["0", "-16"] {
            assert_refused(&[("ZTILE1", ztile)], &[&format!("ZTILE1 = {ztile}")]);
        }
    }

    fn load_within(dir: &Path, bytes: &[u8], limit: std::time::Duration) -> Result<Array2<f32>> {
        let path = dir.join("patched.fits");
        std::fs::write(&path, bytes).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(load_fits_image(path.to_str().unwrap()));
        });
        receiver
            .recv_timeout(limit)
            .expect("decoding did not return: the Rice decoder never advances")
    }

    #[test]
    fn a_rice_blocksize_below_one_is_refused_instead_of_looping_forever() {
        for blocksize in ["0", "-1"] {
            let dir = tempfile::tempdir().unwrap();
            let bytes = with_cards(rice_mono_file(dir.path()), &[("ZVAL1", blocksize)]);
            let err = match load_within(dir.path(), &bytes, std::time::Duration::from_secs(20)) {
                Ok(image) => panic!(
                    "BLOCKSIZE = {blocksize} must be refused, it decoded to {:?}",
                    image.dim()
                ),
                Err(err) => format!("{err:#}"),
            };
            assert!(err.contains("BLOCKSIZE"), "{err}");
        }
    }

    #[test]
    fn a_rice_bytepix_outside_1_2_4_8_is_refused() {
        for bytepix in ["3", "0"] {
            assert_refused(
                &[("ZVAL2", bytepix)],
                &[&format!("BYTEPIX (ZVAL2) = {bytepix}")],
            );
        }
    }

    #[test]
    fn a_heap_that_runs_past_the_end_of_the_file_is_refused() {
        assert_refused(
            &[("PCOUNT", "10000000")],
            &["PCOUNT", "bytes after the header"],
        );
    }

    #[test]
    fn a_decoded_size_beyond_the_memory_budget_is_refused_before_allocating() {
        let side = "1000000";
        assert_refused(
            &[
                ("ZCMPTYPE", "'GZIP_1'"),
                ("NAXIS2", "1"),
                ("ZNAXIS1", side),
                ("ZNAXIS2", side),
                ("ZTILE1", side),
                ("ZTILE2", side),
            ],
            &[
                "1000000000000 pixels",
                "memory",
                "ASTROBURST_MEMORY_BUDGET_MB",
            ],
        );
    }

    #[test]
    fn the_memory_budget_counts_the_output_and_the_decoder_copy() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = rice_mono_file(dir.path());
        let hdu = compressed_hdu(&bytes);
        let needed = 16 * 16 * (4 + 2);
        let err =
            validate_layout(&bytes, &hdu.header, hdu.data_start, memory(needed - 1)).unwrap_err();
        assert!(err.to_string().contains("memory"), "{err}");
        validate_layout(&bytes, &hdu.header, hdu.data_start, memory(needed)).unwrap();
    }

    #[test]
    fn a_missing_or_non_positive_image_axis_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let rice = rice_mono_file(dir.path());
        for (value, needle) in [("-1", "ZNAXIS1 = -1"), ("'16'", "ZNAXIS1 is missing")] {
            let bytes = with_card(rice.clone(), "ZNAXIS1", value.as_bytes());
            let hdu = compressed_hdu(&bytes);
            let err = validate_layout(&bytes, &hdu.header, hdu.data_start, None).unwrap_err();
            assert!(
                err.to_string().contains(needle),
                "{needle:?} missing from {err}"
            );
        }
    }

    #[test]
    fn a_duplicated_layout_keyword_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = with_extra_card(rice_mono_file(dir.path()), "ZNAXIS1", b"200000");
        let err = load_error(dir.path(), &bytes);
        assert!(err.contains("ZNAXIS1 appears more than once"), "{err}");
    }

    #[test]
    fn integer_pixels_matching_zblank_without_blank_decode_as_nan() {
        let nulls = [(0usize, 0usize), (3, 7), (15, 15)];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zblank_only.fits");
        let mut data = ramp(16, 16);
        for &(y, x) in &nulls {
            data[[y, x]] = f32::NAN;
        }
        write_fits_mono_rice(path.to_str().unwrap(), &data, None, 16, 0.0).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let start = compressed_header_start(&bytes);
        assert_eq!(card_position(&bytes, start, "ZBLANK"), None);
        let blank = card_position(&bytes, start, "BLANK")
            .expect("the int16 writer stores its null value in BLANK");
        bytes[blank..blank + 8].copy_from_slice(b"ZBLANK  ");

        let image = load(dir.path(), &bytes).unwrap();
        for &(y, x) in &nulls {
            assert!(
                image[[y, x]].is_nan(),
                "pixel ({y}, {x}) = {} must be NaN",
                image[[y, x]]
            );
        }
        assert_eq!(image.iter().filter(|v| v.is_nan()).count(), nulls.len());
    }

    #[test]
    fn non_dividing_tiles_and_cubes_still_decode() {
        use fitsio_pure::compress::{compress_image_hdu, TileCompression};

        let dir = tempfile::tempdir().unwrap();
        let (cols, rows, planes) = (300usize, 217usize, 2usize);
        let pixels: Vec<i16> = (0..cols * rows * planes)
            .map(|i| (i % 4093) as i16 - 2000)
            .collect();
        let hdu = compress_image_hdu(
            &[cols, rows, planes],
            &pixels,
            &TileCompression::rice().tile_dims(&[37, 16, 1]),
            &[],
        )
        .unwrap();
        let mut file: Vec<u8> = [
            "SIMPLE  = T",
            "BITPIX  = 8",
            "NAXIS   = 0",
            "EXTEND  = T",
            "END",
        ]
        .iter()
        .flat_map(|card| format!("{card:<80}").into_bytes())
        .collect();
        file.resize(2880, b' ');
        file.extend_from_slice(&hdu);
        let path = dir.path().join("tiles_37x16.fits");
        std::fs::write(&path, &file).unwrap();

        let image = load_fits_image(&format!("{}#hdu=1", path.to_str().unwrap())).unwrap();
        assert_eq!(image.dim(), (rows, cols));
        for (got, want) in image.iter().zip(&pixels[..cols * rows]) {
            assert_eq!(*got, f32::from(*want));
        }
    }

    type Cards = Vec<(String, String)>;

    fn header_block(cards: &[(String, String)]) -> Vec<u8> {
        let mut block: Vec<u8> = cards
            .iter()
            .map(|(key, value)| format!("{key:<8}= {value}"))
            .chain(["END".to_string()])
            .flat_map(|card| format!("{card:<80}").into_bytes())
            .collect();
        block.resize(block.len().next_multiple_of(2880), b' ');
        block
    }

    fn owned(cards: &[(&str, &str)]) -> Cards {
        cards
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn unpadded_table_file(cards: &[(String, String)], data: &[u8]) -> Vec<u8> {
        let mut file = header_block(&owned(&[
            ("SIMPLE", "T"),
            ("BITPIX", "8"),
            ("NAXIS", "0"),
            ("EXTEND", "T"),
        ]));
        file.extend(header_block(cards));
        file.extend_from_slice(data);
        file
    }

    fn table_file(cards: &[(String, String)], data: &[u8]) -> Vec<u8> {
        let mut file = unpadded_table_file(cards, data);
        file.resize(file.len().next_multiple_of(2880), 0);
        file
    }

    fn table_cards(columns: &[(&str, &str)], naxis1: usize, naxis2: usize, pcount: usize) -> Cards {
        let mut cards = owned(&[
            ("XTENSION", "'BINTABLE'"),
            ("BITPIX", "8"),
            ("NAXIS", "2"),
            ("NAXIS1", &naxis1.to_string()),
            ("NAXIS2", &naxis2.to_string()),
            ("PCOUNT", &pcount.to_string()),
            ("GCOUNT", "1"),
            ("TFIELDS", &columns.len().to_string()),
        ]);
        for (index, (name, tform)) in columns.iter().enumerate() {
            cards.push((format!("TTYPE{}", index + 1), format!("'{name}'")));
            cards.push((format!("TFORM{}", index + 1), format!("'{tform}'")));
        }
        cards
    }

    fn image_cards(zcmptype: &str, zbitpix: i64, axes: &[(u64, u64)]) -> Cards {
        let mut cards = owned(&[
            ("ZIMAGE", "T"),
            ("ZCMPTYPE", &format!("'{zcmptype}'")),
            ("ZBITPIX", &zbitpix.to_string()),
            ("ZNAXIS", &axes.len().to_string()),
        ]);
        for (index, (axis, tile)) in axes.iter().enumerate() {
            cards.push((format!("ZNAXIS{}", index + 1), axis.to_string()));
            cards.push((format!("ZTILE{}", index + 1), tile.to_string()));
        }
        cards
    }

    fn rice_image_cards(zbitpix: i64, bytepix: u8, axes: &[(u64, u64)]) -> Cards {
        let mut cards = image_cards("RICE_1", zbitpix, axes);
        cards.extend(owned(&[
            ("ZNAME1", "'BLOCKSIZE'"),
            ("ZVAL1", "32"),
            ("ZNAME2", "'BYTEPIX'"),
            ("ZVAL2", &bytepix.to_string()),
        ]));
        cards
    }

    const ONE_TILE_4X4: [(u64, u64); 2] = [(4, 4), (4, 4)];
    const INT16_SEVEN: [u8; 2] = 7i16.to_be_bytes();

    fn q_descriptor_file(count: u64, offset: u64) -> Vec<u8> {
        let cards = [
            table_cards(&[("COMPRESSED_DATA", "1QB(2)")], 16, 1, INT16_SEVEN.len()),
            rice_image_cards(16, 2, &ONE_TILE_4X4),
        ]
        .concat();
        let data = [
            &count.to_be_bytes()[..],
            &offset.to_be_bytes(),
            &INT16_SEVEN,
        ]
        .concat();
        table_file(&cards, &data)
    }

    #[test]
    fn a_tile_table_with_64_bit_descriptors_still_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let image = load(dir.path(), &q_descriptor_file(2, 0)).unwrap();
        assert_eq!(image.dim(), (4, 4));
        assert!(image.iter().all(|&v| v == 7.0), "{image:?}");
    }

    #[test]
    fn a_wrapping_64_bit_descriptor_offset_is_refused_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let probe = q_descriptor_file(2, 0);
        let heap_start = (compressed_hdu(&probe).data_start + 16) as u64;
        let bytes = q_descriptor_file(20, u64::MAX - 5 - heap_start);
        let err = load_error(dir.path(), &bytes);
        assert!(
            err.contains("row 1") && err.contains("COMPRESSED_DATA") && err.contains("2-byte heap"),
            "{err}"
        );
    }

    #[test]
    fn a_32_bit_descriptor_past_the_heap_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cards = [
            table_cards(&[("COMPRESSED_DATA", "1PB(2)")], 8, 1, INT16_SEVEN.len()),
            rice_image_cards(16, 2, &ONE_TILE_4X4),
        ]
        .concat();
        let data = [&2u32.to_be_bytes()[..], &2u32.to_be_bytes(), &INT16_SEVEN].concat();
        let err = load_error(dir.path(), &table_file(&cards, &data));
        assert!(
            err.contains("row 1") && err.contains("2-byte heap"),
            "{err}"
        );
    }

    fn one_tile_file(zcmptype: &str, columns: &[(&str, &[u8])]) -> Vec<u8> {
        let tforms: Vec<String> = columns
            .iter()
            .map(|(_, heap)| format!("1PB({})", heap.len()))
            .collect();
        let layout: Vec<(&str, &str)> = columns
            .iter()
            .zip(&tforms)
            .map(|((name, _), tform)| (*name, tform.as_str()))
            .collect();
        let heap: Vec<u8> = columns
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        let cards = [
            table_cards(&layout, 8 * columns.len(), 1, heap.len()),
            image_cards(zcmptype, 16, &ONE_TILE_4X4),
        ]
        .concat();
        let mut offset = 0u32;
        let mut data = Vec::new();
        for (_, bytes) in columns {
            let count = bytes.len() as u32;
            data.extend_from_slice(&count.to_be_bytes());
            data.extend_from_slice(&offset.to_be_bytes());
            offset += count;
        }
        data.extend_from_slice(&heap);
        table_file(&cards, &data)
    }

    fn encoded<W: std::io::Write>(mut encoder: W, raw: &[u8]) -> W {
        encoder.write_all(raw).unwrap();
        encoder
    }

    fn gzip(raw: &[u8]) -> Vec<u8> {
        encoded(
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best()),
            raw,
        )
        .finish()
        .unwrap()
    }

    fn zlib(raw: &[u8]) -> Vec<u8> {
        encoded(
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best()),
            raw,
        )
        .finish()
        .unwrap()
    }

    fn deflate(raw: &[u8]) -> Vec<u8> {
        encoded(
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best()),
            raw,
        )
        .finish()
        .unwrap()
    }

    fn ramp_i16(width: usize) -> Vec<u8> {
        (0..16i32)
            .flat_map(|v| match width {
                2 => (v as i16).to_be_bytes().to_vec(),
                _ => v.to_be_bytes().to_vec(),
            })
            .collect()
    }

    const INFLATED_BOMB: usize = 16 << 20;

    #[test]
    fn a_gzip_tile_that_inflates_past_its_tile_is_refused_before_decoding() {
        let bomb = vec![0u8; INFLATED_BOMB];
        let two_members = [gzip(&ramp_i16(2)), gzip(&bomb)].concat();
        let cases: [(&str, &str, Vec<u8>); 5] = [
            ("gzip", "GZIP_1", gzip(&bomb)),
            ("gzip", "GZIP_2", gzip(&bomb)),
            ("zlib", "GZIP_1", zlib(&bomb)),
            ("raw deflate", "GZIP_1", deflate(&bomb)),
            ("second gzip member", "GZIP_1", two_members),
        ];
        for (framing, zcmptype, tile) in cases {
            let dir = tempfile::tempdir().unwrap();
            let bytes = one_tile_file(zcmptype, &[("COMPRESSED_DATA", &tile)]);
            let err = load_error(dir.path(), &bytes);
            assert!(
                err.contains("row 1 of column 1 (COMPRESSED_DATA)") && err.contains("64 bytes"),
                "{framing} {zcmptype}: {err}"
            );
        }
    }

    #[test]
    fn a_gzip_fallback_tile_that_inflates_past_its_tile_is_refused_before_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let bomb = gzip(&vec![0u8; INFLATED_BOMB]);
        let bytes = one_tile_file(
            "RICE_1",
            &[("COMPRESSED_DATA", &[]), ("GZIP_COMPRESSED_DATA", &bomb)],
        );
        let err = load_error(dir.path(), &bytes);
        assert!(
            err.contains("row 1 of column 2 (GZIP_COMPRESSED_DATA)") && err.contains("64 bytes"),
            "{err}"
        );
    }

    #[test]
    fn gzip_tiles_up_to_their_widest_pixel_encoding_still_decode() {
        let wanted: Vec<f32> = (0..16).map(|v| v as f32).collect();
        let shuffled_int16: Vec<u8> = {
            let plain = ramp_i16(2);
            [plain.iter().step_by(2), plain[1..].iter().step_by(2)]
                .into_iter()
                .flatten()
                .copied()
                .collect()
        };
        let cases: [(&str, &str, Vec<u8>); 5] = [
            ("int16 gzip", "GZIP_1", gzip(&ramp_i16(2))),
            ("int16 stored as int32", "GZIP_1", gzip(&ramp_i16(4))),
            ("int16 zlib", "GZIP_1", zlib(&ramp_i16(2))),
            ("int16 raw deflate", "GZIP_1", deflate(&ramp_i16(2))),
            ("int16 shuffled", "GZIP_2", gzip(&shuffled_int16)),
        ];
        for (label, zcmptype, tile) in cases {
            let dir = tempfile::tempdir().unwrap();
            let image = load(
                dir.path(),
                &one_tile_file(zcmptype, &[("COMPRESSED_DATA", &tile)]),
            )
            .unwrap_or_else(|err| panic!("{label}: {err:#}"));
            assert_eq!(image.iter().copied().collect::<Vec<_>>(), wanted, "{label}");
        }
    }

    fn hcompress_file(rows: i32, columns: i32, bit_planes: [u8; 3], body: &[u8]) -> Vec<u8> {
        let stream = [
            &[0xDD, 0x99][..],
            &rows.to_be_bytes(),
            &columns.to_be_bytes(),
            &0i32.to_be_bytes(),
            &0i64.to_be_bytes(),
            &bit_planes,
            body,
        ]
        .concat();
        let pixels = u64::try_from(rows * columns).unwrap();
        let tform = format!("1PB({})", stream.len());
        let cards = [
            table_cards(&[("COMPRESSED_DATA", &tform)], 8, 2, stream.len()),
            image_cards("HCOMPRESS_1", 16, &[(pixels, pixels), (2, 1)]),
        ]
        .concat();
        let descriptor = [
            &(stream.len() as u32).to_be_bytes()[..],
            &0u32.to_be_bytes(),
        ]
        .concat();
        table_file(&cards, &[&descriptor[..], &descriptor, &stream].concat())
    }

    #[test]
    fn a_one_pixel_wide_hcompress_tile_is_refused_instead_of_panicking() {
        let cases = [
            (6, 1, [0, 1, 0], &[0xF0, 0, 0, 0][..]),
            (1, 6, [0, 1, 0], &[0x00, 0x0F, 0, 0][..]),
        ];
        for (rows, columns, bit_planes, body) in cases {
            let dir = tempfile::tempdir().unwrap();
            let err = load_error(dir.path(), &hcompress_file(rows, columns, bit_planes, body));
            assert!(
                err.contains("row 1")
                    && err.contains(&format!("{rows} x {columns} HCOMPRESS tile")),
                "{rows}x{columns}: {err}"
            );
        }
    }

    #[test]
    fn narrow_hcompress_tiles_the_decoder_can_expand_still_decode() {
        for (rows, columns) in [(1, 4), (4, 1), (2, 3)] {
            let dir = tempfile::tempdir().unwrap();
            let image = load(
                dir.path(),
                &hcompress_file(rows, columns, [0, 0, 0], &[0; 4]),
            )
            .unwrap_or_else(|err| panic!("{rows}x{columns}: {err:#}"));
            assert_eq!(image.dim(), (2, (rows * columns) as usize));
            assert!(
                image.iter().all(|&v| v == 0.0),
                "{rows}x{columns}: {image:?}"
            );
        }
    }

    #[test]
    fn a_tile_size_that_overflows_is_refused_instead_of_panicking() {
        let side = 1u64 << 22;
        let cards = [
            table_cards(&[("COMPRESSED_DATA", "1PB")], 8, 1, 0),
            rice_image_cards(16, 2, &[(side, side); 3]),
        ]
        .concat();
        let bytes = table_file(&cards, &[0; 8]);
        let hdu = compressed_hdu(&bytes);
        let err = validate_layout(&bytes, &hdu.header, hdu.data_start, None).unwrap_err();
        assert!(err.to_string().contains("tile size overflows"), "{err}");
    }

    #[test]
    fn a_quantization_column_narrower_than_its_value_is_refused() {
        let float_cards = |column: (&str, &str), naxis1: usize, keywords: &[(&str, &str)]| {
            [
                table_cards(&[("COMPRESSED_DATA", "1PB(4)"), column], naxis1, 1, 4),
                rice_image_cards(-32, 4, &ONE_TILE_4X4),
                owned(&[("ZQUANTIZ", "'NO_DITHER'")]),
                owned(keywords),
            ]
            .concat()
        };
        let cases = [
            (("ZZERO", "1B"), 9, vec![("ZSCALE", "1.0")], "8-byte"),
            (("ZSCALE", "1E"), 12, vec![("ZZERO", "0.0")], "8-byte"),
            (
                ("ZBLANK", "1I"),
                10,
                vec![("ZSCALE", "1.0"), ("ZZERO", "0.0")],
                "4-byte",
            ),
        ];
        for (column, naxis1, keywords, needle) in cases {
            let dir = tempfile::tempdir().unwrap();
            let mut data = [&4u32.to_be_bytes()[..], &0u32.to_be_bytes()].concat();
            data.resize(naxis1, 0);
            data.extend_from_slice(&7i32.to_be_bytes());
            let bytes = unpadded_table_file(&float_cards(column, naxis1, &keywords), &data);
            let err = load_error(dir.path(), &bytes);
            assert!(
                err.contains(column.0) && err.contains(needle),
                "{column:?}: {err}"
            );
        }
    }

    fn with_raw_card(mut bytes: Vec<u8>, card: &[u8]) -> Vec<u8> {
        let start = compressed_header_start(&bytes);
        let at = insert_before_end(&mut bytes, start);
        let mut padded = card.to_vec();
        padded.resize(CARD, b' ');
        bytes[at..at + CARD].copy_from_slice(&padded);
        bytes
    }

    #[test]
    fn a_non_ascii_byte_in_a_descriptive_keyword_does_not_block_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = with_raw_card(rice_mono_file(dir.path()), b"OBS\xC9RV  = 'x'");
        let image = load(dir.path(), &bytes).unwrap();
        assert_eq!(image.dim(), (16, 16));
        for (got, want) in image.iter().zip(ramp(16, 16).iter()) {
            assert!((got - want).abs() < 0.01, "{got} vs {want}");
        }
    }

    #[test]
    fn a_non_ascii_byte_in_a_decoder_keyword_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = rice_mono_file(dir.path());
        let start = compressed_header_start(&bytes);
        let at = card_position(&bytes, start, "TFORM1").unwrap();
        bytes[at..at + 8].copy_from_slice(b"TFORM\xC91 ");
        let err = load_error(dir.path(), &bytes);
        assert!(
            err.contains("TFORM") && err.contains("printable ASCII"),
            "{err}"
        );
    }
}
