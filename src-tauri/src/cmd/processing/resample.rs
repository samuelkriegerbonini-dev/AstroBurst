use serde_json::json;

use crate::cmd::common::{
    blocking_cmd, derived_output_header, extract_image_resolved, output_stem, render_named_and_save,
    resolve_output_dir, write_derived_fits, OutputValues,
};
use crate::cmd::compose::rescale_per_pixel_calibration;
use crate::core::imaging::resample::resample_with_wcs;
use crate::core::imaging::stats::compute_image_stats;
use crate::types::constants::{
    RES_DIMENSIONS, RES_FITS_PATH, RES_MAX, RES_MEAN, RES_MIN,
    RES_ORIGINAL_DIMENSIONS, RES_PNG_PATH, RES_SIGMA, RES_STATS, RES_WCS_UPDATES,
};
use crate::types::header::HduHeader;

const ABPROC_RESAMPLED: &str = "resampled";

fn resampled_name(path: &str, width: usize, height: usize) -> String {
    format!("{}_resampled_{}x{}", output_stem(path), width, height)
}

fn pixel_area_ratio(original_dims: [usize; 2], resampled_dims: [usize; 2]) -> f64 {
    let scale_x = original_dims[0] as f64 / resampled_dims[0] as f64;
    let scale_y = original_dims[1] as f64 / resampled_dims[1] as f64;
    scale_x * scale_y
}

fn resampled_header(source: &HduHeader, wcs_updates: &[(String, f64)], area_ratio: f64) -> HduHeader {
    let mut header = derived_output_header(Some(source), ABPROC_RESAMPLED, OutputValues::Linear);
    for (key, value) in wcs_updates {
        header.set_f64(key, *value);
    }
    rescale_per_pixel_calibration(&mut header, area_ratio);
    header
}

