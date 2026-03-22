pub use crate::core::analysis::star_detection::DetectedStar;
pub use crate::core::astrometry::plate_solve::{
    SolveResult, SolveConfig, solve_offline_placeholder,
};

#[cfg(feature = "astrometry-net")]
pub use self::astrometry_net_impl::solve_astrometry_net;

#[cfg(feature = "astrometry-net")]
mod astrometry_net_impl {
    use std::collections::HashMap;
    use anyhow::{bail, Context, Result};
    use super::{DetectedStar, SolveResult, SolveConfig};

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
            upload_json["scale_type"] = serde_json::json!("arcsecperpix");
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
        let field_w = cal["radius"].as_f64().unwrap_or(0.0) * 2.0 * 60.0;

        log::info!(
            "Solved: RA={:.4} Dec={:.4} scale={:.3}\"/px orient={:.1}deg",
            ra_center, dec_center, pixel_scale, orientation
        );

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

        let info_resp = client
            .get(format!("{}/api/jobs/{}/info", base_url, jid))
            .send()
            .await?;
        let wcs_info = parse_json_response(info_resp, "Job info").await?;

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
}
