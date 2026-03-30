pub use crate::core::analysis::star_detection::DetectedStar;
pub use crate::core::astrometry::plate_solve::{
    FieldAnnotation, SolveConfig, SolveResult,
};
#[cfg(not(feature = "astrometry-net"))]
pub use crate::core::astrometry::plate_solve::solve_offline_placeholder;

#[cfg(feature = "astrometry-net")]
pub use self::astrometry_net_impl::solve_astrometry_net;

#[cfg(feature = "astrometry-net")]
mod astrometry_net_impl {
    use std::collections::HashMap;
    use anyhow::{bail, Context, Result};
    use super::{DetectedStar, FieldAnnotation, SolveResult, SolveConfig};

    const REFERER: &str = "https://nova.astrometry.net/api/login";

    const WCS_KEYS: &[&str] = &[
        "CRPIX1", "CRPIX2", "CRVAL1", "CRVAL2",
        "CD1_1", "CD1_2", "CD2_1", "CD2_2",
        "CDELT1", "CDELT2", "CROTA2",
        "CTYPE1", "CTYPE2", "CUNIT1", "CUNIT2",
        "IMAGEW", "IMAGEH",
        "A_ORDER", "B_ORDER", "AP_ORDER", "BP_ORDER",
    ];

    fn is_wcs_key(key: &str) -> bool {
        if WCS_KEYS.contains(&key) {
            return true;
        }
        let prefixes = ["A_", "B_", "AP_", "BP_"];
        for p in prefixes {
            if key.starts_with(p) {
                let rest = &key[p.len()..];
                if rest.chars().all(|c| c.is_ascii_digit() || c == '_') && !rest.is_empty() {
                    return true;
                }
            }
        }
        false
    }

    fn extract_wcs_headers(fits_bytes: &[u8]) -> Result<HashMap<String, String>> {
        let parsed = crate::infra::fits::reader::parse_header_at(fits_bytes, 0)
            .context("Failed to parse WCS FITS header")?;

        let mut headers = HashMap::new();
        for (key, value) in &parsed.header.cards {
            if is_wcs_key(key) {
                headers.insert(key.clone(), value.clone());
            }
        }
        Ok(headers)
    }

