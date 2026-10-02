use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use image::ImageEncoder;
use ndarray::Array2;
use rayon::prelude::*;
use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, load_from_cache_or_disk, reduce_for_preview, resolve_output_dir, MAX_PREVIEW_DIM,
};
use crate::cmd::compose::rgb::remove_stale_pngs;
use crate::core::imaging::resample::resample_image;
use crate::core::imaging::sampling::{cell_range, preview_dims};
use crate::core::imaging::stats::{compute_image_stats, is_padding};
use crate::core::imaging::stf::{auto_stf, make_stf_u8_fn, AutoStfConfig};
use crate::infra::cache::ImageEntry;
use crate::types::constants::{
    RES_CHANNEL_PREVIEWS, RES_DIMENSIONS, RES_ELAPSED_MS, RES_PNG_PATH, RES_PREVIEW_DIMENSIONS,
    WIZARD_CACHE_PREFIX,
};
use crate::types::image::ImageStats;

const OVERLAY_PNG_PREFIX: &str = "channel_overlay";
const DEFAULT_MAX_DIM: usize = 2048;
const MIN_MAX_DIM: usize = 256;
const CHECKER_CELL: usize = 8;
const CHECKER_DARK: [u8; 3] = [26, 26, 32];
const CHECKER_LIGHT: [u8; 3] = [46, 46, 56];
pub(super) const ALIGNED_GONE: &str = "Aligned channels are no longer in memory; run Align again";

static OVERLAY_SEQ: AtomicU64 = AtomicU64::new(0);

pub(super) fn load_channel_entry(key: &str) -> anyhow::Result<ImageEntry> {
    load_from_cache_or_disk(key).map_err(|e| {
        if key.starts_with(WIZARD_CACHE_PREFIX) {
            anyhow::anyhow!(ALIGNED_GONE)
        } else {
            e
        }
    })
}

fn extra_mask_keys(keys: &[String], mask_keys: Vec<String>) -> Vec<String> {
    let mut extra: Vec<String> = Vec::new();
    for key in mask_keys {
        if !keys.contains(&key) && !extra.contains(&key) {
            extra.push(key);
        }
    }
    extra
}

fn contiguous(arr: &Array2<f32>) -> Cow<'_, Array2<f32>> {
    if arr.as_slice().is_some() {
        Cow::Borrowed(arr)
    } else {
        Cow::Owned(arr.as_standard_layout().into_owned())
    }
}

struct OverlayPlane<'a> {
    arr: Cow<'a, Array2<f32>>,
    stats: Cow<'a, ImageStats>,
}

fn overlay_plane(entry: &ImageEntry, rows: usize, cols: usize) -> anyhow::Result<OverlayPlane<'_>> {
    let arr = entry.arr();
    if arr.dim() == (rows, cols) {
        return Ok(OverlayPlane { arr: contiguous(arr), stats: Cow::Borrowed(entry.stats()) });
    }
    let resampled = resample_image(arr, rows, cols)?;
    let stats = compute_image_stats(&resampled);
    Ok(OverlayPlane { arr: Cow::Owned(resampled), stats: Cow::Owned(stats) })
}

fn mask_plane(entry: &ImageEntry, rows: usize, cols: usize) -> anyhow::Result<Cow<'_, Array2<f32>>> {
    let arr = entry.arr();
    if arr.dim() == (rows, cols) {
        Ok(contiguous(arr))
    } else {
        Ok(Cow::Owned(resample_image(arr, rows, cols)?))
    }
}