#[tauri::command]
pub async fn resample_fits_cmd(
    path: String,
    target_width: usize,
    target_height: usize,
    output_dir: String,
) -> Result<serde_json::Value, String> {
    blocking_cmd!({
        let output_dir = resolve_output_dir(&output_dir)?;
        let resolved = extract_image_resolved(&path)?;
        let result = resample_with_wcs(
            &resolved.arr,
            &resolved.header,
            target_height,
            target_width,
        )?;

        let area_ratio = pixel_area_ratio(result.original_dims, result.resampled_dims);
        let out_header = resampled_header(&resolved.header, &result.header_updates, area_ratio);

        let name = resampled_name(&path, target_width, target_height);
        let (png_path, _) = render_named_and_save(&result.image, &output_dir, &name, false, None)?;
        let fits_path = format!("{}/{}.fits", output_dir, name);
        write_derived_fits(&fits_path, &result.image, Some(&out_header))?;

        let stats = compute_image_stats(&result.image);

        Ok(json!({
            RES_PNG_PATH: png_path,
            RES_FITS_PATH: fits_path,
            RES_DIMENSIONS: result.resampled_dims,
            RES_ORIGINAL_DIMENSIONS: result.original_dims,
            RES_WCS_UPDATES: result.header_updates,
            RES_STATS: {
                RES_MIN: stats.min,
                RES_MAX: stats.max,
                RES_MEAN: stats.mean,
                RES_SIGMA: stats.sigma,
            },
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::common::{cached_header, load_cached, HEADER_ABPROC};
    use crate::infra::fits::writer::write_fits_mono;
    use ndarray::Array2;

    fn header_with(cards: &[(&str, &str)]) -> HduHeader {
        let mut header = HduHeader::empty();
        for (k, v) in cards {
            header.set(k, v.to_string());
        }
        header
    }

    fn field(dir: &tempfile::TempDir, name: &str, header: &HduHeader) -> String {
        let path = dir.path().join(name).to_str().unwrap().to_string();
        let data = Array2::from_shape_fn((8, 8), |(y, x)| 10.0 + (y * 8 + x) as f32);
        write_fits_mono(&path, &data, Some(header)).unwrap();
        path
    }

    fn close(actual: Option<f64>, expected: f64) -> bool {
        actual.is_some_and(|v| (v - expected).abs() <= expected.abs() * 1e-9)
    }

    #[tokio::test]
    async fn resampling_one_source_to_two_sizes_keeps_both_results() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let src = field(&dir, "light_001.fits", &HduHeader::empty());

        let half = resample_fits_cmd(src.clone(), 4, 4, out.clone()).await.unwrap();
        let quarter = resample_fits_cmd(src, 2, 2, out).await.unwrap();
        let half_path = half[RES_FITS_PATH].as_str().unwrap().to_string();
        let quarter_path = quarter[RES_FITS_PATH].as_str().unwrap().to_string();
        assert_ne!(half_path, quarter_path, "the second resample overwrote the first");
        assert_ne!(half[RES_PNG_PATH], quarter[RES_PNG_PATH]);
        assert_eq!(load_cached(&half_path).unwrap().arr().dim(), (4, 4));
        assert_eq!(load_cached(&quarter_path).unwrap().arr().dim(), (2, 2));
    }

    #[tokio::test]
    async fn resampling_rescales_the_per_pixel_calibration_and_drops_saturation() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let source = header_with(&[
            ("XTENSION", "'IMAGE   '"),
            ("CHECKSUM", "'abc'"),
            ("CTYPE1", "'RA---TAN'"),
            ("CTYPE2", "'DEC--TAN'"),
            ("CRPIX1", "4.5"),
            ("CRPIX2", "4.5"),
            ("CD1_1", "-1.0E-5"),
            ("CD2_2", "1.0E-5"),
            ("BUNIT", "'MJy/sr'"),
            ("PHOTMJSR", "1.5"),
            ("PIXAR_SR", "2.0E-14"),
            ("PIXAR_A2", "0.001"),
            ("PHOTFLAM", "3.0E-19"),
            ("MAGZERO", "25.0"),
            ("ZP", "26.0"),
            ("ZPTMAG", "24.0"),
            ("SATURATE", "60000.0"),
        ]);
        let src = field(&dir, "jw_i2d.fits", &source);

        let value = resample_fits_cmd(src, 4, 4, out).await.unwrap();
        let header = cached_header(value[RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert!(close(header.get_f64("PIXAR_SR"), 8.0e-14), "PIXAR_SR {:?}", header.get("PIXAR_SR"));
        assert!(close(header.get_f64("PIXAR_A2"), 0.004));
        assert!(close(header.get_f64("PHOTFLAM"), 1.2e-18));
        assert!(close(header.get_f64("MAGZERO"), 25.0 - 2.5 * 4f64.log10()));
        assert!(close(header.get_f64("ZP"), 26.0 - 2.5 * 4f64.log10()), "ZP {:?}", header.get("ZP"));
        assert!(close(header.get_f64("ZPTMAG"), 24.0 - 2.5 * 4f64.log10()), "ZPTMAG {:?}", header.get("ZPTMAG"));
        assert!(close(header.get_f64("PHOTMJSR"), 1.5), "a surface-brightness conversion does not depend on the pixel size");
        assert!(close(header.get_f64("CD1_1"), -2.0e-5));
        assert_eq!(header.get("BUNIT").map(|v| v.trim().trim_matches('\'').trim()), Some("MJy/sr"));
        assert!(header.get("SATURATE").is_none(), "an averaged pixel no longer saturates at the source level");
        assert!(header.get("CHECKSUM").is_none());
        assert_eq!(header.get(HEADER_ABPROC).map(|v| v.trim().trim_matches('\'').trim()), Some(ABPROC_RESAMPLED));
    }

    #[tokio::test]
    async fn a_resampled_output_keeps_its_sip_distortion_on_the_new_grid() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().to_str().unwrap().to_string();
        let source = header_with(&[
            ("CTYPE1", "'RA---TAN-SIP'"),
            ("CTYPE2", "'DEC--TAN-SIP'"),
            ("CRPIX1", "4.5"),
            ("CRPIX2", "4.5"),
            ("CD1_1", "-1.0E-5"),
            ("CD2_2", "1.0E-5"),
            ("A_ORDER", "2"),
            ("B_ORDER", "2"),
            ("A_2_0", "1.0E-5"),
            ("B_0_2", "-6.0E-6"),
            ("A_DMAX", "3.0"),
        ]);
        let src = field(&dir, "hst_flc.fits", &source);

        let value = resample_fits_cmd(src, 4, 4, out).await.unwrap();
        let header = cached_header(value[RES_FITS_PATH].as_str().unwrap()).unwrap();
        assert_eq!(header.get("CTYPE1").map(|v| v.trim().trim_matches('\'').trim()), Some("RA---TAN-SIP"));
        assert_eq!(header.get_f64("A_ORDER"), Some(2.0), "the SIP polynomial was stripped");
        assert!(close(header.get_f64("A_2_0"), 2.0e-5), "A_2_0 {:?}", header.get("A_2_0"));
        assert!(close(header.get_f64("B_0_2"), -1.2e-5), "B_0_2 {:?}", header.get("B_0_2"));
        assert!(close(header.get_f64("A_DMAX"), 1.5), "A_DMAX {:?}", header.get("A_DMAX"));
    }
}