    fn parse_annotations(json: &serde_json::Value) -> Vec<FieldAnnotation> {
        let mut result = Vec::new();
        let annotations = match json["annotations"].as_array() {
            Some(a) => a,
            None => return result,
        };
        for ann in annotations {
            let kind = ann["type"].as_str().unwrap_or("").to_string();
            let names: Vec<String> = ann["names"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let pixelx = ann["pixelx"].as_f64().unwrap_or(0.0);
            let pixely = ann["pixely"].as_f64().unwrap_or(0.0);
            let radius = ann["radius"].as_f64();
            if !kind.is_empty() {
                result.push(FieldAnnotation {
                    kind,
                    names,
                    pixelx,
                    pixely,
                    radius,
                });
            }
        }
        result
    }

    async fn parse_json_response(resp: reqwest::Response, label: &str) -> Result<serde_json::Value> {
        let status = resp.status();
        let body = resp.text().await
            .with_context(|| format!("{}: failed to read response body", label))?;
        if !status.is_success() {
            bail!("{}: HTTP {} -- {}", label, status, body);
        }
        serde_json::from_str(&body)
            .with_context(|| format!("{}: invalid JSON -- {}", label, &body[..body.len().min(200)]))
    }

    pub async fn solve_astrometry_net(
        fits_path: &str,
        stars: &[DetectedStar],
        image_width: usize,
        image_height: usize,
        config: &SolveConfig,
    ) -> Result<SolveResult> {
        use reqwest::Client;
        use reqwest::multipart;

        if config.api_key.is_empty() {
            bail!("No API key configured. Set your astrometry.net key in Settings.");
        }

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        let base_url = &config.api_url;

        let login_body = serde_json::json!({ "apikey": config.api_key });
        let login_resp = client
            .post(format!("{}/api/login", base_url))
            .form(&[("request-json", serde_json::to_string(&login_body)?)])
            .send()
            .await
            .context("Login request failed")?;
        let login_json = parse_json_response(login_resp, "Login").await?;

        let status = login_json["status"].as_str().unwrap_or("");
        if status != "success" {
            bail!(
                "Astrometry.net login failed: {}",
                login_json["errormessage"].as_str().unwrap_or("unknown error")
            );
        }

        let session = login_json["session"]
            .as_str()
            .context("No session in login response")?
            .to_string();

        log::info!("Astrometry.net session: {}", &session[..session.len().min(8)]);

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
            upload_json["scale_type"] = serde_json::json!("ul");
            upload_json["scale_units"] = serde_json::json!("arcsecperpix");
        }

        let file_bytes = std::fs::read(fits_path)
            .with_context(|| format!("Failed to read FITS file: {}", fits_path))?;

        let file_name = std::path::Path::new(fits_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "image.fits".into());

        log::info!("Uploading {} ({} bytes) to astrometry.net", file_name, file_bytes.len());

        let file_part = multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str("application/fits")?;

        let form = multipart::Form::new()
            .text("request-json", serde_json::to_string(&upload_json)?)
            .part("file", file_part);

        let upload_resp = client
            .post(format!("{}/api/upload", base_url))
            .multipart(form)
            .send()
            .await
            .context("Upload request failed")?;
        let upload_data = parse_json_response(upload_resp, "Upload").await?;

        let upload_status = upload_data["status"].as_str().unwrap_or("");
        if upload_status != "success" {
            bail!(
                "Astrometry.net upload failed: {}",
                upload_data["errormessage"].as_str().unwrap_or("unknown error")
            );
        }

        let subid = upload_data["subid"]
            .as_u64()
            .context("No subid in upload response")?;

        log::info!("Submission {}, waiting for job...", subid);

        let mut job_id: Option<u64> = None;
        for attempt in 0..90 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let resp = client
                .get(format!("{}/api/submissions/{}", base_url, subid))
                .send()
                .await?;
            let sub_status = parse_json_response(resp, "Submission status").await?;

            if let Some(jobs) = sub_status["jobs"].as_array() {
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
            if attempt % 10 == 9 {
                log::info!("Still waiting for job after {}s...", (attempt + 1) * 2);
            }
        }

        let jid = job_id.context("Timed out waiting for astrometry.net job (180s)")?;
        log::info!("Job {} started, polling for solution...", jid);

        let mut solved = false;
        for attempt in 0..90 {
            let resp = client
                .get(format!("{}/api/jobs/{}", base_url, jid))
                .send()
                .await?;
            let job_data = parse_json_response(resp, "Job status").await?;

            let status_str = job_data["status"].as_str().unwrap_or("");
            if status_str == "success" {
                solved = true;
                break;
            }
            if status_str == "failure" {
                bail!("Plate solve failed on astrometry.net (job {})", jid);
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if attempt % 10 == 9 {
                log::info!("Job {} still solving after {}s...", jid, (attempt + 1) * 2);
            }
        }

        if !solved {
            bail!("Plate solve timed out after 180s (job {})", jid);
        }

        let cal_resp = client
            .get(format!("{}/api/jobs/{}/calibration", base_url, jid))
            .send()
            .await?;
        let cal = parse_json_response(cal_resp, "Calibration").await?;

        let ra_center = cal["ra"].as_f64().unwrap_or(0.0);
        let dec_center = cal["dec"].as_f64().unwrap_or(0.0);
        let orientation = cal["orientation"].as_f64().unwrap_or(0.0);
        let pixel_scale = cal["pixscale"].as_f64().unwrap_or(0.0);
        let field_w = pixel_scale * image_width as f64 / 60.0;
        let field_h = pixel_scale * image_height as f64 / 60.0;

        log::info!(
            "Solved: RA={:.4} Dec={:.4} scale={:.3}\"/px orient={:.1}deg FOV={:.1}'x{:.1}'",
            ra_center, dec_center, pixel_scale, orientation, field_w, field_h
        );

        let wcs_url = format!(
            "{}/wcs_file/{}",
            base_url.trim_end_matches('/'),
            jid
        );
        let wcs_headers = match client
            .get(&wcs_url)
            .header("Referer", REFERER)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                match resp.bytes().await {
                    Ok(bytes) => {
                        log::info!("Downloaded WCS file ({} bytes)", bytes.len());
                        extract_wcs_headers(&bytes).unwrap_or_else(|e| {
                            log::warn!("Failed to parse WCS FITS: {}", e);
                            fallback_wcs_headers(ra_center, dec_center, pixel_scale, orientation, image_width, image_height)
                        })
                    }
                    Err(e) => {
                        log::warn!("Failed to read WCS response body: {}", e);
                        fallback_wcs_headers(ra_center, dec_center, pixel_scale, orientation, image_width, image_height)
                    }
                }
            }
            Ok(resp) => {
                log::warn!("WCS file download returned HTTP {}", resp.status());
                fallback_wcs_headers(ra_center, dec_center, pixel_scale, orientation, image_width, image_height)
            }
            Err(e) => {
                log::warn!("WCS file download failed: {}", e);
                fallback_wcs_headers(ra_center, dec_center, pixel_scale, orientation, image_width, image_height)
            }
        };

        let annotations = match client
            .get(format!("{}/api/jobs/{}/annotations", base_url, jid))
            .header("Referer", REFERER)
            .send()
            .await
        {
            Ok(resp) => {
                match parse_json_response(resp, "Annotations").await {
                    Ok(json) => parse_annotations(&json),
                    Err(e) => {
                        log::warn!("Failed to parse annotations: {}", e);
                        Vec::new()
                    }
                }
            }
            Err(e) => {
                log::warn!("Annotations request failed: {}", e);
                Vec::new()
            }
        };

        Ok(SolveResult {
            success: true,
            ra_center,
            dec_center,
            orientation,
            pixel_scale,
            field_w_arcmin: field_w,
            field_h_arcmin: field_h,
            index_name: "astrometry.net".into(),
            stars_used: stars.len().min(config.max_stars.unwrap_or(100)),
            wcs_headers,
            annotations,
        })
    }

    fn fallback_wcs_headers(
        ra: f64,
        dec: f64,
        pixel_scale: f64,
        orientation: f64,
        width: usize,
        height: usize,
    ) -> HashMap<String, String> {
        let mut h = HashMap::new();
        h.insert("CRVAL1".into(), format!("{:.8}", ra));
        h.insert("CRVAL2".into(), format!("{:.8}", dec));
        h.insert("CRPIX1".into(), format!("{:.1}", width as f64 / 2.0));
        h.insert("CRPIX2".into(), format!("{:.1}", height as f64 / 2.0));

        let theta = orientation.to_radians();
        let scale_deg = pixel_scale / 3600.0;
        h.insert("CD1_1".into(), format!("{:.12E}", -scale_deg * theta.cos()));
        h.insert("CD1_2".into(), format!("{:.12E}", scale_deg * theta.sin()));
        h.insert("CD2_1".into(), format!("{:.12E}", -scale_deg * theta.sin()));
        h.insert("CD2_2".into(), format!("{:.12E}", scale_deg * theta.cos()));
        h.insert("CTYPE1".into(), "RA---TAN".into());
        h.insert("CTYPE2".into(), "DEC--TAN".into());
        h
    }
}