fn padding_at_preview(
    planes: &[&Array2<f32>],
    rows: usize,
    cols: usize,
    dst_rows: usize,
    dst_cols: usize,
) -> anyhow::Result<Vec<bool>> {
    let slices: Vec<&[f32]> = planes
        .iter()
        .map(|p| p.as_slice().context("overlay plane is not contiguous"))
        .collect::<anyhow::Result<_>>()?;
    let mut out = vec![false; dst_rows * dst_cols];
    out.par_chunks_mut(dst_cols).enumerate().for_each(|(dy, row)| {
        let (y0, y1) = cell_range(dy, rows, dst_rows);
        for (dx, cell) in row.iter_mut().enumerate() {
            let (x0, x1) = cell_range(dx, cols, dst_cols);
            *cell = slices
                .iter()
                .any(|s| (y0..y1).any(|y| s[y * cols + x0..y * cols + x1].iter().any(|&v| is_padding(v))));
        }
    });
    Ok(out)
}

fn reduced_plane<'a>(arr: &'a Array2<f32>, max_dim: usize) -> Cow<'a, Array2<f32>> {
    match reduce_for_preview(arr, max_dim) {
        Some(reduced) => Cow::Owned(reduced),
        None => contiguous(arr),
    }
}

fn stf_bytes(plane: &Array2<f32>, stats: &ImageStats) -> anyhow::Result<Vec<u8>> {
    let stf = make_stf_u8_fn(&auto_stf(stats, &AutoStfConfig::default()), stats);
    let slice = plane.as_slice().context("reduced plane is not contiguous")?;
    Ok(slice.par_iter().map(|&v| stf(v)).collect())
}

fn checker_colour(x: usize, y: usize) -> [u8; 3] {
    if (x / CHECKER_CELL + y / CHECKER_CELL) % 2 == 0 {
        CHECKER_DARK
    } else {
        CHECKER_LIGHT
    }
}

fn compose_rgb(channels: [&[u8]; 3], padding: &[bool], width: usize) -> Vec<u8> {
    let mut out = vec![0u8; padding.len() * 3];
    out.par_chunks_mut(width * 3).enumerate().for_each(|(y, row)| {
        let base = y * width;
        for x in 0..width {
            let i = base + x;
            let o = x * 3;
            if padding[i] {
                row[o..o + 3].copy_from_slice(&checker_colour(x, y));
            } else {
                row[o] = channels[0][i];
                row[o + 1] = channels[1][i];
                row[o + 2] = channels[2][i];
            }
        }
    });
    out
}

fn overlay_channels(bytes: &[Vec<u8>]) -> [&[u8]; 3] {
    match bytes.len() {
        1 => [&bytes[0], &bytes[0], &bytes[0]],
        2 => [&bytes[0], &bytes[1], &bytes[1]],
        _ => [&bytes[0], &bytes[1], &bytes[2]],
    }
}

fn write_rgb_png(pixels: &[u8], width: usize, height: usize, path: &str) -> anyhow::Result<()> {
    let file = std::fs::File::create(path).context("Failed to create output file")?;
    let buf_writer = std::io::BufWriter::with_capacity(2 * 1024 * 1024, file);
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        buf_writer,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    );
    encoder
        .write_image(pixels, width as u32, height as u32, image::ColorType::Rgb8.into())
        .context("Failed to write overlay PNG")?;
    Ok(())
}

fn overlay_file_stem(output_dir: &str) -> String {
    let now = SystemTime::now();
    remove_stale_pngs(output_dir, OVERLAY_PNG_PREFIX, now);
    let ms = now.duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let seq = OVERLAY_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{}/{}_{}_{}", output_dir, OVERLAY_PNG_PREFIX, ms, seq)
}

