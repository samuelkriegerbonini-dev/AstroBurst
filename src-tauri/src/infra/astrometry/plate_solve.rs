pub use crate::core::astrometry::plate_solve::{
    FieldAnnotation, SolveConfig, SolveResult,
};
#[cfg(not(feature = "astrometry-net"))]
pub use crate::core::astrometry::plate_solve::solve_offline_placeholder;

#[cfg(feature = "astrometry-net")]
pub use self::astrometry_net_impl::solve_astrometry_net;

#[cfg(feature = "astrometry-net")]
mod astrometry_net_impl {
    use anyhow::{bail, Context, Result};
    use super::{FieldAnnotation, SolveResult, SolveConfig};

    const REFERER: &str = "https://nova.astrometry.net/api/login";
    const ERROR_SNIPPET_CHARS: usize = 200;
    const SESSION_LOG_CHARS: usize = 8;
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
    const POLLS_PER_LOG: u64 = 10;

    pub(super) fn char_prefix(text: &str, max_chars: usize) -> &str {
        text.char_indices().nth(max_chars).map_or(text, |(end, _)| &text[..end])
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

    pub(super) fn parse_json_body(body: &str, label: &str) -> Result<serde_json::Value> {
        serde_json::from_str(body).with_context(|| {
            format!("{}: invalid JSON -- {}", label, char_prefix(body, ERROR_SNIPPET_CHARS))
        })
    }

    async fn parse_json_response(resp: reqwest::Response, label: &str) -> Result<serde_json::Value> {
        let status = resp.status();
        let body = resp.text().await
            .with_context(|| format!("{}: failed to read response body", label))?;
        if !status.is_success() {
            bail!("{}: HTTP {} -- {}", label, status, body);
        }
        parse_json_body(&body, label)
    }

    pub(super) fn first_job_id(submission: &serde_json::Value) -> Option<u64> {
        submission["jobs"].as_array()?.iter().filter_map(|j| j.as_u64()).find(|&id| id > 0)
    }

    pub async fn solve_astrometry_net(
        fits_path: &str,
        image_width: usize,
        image_height: usize,
        config: &SolveConfig,
    ) -> Result<SolveResult> {
        let limit = std::time::Duration::from_secs(config.timeout_secs);
        match tokio::time::timeout(limit, solve_before_deadline(fits_path, image_width, image_height, config)).await {
            Ok(result) => result,
            Err(_) => bail!(
                "Plate solve timed out after {} s; raise the plate-solve timeout in Settings",
                config.timeout_secs
            ),
        }
    }

    async fn solve_before_deadline(
        fits_path: &str,
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

        log::info!("Astrometry.net session: {}", char_prefix(&session, SESSION_LOG_CHARS));

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
            upload_json["scale_units"] =
                serde_json::json!(config.scale_units.as_deref().unwrap_or("arcsecperpix"));
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

        let mut polls: u64 = 0;
        let jid = loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            polls += 1;
            let resp = client
                .get(format!("{}/api/submissions/{}", base_url, subid))
                .send()
                .await?;
            let sub_status = parse_json_response(resp, "Submission status").await?;
            if let Some(id) = first_job_id(&sub_status) {
                break id;
            }
            if polls % POLLS_PER_LOG == 0 {
                log::info!("Still waiting for job after {}s...", polls * POLL_INTERVAL.as_secs());
            }
        };
        log::info!("Job {} started, polling for solution...", jid);

        let mut polls: u64 = 0;
        loop {
            let resp = client
                .get(format!("{}/api/jobs/{}", base_url, jid))
                .send()
                .await?;
            let job_data = parse_json_response(resp, "Job status").await?;
            match job_data["status"].as_str().unwrap_or("") {
                "success" => break,
                "failure" => bail!("Plate solve failed on astrometry.net (job {})", jid),
                _ => {}
            }
            tokio::time::sleep(POLL_INTERVAL).await;
            polls += 1;
            if polls % POLLS_PER_LOG == 0 {
                log::info!("Job {} still solving after {}s...", jid, polls * POLL_INTERVAL.as_secs());
            }
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
            ra_center,
            dec_center,
            orientation,
            pixel_scale,
            field_w_arcmin: field_w,
            field_h_arcmin: field_h,
            annotations,
        })
    }
}

