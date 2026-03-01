use std::collections::{HashMap, VecDeque};
use crate::domain::stats;

use anyhow::{bail, Context, Result};
use ndarray::Array2;
use serde::{Deserialize, Serialize};



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedStar {
    /// Sub-pixel centroid X (0-based, in image columns)
    pub x: f64,
    /// Sub-pixel centroid Y (0-based, in image rows)
    pub y: f64,
    /// Instrumental flux (sum of background-subtracted pixels)
    pub flux: f64,
    /// Estimated FWHM in pixels
    pub fwhm: f64,
    /// Peak value above background
    pub peak: f64,
    /// Number of pixels in the source
    pub npix: usize,
    /// Signal-to-noise ratio (peak / σ_background)
    pub snr: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionResult {
    pub stars: Vec<DetectedStar>,
    pub background_median: f64,
    pub background_sigma: f64,
    pub threshold_sigma: f64,
    pub image_width: usize,
    pub image_height: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveResult {
    pub success: bool,
    pub ra_center: f64,
    pub dec_center: f64,
    pub orientation: f64,
    pub pixel_scale: f64,
    pub field_w_arcmin: f64,
    pub field_h_arcmin: f64,
    pub index_name: String,
    pub stars_used: usize,
    pub wcs_headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolveConfig {
    /// Astrometry.net API URL (default: nova.astrometry.net)
    pub api_url: String,
    /// API key (required for nova.astrometry.net)
    pub api_key: String,
    /// Optional RA hint (degrees) to speed up solve
    pub ra_hint: Option<f64>,
    /// Optional Dec hint (degrees) to speed up solve
    pub dec_hint: Option<f64>,
    /// Search radius around hint (degrees, default 10)
    pub radius_hint: Option<f64>,
    /// Approximate pixel scale range (arcsec/px)
    pub scale_low: Option<f64>,
    pub scale_high: Option<f64>,
    /// Max number of stars to send (default: 100)
    pub max_stars: Option<usize>,
}

impl Default for SolveConfig {
    fn default() -> Self {
        Self {
            api_url: "https://nova.astrometry.net".into(),
            api_key: String::new(),
            ra_hint: None,
            dec_hint: None,
            radius_hint: Some(10.0),
            scale_low: None,
            scale_high: None,
            max_stars: Some(100),
        }
    }
}

// ---------------------------------------------------------------------------
// Star detection
// ---------------------------------------------------------------------------

/// Estimate background level and noise via tiled sigma-clipped median.
fn estimate_background(image: &Array2<f32>, tile_size: usize) -> (f64, f64) {
    let (rows, cols) = image.dim();
    let mut medians = Vec::new();
    let mut sigmas = Vec::new();

    let step = tile_size.max(16);
    let mut y = 0;
    while y < rows {
        let mut x = 0;
        while x < cols {
            let ye = (y + step).min(rows);
            let xe = (x + step).min(cols);

            let mut vals: Vec<f32> = Vec::with_capacity((ye - y) * (xe - x));
            for r in y..ye {
                for c in x..xe {
                    let v = image[[r, c]];
                    if v.is_finite() {
                        vals.push(v);
                    }
                }
            }

            if vals.len() >= 8 {
                // Sigma-clipped statistics (2 iterations, 3σ)
                let (med, sig) = stats::sigma_clipped_stats(&mut vals, 3.0, 2);
                medians.push(med);
                sigmas.push(sig);
            }
            x += step;
        }
        y += step;
    }

    if medians.is_empty() {
        return (0.0, 1.0);
    }

    medians.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sigmas.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let global_median = medians[medians.len() / 2];
    let global_sigma = sigmas[sigmas.len() / 2];

    (global_median, global_sigma.max(1e-10))
}

/// Detect stars by threshold + connected components + centroiding.
pub fn detect_stars(image: &Array2<f32>, sigma_threshold: f64) -> DetectionResult {
    let (rows, cols) = image.dim();
    let tile_size = (rows.min(cols) / 8).max(32).min(256);
    let (bg_median, bg_sigma) = estimate_background(image, tile_size);

    let threshold = bg_median + sigma_threshold * bg_sigma;

    let mut visited = Array2::<bool>::default((rows, cols));
    let mut stars = Vec::new();

    for r in 1..rows - 1 {
        for c in 1..cols - 1 {
            let v = image[[r, c]] as f64;
            if v <= threshold || visited[[r, c]] || !v.is_finite() {
                continue;
            }

            // BFS flood-fill
            let mut queue = VecDeque::new();
            let mut component: Vec<(usize, usize)> = Vec::new();
            queue.push_back((r, c));
            visited[[r, c]] = true;

            while let Some((cr, cc)) = queue.pop_front() {
                component.push((cr, cc));

                for (dr, dc) in &[(-1i32, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (-1, 1), (1, -1), (1, 1)] {
                    let nr = cr as i32 + dr;
                    let nc = cc as i32 + dc;
                    if nr < 0 || nc < 0 || nr >= rows as i32 || nc >= cols as i32 {
                        continue;
                    }
                    let nr = nr as usize;
                    let nc = nc as usize;
                    if visited[[nr, nc]] {
                        continue;
                    }
                    let nv = image[[nr, nc]] as f64;
                    if nv > threshold && nv.is_finite() {
                        visited[[nr, nc]] = true;
                        queue.push_back((nr, nc));
                    }
                }
            }

            let npix = component.len();
            if npix < 3 || npix > 5000 {
                continue;
            }
            let mut sum_flux = 0.0f64;
            let mut sum_x = 0.0f64;
            let mut sum_y = 0.0f64;
            let mut peak_val = 0.0f64;

            for &(pr, pc) in &component {
                let v = (image[[pr, pc]] as f64 - bg_median).max(0.0);
                sum_flux += v;
                sum_x += pc as f64 * v;
                sum_y += pr as f64 * v;
                if v > peak_val {
                    peak_val = v;
                }
            }

            if sum_flux <= 0.0 {
                continue;
            }

            let cx = sum_x / sum_flux;
            let cy = sum_y / sum_flux;

            let mut sum_r2 = 0.0f64;
            for &(pr, pc) in &component {
                let v = (image[[pr, pc]] as f64 - bg_median).max(0.0);
                let dx = pc as f64 - cx;
                let dy = pr as f64 - cy;
                sum_r2 += (dx * dx + dy * dy) * v;
            }
            let sigma_star = (sum_r2 / sum_flux).sqrt();
            let fwhm = sigma_star * 2.355; // σ → FWHM

            if fwhm < 0.5 || fwhm > 30.0 {
                continue;
            }

            let snr = peak_val / bg_sigma;

            stars.push(DetectedStar {
                x: cx,
                y: cy,
                flux: sum_flux,
                fwhm,
                peak: peak_val,
                npix,
                snr,
            });
        }
    }

    stars.sort_by(|a, b| b.flux.partial_cmp(&a.flux).unwrap_or(std::cmp::Ordering::Equal));

    let mut deduped = Vec::with_capacity(stars.len());
    let mut used = vec![false; stars.len()];
    for i in 0..stars.len() {
        if used[i] {
            continue;
        }
        deduped.push(stars[i].clone());
        for j in i + 1..stars.len() {
            if used[j] {
                continue;
            }
            let dx = stars[i].x - stars[j].x;
            let dy = stars[i].y - stars[j].y;
            if dx * dx + dy * dy < 9.0 {
                used[j] = true;
            }
        }
    }

    DetectionResult {
        stars: deduped,
        background_median: bg_median,
        background_sigma: bg_sigma,
        threshold_sigma: sigma_threshold,
        image_width: cols,
        image_height: rows,
    }
}


/// Submit detected stars to astrometry.net and wait for a solution.
///
/// This is an async HTTP workflow:
/// 1. POST /api/login → session key
/// 2. POST /api/upload with XYLS table → submission ID
/// 3. Poll /api/submissions/{id} until job is ready
/// 4. GET /api/jobs/{id}/info → WCS solution
///
/// Returns `SolveResult` on success.
#[cfg(feature = "astrometry-net")]
pub async fn solve_astrometry_net(
    fits_path: &str,
    stars: &[DetectedStar],
    image_width: usize,
    image_height: usize,
    config: &SolveConfig,
) -> Result<SolveResult> {
    use reqwest::Client;
    use reqwest::multipart;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let base_url = &config.api_url;

    let login_body = serde_json::json!({ "apikey": config.api_key });
    let login_resp: serde_json::Value = client
        .post(format!("{}/api/login", base_url))
        .form(&[("request-json", serde_json::to_string(&login_body)?)])
        .send()
        .await?
        .json()
        .await?;

    let status = login_resp["status"].as_str().unwrap_or("");
    if status != "success" {
        bail!(
            "Astrometry.net login failed: {}",
            login_resp["errormessage"].as_str().unwrap_or("unknown error")
        );
    }

    let session = login_resp["session"]
        .as_str()
        .context("No session in login response")?
        .to_string();

    let mut upload_json = serde_json::json!({
        "session": session,
        "allow_commercial_use": "n",
        "allow_modifications": "n",
        "publicly_visible": "n",
    });

    if let (Some(ra), Some(dec)) = (config.ra_hint, config.dec_hint) {
        upload_json["center_ra"] = serde_json::json!(ra);
        upload_json["center_dec"] = serde_json::json!(dec);
        upload_json["radius"] = serde_json::json!(config.radius_hint.unwrap_or(10.0));
    }
    if let (Some(lo), Some(hi)) = (config.scale_low, config.scale_high) {
        upload_json["scale_lower"] = serde_json::json!(lo);
        upload_json["scale_upper"] = serde_json::json!(hi);
        upload_json["scale_type"] = serde_json::json!("arcsecperpix");
        upload_json["scale_units"] = serde_json::json!("arcsecperpix");
    }

    let file_bytes = std::fs::read(fits_path)
        .with_context(|| format!("Failed to read FITS file: {}", fits_path))?;

    let file_name = std::path::Path::new(fits_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.fits".into());

    let file_part = multipart::Part::bytes(file_bytes)
        .file_name(file_name)
        .mime_str("application/fits")?;

    let form = multipart::Form::new()
        .text("request-json", serde_json::to_string(&upload_json)?)
        .part("file", file_part);

    let upload_resp: serde_json::Value = client
        .post(format!("{}/api/upload", base_url))
        .multipart(form)
        .send()
        .await?
        .json()
        .await?;

    let upload_status = upload_resp["status"].as_str().unwrap_or("");
    if upload_status != "success" {
        bail!(
            "Astrometry.net upload failed: {}",
            upload_resp["errormessage"].as_str().unwrap_or("unknown error")
        );
    }

    let subid = upload_resp["subid"]
        .as_u64()
        .context("No subid in upload response")?;

    let mut job_id: Option<u64> = None;
    for _ in 0..90 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let status: serde_json::Value = client
            .get(format!("{}/api/submissions/{}", base_url, subid))
            .send()
            .await?
            .json()
            .await?;

        if let Some(jobs) = status["jobs"].as_array() {
            for j in jobs {
                if let Some(id) = j.as_u64() {
                    if id > 0 {
                        job_id = Some(id);
                        break;
                    }
                }
            }
        }
        if job_id.is_some() {
            break;
        }
    }

    let jid = job_id.context("Timed out waiting for astrometry.net job")?;

    let mut solved = false;
    for _ in 0..90 {
        let job_status: serde_json::Value = client
            .get(format!("{}/api/jobs/{}", base_url, jid))
            .send()
            .await?
            .json()
            .await?;

        let status_str = job_status["status"].as_str().unwrap_or("");
        if status_str == "success" {
            solved = true;
            break;
        }
        if status_str == "failure" {
            bail!("Plate solve failed on astrometry.net");
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    if !solved {
        bail!("Plate solve timed out after 180s");
    }

    let cal: serde_json::Value = client
        .get(format!("{}/api/jobs/{}/calibration", base_url, jid))
        .send()
        .await?
        .json()
        .await?;

    let ra_center = cal["ra"].as_f64().unwrap_or(0.0);
    let dec_center = cal["dec"].as_f64().unwrap_or(0.0);
    let orientation = cal["orientation"].as_f64().unwrap_or(0.0);
    let pixel_scale = cal["pixscale"].as_f64().unwrap_or(0.0);
    let field_w = cal["radius"].as_f64().unwrap_or(0.0) * 2.0 * 60.0;

    let mut wcs_headers = HashMap::new();
    wcs_headers.insert("CRVAL1".into(), format!("{:.8}", ra_center));
    wcs_headers.insert("CRVAL2".into(), format!("{:.8}", dec_center));
    wcs_headers.insert("CRPIX1".into(), format!("{:.1}", image_width as f64 / 2.0));
    wcs_headers.insert("CRPIX2".into(), format!("{:.1}", image_height as f64 / 2.0));

    let theta = orientation.to_radians();
    let scale_deg = pixel_scale / 3600.0;
    wcs_headers.insert("CD1_1".into(), format!("{:.12E}", -scale_deg * theta.cos()));
    wcs_headers.insert("CD1_2".into(), format!("{:.12E}", scale_deg * theta.sin()));
    wcs_headers.insert("CD2_1".into(), format!("{:.12E}", scale_deg * theta.sin()));
    wcs_headers.insert("CD2_2".into(), format!("{:.12E}", scale_deg * theta.cos()));
    wcs_headers.insert("CTYPE1".into(), "RA---TAN".into());
    wcs_headers.insert("CTYPE2".into(), "DEC--TAN".into());

    let wcs_info: serde_json::Value = client
        .get(format!("{}/api/jobs/{}/info", base_url, jid))
        .send()
        .await?
        .json()
        .await?;

    if let Some(tags) = wcs_info["tags"].as_array() {
        for tag in tags {
            if let Some(t) = tag.as_str() {
                if t.starts_with("index-") || t.contains("field") {
                    wcs_headers.insert("COMMENT".into(), format!("Solved: {}", t));
                    break;
                }
            }
        }
    }

    Ok(SolveResult {
        success: true,
        ra_center,
        dec_center,
        orientation,
        pixel_scale,
        field_w_arcmin: field_w,
        field_h_arcmin: field_w * image_height as f64 / image_width as f64,
        index_name: "astrometry.net".into(),
        stars_used: stars.len().min(config.max_stars.unwrap_or(100)),
        wcs_headers,
    })
}

/// Offline placeholder when `astrometry-net` feature is not enabled.
/// Returns an error indicating the user must provide a WCS or API key.
#[cfg(not(feature = "astrometry-net"))]
pub fn solve_offline_placeholder() -> Result<SolveResult> {
    bail!(
        "Offline plate solving not available. \
         Use astrometry.net API by enabling the 'astrometry-net' feature, \
         or provide an image with WCS headers."
    )
}


#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    fn make_test_image(rows: usize, cols: usize) -> Array2<f32> {
        let mut img = Array2::from_elem((rows, cols), 100.0f32);
        for r in 0..rows {
            for c in 0..cols {
                img[[r, c]] += ((r * 7 + c * 13) % 17) as f32 * 0.5;
            }
        }
        let stars = [(50, 50, 5000.0), (100, 200, 3000.0), (200, 150, 8000.0)];
        for (sy, sx, peak) in &stars {
            for dy in -5i32..=5 {
                for dx in -5i32..=5 {
                    let r = (*sy as i32 + dy) as usize;
                    let c = (*sx as i32 + dx) as usize;
                    if r < rows && c < cols {
                        let d2 = (dx * dx + dy * dy) as f64;
                        let sigma = 2.0;
                        let val = peak * (-d2 / (2.0 * sigma * sigma)).exp();
                        img[[r, c]] += val as f32;
                    }
                }
            }
        }
        img
    }

    #[test]
    fn test_detect_stars_finds_sources() {
        let img = make_test_image(300, 300);
        let result = detect_stars(&img, 5.0);
        assert!(result.stars.len() >= 3, "Should detect at least 3 stars, got {}", result.stars.len());
        assert!(result.background_sigma > 0.0);
    }

    #[test]
    fn test_detect_stars_brightest_first() {
        let img = make_test_image(300, 300);
        let result = detect_stars(&img, 5.0);
        if result.stars.len() >= 2 {
            assert!(result.stars[0].flux >= result.stars[1].flux);
        }
    }

    #[test]
    fn test_detect_stars_centroid_accuracy() {
        let img = make_test_image(300, 300);
        let result = detect_stars(&img, 5.0);
        // The brightest star should be near (150, 200) — center of 8000 peak
        let brightest = &result.stars[0];
        assert!((brightest.x - 150.0).abs() < 2.0, "X centroid off: {}", brightest.x);
        assert!((brightest.y - 200.0).abs() < 2.0, "Y centroid off: {}", brightest.y);
    }

    #[test]
    fn test_detect_stars_empty_image() {
        let img = Array2::from_elem((100, 100), 50.0f32);
        let result = detect_stars(&img, 5.0);
        assert!(result.stars.is_empty(), "Flat image should have no detections");
    }

    #[test]
    fn test_background_estimation() {
        let img = Array2::from_elem((200, 200), 100.0f32);
        let (med, sig) = estimate_background(&img, 64);
        assert!((med - 100.0).abs() < 1.0);
        assert!(sig < 1.0, "Flat image should have near-zero sigma");
    }
}