#[tauri::command]
pub async fn channel_overlay_preview_cmd(
    keys: Vec<String>,
    mask_keys: Option<Vec<String>>,
    with_channel_frames: Option<bool>,
    output_dir: String,
    max_dim: Option<u32>,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let t0 = Instant::now();
        if keys.is_empty() || keys.len() > 3 {
            anyhow::bail!("An overlay takes 1 to 3 channels, got {}", keys.len());
        }
        let out_dir = resolve_output_dir(&output_dir)?;
        let max_dim = max_dim.map(|m| m as usize).unwrap_or(DEFAULT_MAX_DIM).clamp(MIN_MAX_DIM, MAX_PREVIEW_DIM);
        let mask_keys = extra_mask_keys(&keys, mask_keys.unwrap_or_default());

        let overlay_entries: Vec<ImageEntry> =
            keys.iter().map(|k| load_channel_entry(k)).collect::<anyhow::Result<_>>()?;
        let mask_entries: Vec<ImageEntry> =
            mask_keys.iter().map(|k| load_channel_entry(k)).collect::<anyhow::Result<_>>()?;

        let (rows, cols) = overlay_entries[0].arr().dim();
        let overlay_planes: Vec<OverlayPlane> = overlay_entries
            .iter()
            .map(|e| overlay_plane(e, rows, cols))
            .collect::<anyhow::Result<_>>()?;
        let mask_planes: Vec<Cow<Array2<f32>>> = mask_entries
            .iter()
            .map(|e| mask_plane(e, rows, cols))
            .collect::<anyhow::Result<_>>()?;

        let (ph, pw) = preview_dims(rows, cols, max_dim);
        let all_planes: Vec<&Array2<f32>> = overlay_planes
            .iter()
            .map(|p| p.arr.as_ref())
            .chain(mask_planes.iter().map(|p| p.as_ref()))
            .collect();
        let padding = padding_at_preview(&all_planes, rows, cols, ph, pw)?;
        drop(all_planes);
        drop(mask_planes);

        let channel_bytes: Vec<Vec<u8>> = overlay_planes
            .iter()
            .map(|p| {
                let reduced = reduced_plane(p.arr.as_ref(), max_dim);
                if reduced.dim() != (ph, pw) {
                    anyhow::bail!("Preview reduction produced {:?}, expected {:?}", reduced.dim(), (ph, pw));
                }
                stf_bytes(reduced.as_ref(), p.stats.as_ref())
            })
            .collect::<anyhow::Result<_>>()?;

        let stem = overlay_file_stem(&out_dir);
        let png_path = format!("{}_overlay.png", stem);
        let overlay = compose_rgb(overlay_channels(&channel_bytes), &padding, pw);
        write_rgb_png(&overlay, pw, ph, &png_path)?;
        drop(overlay);

        let mut channel_previews = Vec::new();
        if with_channel_frames.unwrap_or(false) {
            for (i, bytes) in channel_bytes.iter().enumerate() {
                let frame_path = format!("{}_{}.png", stem, i);
                let frame = compose_rgb([bytes, bytes, bytes], &padding, pw);
                write_rgb_png(&frame, pw, ph, &frame_path)?;
                channel_previews.push(frame_path);
            }
        }

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_CHANNEL_PREVIEWS: channel_previews,
            RES_DIMENSIONS: [cols, rows],
            RES_PREVIEW_DIMENSIONS: [pw, ph],
            RES_ELAPSED_MS: t0.elapsed().as_millis() as u64,
        }))
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::cmd::compose::align_channels_cmd;
    use crate::cmd::helpers;
    use crate::infra::cache::GLOBAL_IMAGE_CACHE;
    use crate::infra::fits::writer::write_fits_mono;
    use crate::types::constants::{wizard_aligned_key, CHANNELS, RES_REGISTERED};

    fn star_field(dy: f64, dx: f64) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            let (yf, xf) = (y as f64 - dy, x as f64 - dx);
            let blob = |cy: f64, cx: f64| 1000.0 * (-((yf - cy).powi(2) + (xf - cx).powi(2)) / 4.0).exp();
            (10.0 + blob(20.0, 30.0) + blob(40.0, 14.0) + blob(45.0, 45.0) + blob(12.0, 50.0)) as f32
        })
    }

    fn single_star(cy: f64, cx: f64) -> Array2<f32> {
        Array2::from_shape_fn((64, 64), |(y, x)| {
            (10.0 + 1000.0 * (-((y as f64 - cy).powi(2) + (x as f64 - cx).powi(2)) / 4.0).exp()) as f32
        })
    }

    fn seed_wizard_entry(bin: &str, arr: &Array2<f32>) -> String {
        let key = wizard_aligned_key(bin);
        GLOBAL_IMAGE_CACHE.insert_synthetic(&key, Arc::new(arr.clone()), compute_image_stats(arr));
        key
    }

    fn rgb_at(path: &str, x: u32, y: u32) -> [u8; 3] {
        image::open(path).unwrap().to_rgb8().get_pixel(x, y).0
    }

    fn png_size(path: &str) -> (u32, u32) {
        let img = image::open(path).unwrap();
        (img.width(), img.height())
    }

    fn is_checker(px: [u8; 3]) -> bool {
        px == CHECKER_DARK || px == CHECKER_LIGHT
    }

    async fn overlay(keys: Vec<String>, mask_keys: Option<Vec<String>>, frames: bool, out: &str, max_dim: Option<u32>) -> serde_json::Value {
        channel_overlay_preview_cmd(keys, mask_keys, Some(frames), out.to_string(), max_dim).await.unwrap()
    }

    #[tokio::test]
    async fn aligned_channels_overlay_to_grey_while_the_unaligned_pair_shows_colour() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out").to_str().unwrap().to_string();
        let reference_path = dir.path().join("ref.fits").to_str().unwrap().to_string();
        let target_path = dir.path().join("tgt.fits").to_str().unwrap().to_string();
        write_fits_mono(&reference_path, &star_field(0.0, 0.0), None).unwrap();
        write_fits_mono(&target_path, &star_field(2.0, -3.0), None).unwrap();

        let aligned = align_channels_cmd(
            vec![reference_path.clone(), target_path.clone()],
            out.clone(),
            None,
            Some(vec!["ovl_r".to_string(), "ovl_g".to_string()]),
            None,
        )
        .await
        .unwrap();
        assert_eq!(aligned[CHANNELS][1][RES_REGISTERED], true, "{aligned}");
        let keys = vec![wizard_aligned_key("ovl_r"), wizard_aligned_key("ovl_g")];

        let after = overlay(keys.clone(), None, true, &out, None).await;
        let before = overlay(vec![reference_path, target_path], None, false, &out, None).await;
        for k in &keys {
            GLOBAL_IMAGE_CACHE.remove(k);
        }

        assert_eq!(after[RES_DIMENSIONS], json!([64, 64]));
        assert_eq!(after[RES_PREVIEW_DIMENSIONS], json!([64, 64]));
        let after_png = after[RES_PNG_PATH].as_str().unwrap();
        let before_png = before[RES_PNG_PATH].as_str().unwrap();
        assert_eq!(png_size(after_png), (64, 64));

        let flank = rgb_at(after_png, 32, 20);
        assert!(flank[0] > 40, "the star flank must be visible in the reference channel: {flank:?}");
        assert!(flank[0].abs_diff(flank[1]) <= 8, "aligned stars must come out grey: {flank:?}");
        assert_eq!(flank[1], flank[2], "a two-channel overlay paints the second channel in cyan: {flank:?}");

        let unaligned = rgb_at(before_png, 32, 20);
        assert!(unaligned[0].abs_diff(unaligned[1]) > 50, "the unaligned pair must show a colour fringe: {unaligned:?}");
        assert!(is_checker(rgb_at(after_png, 0, 0)), "the shifted-in border of the aligned channel must be padding");
        assert!(!is_checker(rgb_at(before_png, 0, 0)), "the unaligned inputs have no padding");

        let frames = after[RES_CHANNEL_PREVIEWS].as_array().unwrap();
        assert_eq!(frames.len(), 2, "{after}");
        for frame in frames {
            let path = frame.as_str().unwrap();
            assert_eq!(png_size(path), (64, 64));
            let px = rgb_at(path, 32, 20);
            assert!(px[0] == px[1] && px[1] == px[2], "channel frames are grey: {px:?}");
        }
        assert!(before[RES_CHANNEL_PREVIEWS].as_array().unwrap().is_empty());
        assert_ne!(after_png, before_png);
    }

    #[tokio::test]
    async fn the_reference_paints_red_and_the_second_channel_cyan_then_the_third_blue() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let keys = vec![
            seed_wizard_entry("ovl_col_a", &single_star(20.0, 30.0)),
            seed_wizard_entry("ovl_col_b", &single_star(40.0, 14.0)),
            seed_wizard_entry("ovl_col_c", &single_star(45.0, 45.0)),
        ];

        let pair = overlay(keys[..2].to_vec(), None, false, &out, None).await;
        let triple = overlay(keys.clone(), None, false, &out, None).await;
        for k in &keys {
            GLOBAL_IMAGE_CACHE.remove(k);
        }

        let pair_png = pair[RES_PNG_PATH].as_str().unwrap();
        let star_a = rgb_at(pair_png, 30, 20);
        assert!(star_a[0] > 200 && star_a[1] < 16, "keys[0] must drive the red channel: {star_a:?}");
        assert_eq!(star_a[1], star_a[2], "G and B both come from keys[1]: {star_a:?}");
        let star_b = rgb_at(pair_png, 14, 40);
        assert!(star_b[1] > 200 && star_b[0] < 16, "keys[1] must drive green and blue: {star_b:?}");
        assert_eq!(star_b[1], star_b[2], "{star_b:?}");

        let triple_png = triple[RES_PNG_PATH].as_str().unwrap();
        let (a, b, c) = (rgb_at(triple_png, 30, 20), rgb_at(triple_png, 14, 40), rgb_at(triple_png, 45, 45));
        assert!(a[0] > 200 && a[1] < 16 && a[2] < 16, "keys[0] -> R: {a:?}");
        assert!(b[1] > 200 && b[0] < 16 && b[2] < 16, "keys[1] -> G: {b:?}");
        assert!(c[2] > 200 && c[0] < 16 && c[1] < 16, "keys[2] -> B: {c:?}");
    }

    #[tokio::test]
    async fn a_channel_on_another_grid_is_resampled_onto_the_reference_grid() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let doubled = Array2::from_shape_fn((128, 128), |(y, x)| {
            (10.0 + 1000.0 * (-((y as f64 - 40.5).powi(2) + (x as f64 - 60.5).powi(2)) / 16.0).exp()) as f32
        });
        let reference = seed_wizard_entry("ovl_grid_ref", &single_star(20.0, 30.0));
        let target = seed_wizard_entry("ovl_grid_big", &doubled);

        let res = overlay(vec![reference.clone(), target.clone()], None, true, &out, None).await;
        GLOBAL_IMAGE_CACHE.remove(&reference);
        GLOBAL_IMAGE_CACHE.remove(&target);

        assert_eq!(res[RES_DIMENSIONS], json!([64, 64]), "dimensions follow keys[0]: {res}");
        assert_eq!(res[RES_PREVIEW_DIMENSIONS], json!([64, 64]));
        let png = res[RES_PNG_PATH].as_str().unwrap();
        assert_eq!(png_size(png), (64, 64));
        let frame = res[RES_CHANNEL_PREVIEWS][1].as_str().unwrap();
        assert_eq!(png_size(frame), (64, 64), "the resampled channel frame is on the reference grid");

        let peak = rgb_at(png, 30, 20);
        assert!(peak[0] > 200 && peak[1] > 200, "the star must line up in R and G: {peak:?}");
        let flank = rgb_at(png, 31, 21);
        assert!(flank[0] > 40 && flank[0] < 230, "{flank:?}");
        assert!(flank[0].abs_diff(flank[1]) <= 8, "a resampled star must overlay grey on its flank: {flank:?}");
        let elsewhere = rgb_at(png, 60, 40);
        assert!(elsewhere[0] < 16 && elsewhere[1] < 16, "the 128-grid star position must not leak through unscaled: {elsewhere:?}");
    }

    #[tokio::test]
    async fn downsampling_keeps_a_single_bright_pixel_visible() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let mut plane = Array2::from_elem((1024, 1024), 100.0f32);
        plane[[601, 703]] = 5000.0;
        let key = seed_wizard_entry("ovl_star_cell", &plane);

        let res = overlay(vec![key.clone()], None, false, &out, Some(256)).await;
        GLOBAL_IMAGE_CACHE.remove(&key);

        assert_eq!(res[RES_PREVIEW_DIMENSIONS], json!([256, 256]));
        let png = res[RES_PNG_PATH].as_str().unwrap();
        let (cx, cy) = (703 / 4, 601 / 4);
        let star = rgb_at(png, cx, cy);
        assert!(!is_checker(star));
        assert!(star[0] > 200, "the cell holding the bright pixel must stay bright after reduction: {star:?}");
        for (x, y) in [(cx - 1, cy), (cx + 1, cy), (cx, cy - 1), (cx, cy + 1)] {
            let n = rgb_at(png, x, y);
            assert!(n[0] < 10, "neighbour cells hold only the flat background: {n:?}");
        }
    }

    #[tokio::test]
    async fn each_channel_gets_its_own_stf_so_a_rescaled_copy_still_overlays_grey() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let base = single_star(20.0, 30.0);
        let rescaled = base.mapv(|v| v * 20.0 + 500.0);
        let a = seed_wizard_entry("ovl_stf_a", &base);
        let b = seed_wizard_entry("ovl_stf_b", &rescaled);

        let res = overlay(vec![a.clone(), b.clone()], None, false, &out, None).await;
        GLOBAL_IMAGE_CACHE.remove(&a);
        GLOBAL_IMAGE_CACHE.remove(&b);

        let png = res[RES_PNG_PATH].as_str().unwrap();
        let peak = rgb_at(png, 30, 20);
        assert!(peak[0] > 200 && peak[1] > 200, "{peak:?}");
        let flank = rgb_at(png, 31, 21);
        assert!(flank[0] > 40 && flank[0] < 230, "the probe must sit on a star flank: {flank:?}");
        assert!(flank[0].abs_diff(flank[1]) <= 2, "unlinked per-channel STF must cancel a linear rescale: {flank:?}");
        assert_eq!(flank[1], flank[2]);
    }

    #[tokio::test]
    async fn an_overlay_needs_one_to_three_channels() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let none = channel_overlay_preview_cmd(vec![], None, None, out.clone(), None).await;
        assert!(none.expect_err("zero keys must fail").contains("1 to 3"));
        let four: Vec<String> = (0..4).map(|i| format!("__wizard_ch_ovl_four_{i}_aligned")).collect();
        let too_many = channel_overlay_preview_cmd(four, None, None, out, None).await;
        assert!(too_many.expect_err("four keys must fail").contains("1 to 3"));
    }

    #[tokio::test]
    async fn a_missing_wizard_key_says_to_run_align_again() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let gone = channel_overlay_preview_cmd(vec![wizard_aligned_key("ovl_missing_probe")], None, None, out, None).await;
        assert_eq!(gone.expect_err("a missing wizard key must fail"), ALIGNED_GONE);
    }

    #[test]
    fn mask_keys_that_repeat_an_overlay_key_or_each_other_are_dropped() {
        let keys = vec!["a".to_string(), "b".to_string()];
        let extra = extra_mask_keys(&keys, vec!["b".into(), "c".into(), "c".into(), "a".into(), "d".into()]);
        assert_eq!(extra, vec!["c".to_string(), "d".to_string()]);
        assert!(extra_mask_keys(&keys, vec![]).is_empty());
    }

    #[test]
    fn preview_padding_marks_a_cell_when_any_plane_has_padding_inside_it() {
        let mut first = Array2::from_elem((4, 4), 1.0f32);
        first[[0, 0]] = f32::NAN;
        let mut second = Array2::from_elem((4, 4), 1.0f32);
        second[[3, 3]] = 0.0;
        let padding = padding_at_preview(&[&first, &second], 4, 4, 2, 2).unwrap();
        assert_eq!(padding, vec![true, false, false, true]);
        let full = padding_at_preview(&[&first, &second], 4, 4, 4, 4).unwrap();
        assert_eq!(full.iter().filter(|&&p| p).count(), 2);
        assert!(full[0] && full[15]);
    }

    #[tokio::test]
    async fn padding_in_any_key_becomes_a_checkerboard_on_the_output_pixels() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let mut gradient = Array2::from_shape_fn((32, 32), |(y, x)| 100.0 + (y * 32 + x) as f32);
        gradient[[10, 10]] = f32::NAN;
        let mut mask = Array2::from_elem((32, 32), 1.0f32);
        mask[[3, 5]] = f32::NAN;
        mask[[20, 9]] = 0.0;
        let key = seed_wizard_entry("ovl_pad", &gradient);
        let mask_key = seed_wizard_entry("ovl_pad_mask", &mask);

        let res = overlay(vec![key.clone()], Some(vec![key.clone(), mask_key.clone()]), true, &out, None).await;
        GLOBAL_IMAGE_CACHE.remove(&key);
        GLOBAL_IMAGE_CACHE.remove(&mask_key);

        let png = res[RES_PNG_PATH].as_str().unwrap();
        assert_eq!(rgb_at(png, 5, 3), CHECKER_DARK, "NaN in a mask key");
        assert_eq!(rgb_at(png, 9, 20), CHECKER_LIGHT, "exact zero in a mask key");
        assert_eq!(rgb_at(png, 10, 10), CHECKER_DARK, "NaN in the overlay key");
        let data = rgb_at(png, 16, 16);
        assert!(!is_checker(data) && data[0] == data[1] && data[1] == data[2] && data[0] > 0, "{data:?}");
        let frame = res[RES_CHANNEL_PREVIEWS][0].as_str().unwrap();
        assert_eq!(rgb_at(frame, 5, 3), CHECKER_DARK, "frames carry the same padding marking");
        assert_eq!(rgb_at(frame, 16, 16), data);
    }

    #[tokio::test]
    async fn a_downsampled_cell_is_padding_when_any_source_pixel_in_it_is_padding() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let mut plane = Array2::from_shape_fn((520, 520), |(y, x)| 50.0 + ((y * 7 + x * 13) % 101) as f32);
        plane[[300, 300]] = f32::NAN;
        let key = seed_wizard_entry("ovl_reduce", &plane);

        let res = overlay(vec![key.clone()], None, false, &out, Some(256)).await;
        GLOBAL_IMAGE_CACHE.remove(&key);

        assert_eq!(res[RES_DIMENSIONS], json!([520, 520]));
        assert_eq!(res[RES_PREVIEW_DIMENSIONS], json!([256, 256]));
        let png = res[RES_PNG_PATH].as_str().unwrap();
        assert_eq!(png_size(png), (256, 256));
        let cell = (0..256).find(|&d| {
            let (s, e) = cell_range(d, 520, 256);
            (s..e).contains(&300)
        })
        .unwrap();
        let hit = rgb_at(png, cell as u32, cell as u32);
        assert!(is_checker(hit), "a cell with one padding source pixel must be padding: {hit:?}");
        let neighbour = rgb_at(png, cell as u32 + 2, cell as u32 + 2);
        assert!(!is_checker(neighbour), "{neighbour:?}");
    }

    #[tokio::test]
    async fn an_overlay_leaves_the_composite_slots_and_generation_untouched() {
        let _composite = helpers::composite_test_lock().await;
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let sentinel = |seed: f32| Array2::from_shape_fn((8, 8), |(y, x)| seed + (y * 8 + x) as f32);
        let (r, g, b) = (sentinel(1.0), sentinel(2.0), sentinel(3.0));
        helpers::insert_composite_and_orig(
            r.clone(),
            g.clone(),
            b.clone(),
            compute_image_stats(&r),
            compute_image_stats(&g),
            compute_image_stats(&b),
        );
        let (before_r, before_g, before_b) = helpers::load_composite_rgb().unwrap();
        let generation = helpers::composite_generation();
        let keys = vec![
            seed_wizard_entry("ovl_slot_a", &star_field(0.0, 0.0)),
            seed_wizard_entry("ovl_slot_b", &star_field(1.0, 1.0)),
            seed_wizard_entry("ovl_slot_c", &star_field(-1.0, 2.0)),
        ];

        let res = overlay(keys.clone(), None, true, &out, None).await;
        for k in &keys {
            GLOBAL_IMAGE_CACHE.remove(k);
        }
        let (after_r, after_g, after_b) = helpers::load_composite_rgb().unwrap();
        let generation_after = helpers::composite_generation();
        helpers::clear_composite();

        assert!(Arc::ptr_eq(&before_r.data_arc(), &after_r.data_arc()));
        assert!(Arc::ptr_eq(&before_g.data_arc(), &after_g.data_arc()));
        assert!(Arc::ptr_eq(&before_b.data_arc(), &after_b.data_arc()));
        assert_eq!(generation, generation_after, "an overlay preview bumped the composite generation");
        assert_eq!(res[RES_CHANNEL_PREVIEWS].as_array().unwrap().len(), 3);
        assert!(!res[RES_PNG_PATH].as_str().unwrap().contains("rgb_composite"));
        let px = rgb_at(res[RES_PNG_PATH].as_str().unwrap(), 30, 20);
        assert!(px[0] > 0 && px[1] > 0 && px[2] > 0, "three channels map to R, G and B: {px:?}");
    }

    #[tokio::test]
    async fn consecutive_overlays_get_distinct_names_and_keep_fresh_files() {
        let _wizard = crate::infra::cache::lock_wizard_entries();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let key = seed_wizard_entry("ovl_names", &star_field(0.0, 0.0));

        let first = overlay(vec![key.clone()], None, false, &out, None).await;
        let second = overlay(vec![key.clone()], None, false, &out, None).await;
        GLOBAL_IMAGE_CACHE.remove(&key);

        let first_png = first[RES_PNG_PATH].as_str().unwrap();
        let second_png = second[RES_PNG_PATH].as_str().unwrap();
        assert_ne!(first_png, second_png);
        assert!(std::path::Path::new(first_png).exists(), "a fresh overlay was deleted by the next call");
        assert!(std::path::Path::new(second_png).exists());
        let name = std::path::Path::new(second_png).file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("channel_overlay_") && name.ends_with("_overlay.png"), "{name}");
    }

    #[test]
    fn back_to_back_stems_in_the_same_dir_carry_increasing_sequence_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap();
        let first = overlay_file_stem(out);
        let second = overlay_file_stem(out);
        let seq = |stem: &str| stem.rsplit('_').next().unwrap().parse::<u64>().unwrap();
        assert_ne!(first, second);
        assert!(seq(&second) > seq(&first), "{first} vs {second}");
        assert!(first.starts_with(&format!("{out}/channel_overlay_")), "{first}");
    }

    #[test]
    fn stale_overlay_pngs_are_removed_and_other_files_kept() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("channel_overlay_0_0_overlay.png");
        let fresh = dir.path().join("channel_overlay_1_1_overlay.png");
        let other = dir.path().join("rgb_composite_0.png");
        for p in [&stale, &fresh, &other] {
            std::fs::write(p, b"png").unwrap();
        }
        let old = SystemTime::now() - crate::cmd::compose::rgb::STALE_PNG_GRACE - std::time::Duration::from_secs(60);
        for p in [&stale, &other] {
            std::fs::OpenOptions::new().write(true).open(p).unwrap().set_modified(old).unwrap();
        }

        let stem = overlay_file_stem(dir.path().to_str().unwrap());

        assert!(!stale.exists(), "old overlay PNGs are not cleaned up");
        assert!(fresh.exists());
        assert!(other.exists(), "the overlay cleanup must only touch its own prefix");
        assert!(stem.contains("channel_overlay_"));
    }
}