#[cfg(all(test, feature = "astrometry-net"))]
mod tests {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    use super::astrometry_net_impl::{char_prefix, first_job_id, parse_json_body};
    use super::{solve_astrometry_net, SolveConfig};

    fn read_request_line(stream: &mut std::net::TcpStream) -> String {
        let mut data: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..end]).to_string();
                let lower = head.to_ascii_lowercase();
                let body = &data[end + 4..];
                let complete = if lower.contains("transfer-encoding: chunked") {
                    body.ends_with(b"0\r\n\r\n")
                } else {
                    let expected = lower
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    body.len() >= expected
                };
                if complete {
                    return head.lines().next().unwrap_or_default().to_string();
                }
            }
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return String::new(),
                Ok(n) => data.extend_from_slice(&buf[..n]),
            }
        }
    }

    fn serve_a_submission_that_never_starts(seen: Arc<Mutex<Vec<String>>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let line = read_request_line(&mut stream);
                let body = if line.starts_with("POST /api/login") {
                    r#"{"status":"success","session":"test-session"}"#
                } else if line.starts_with("POST /api/upload") {
                    r#"{"status":"success","subid":7}"#
                } else {
                    r#"{"jobs":[]}"#
                };
                seen.lock().unwrap().push(line);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
        });
        url
    }

    #[tokio::test]
    async fn the_configured_timeout_bounds_the_whole_solve() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let api_url = serve_a_submission_that_never_starts(Arc::clone(&seen));
        let dir = tempfile::tempdir().unwrap();
        let fits = dir.path().join("upload.fits");
        std::fs::write(&fits, vec![b' '; 2880]).unwrap();
        let config = SolveConfig {
            api_url,
            api_key: "key".into(),
            ra_hint: None,
            dec_hint: None,
            radius_hint: None,
            scale_low: None,
            scale_high: None,
            scale_units: None,
            timeout_secs: 1,
        };

        let started = std::time::Instant::now();
        let err = solve_astrometry_net(fits.to_str().unwrap(), 10, 10, &config).await.unwrap_err().to_string();
        let elapsed = started.elapsed();
        assert!(err.contains("timed out after 1 s"), "{err}");
        assert!(elapsed < std::time::Duration::from_secs(10), "the solve ran for {elapsed:?}");
        let seen = seen.lock().unwrap().clone();
        assert!(seen.iter().any(|l| l.starts_with("POST /api/login")), "{seen:?}");
        assert!(seen.iter().any(|l| l.starts_with("POST /api/upload")), "{seen:?}");
    }

    #[test]
    fn the_first_positive_job_id_is_taken() {
        assert_eq!(first_job_id(&serde_json::json!({ "jobs": [null, 0, 42, 43] })), Some(42));
        assert_eq!(first_job_id(&serde_json::json!({ "jobs": [] })), None);
        assert_eq!(first_job_id(&serde_json::json!({})), None);
    }

    #[test]
    fn invalid_json_error_truncates_on_a_char_boundary() {
        let body = format!("<html>{}\u{2014}{}</html>", "a".repeat(193), "b".repeat(300));
        assert!(!body.is_char_boundary(200), "the em dash must straddle byte 200");
        let err = parse_json_body(&body, "Calibration").unwrap_err();
        let text = format!("{:#}", err);
        assert!(text.starts_with("Calibration: invalid JSON -- <html>"), "{text}");
        assert!(text.contains('\u{2014}'), "{text}");
        assert!(!text.contains(&"b".repeat(200)), "the snippet is capped: {text}");

        assert_eq!(char_prefix("abc", 8), "abc");
        assert_eq!(char_prefix("\u{e9}\u{e9}\u{e9}", 2), "\u{e9}\u{e9}");
        assert_eq!(char_prefix("", 3), "");
    }
}
