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
    const GET_ATTEMPTS: u32 = 3;
    const RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_secs(1);
    const POOL_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);

    fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
        let mut parts = vec![error.to_string()];
        let mut source = error.source();
        while let Some(cause) = source {
            let text = cause.to_string();
            if !parts.iter().any(|p| p.contains(&text)) {
                parts.push(text);
            }
            source = cause.source();
        }
        parts.join(": ")
    }

    fn is_transient(error: &reqwest::Error) -> bool {
        error.is_connect() || error.is_request() || error.is_timeout() || error.is_body()
    }

    async fn get_with_retry(
        client: &reqwest::Client,
        url: &str,
        label: &str,
        subject: &str,
        referer: bool,
    ) -> Result<(reqwest::StatusCode, Vec<u8>)> {
        let mut last_error: Option<reqwest::Error> = None;
        for attempt in 1..=GET_ATTEMPTS {
            if attempt > 1 {
                tokio::time::sleep(RETRY_BACKOFF * (attempt - 1)).await;
            }
            let mut request = client.get(url);
            if referer {
                request = request.header("Referer", REFERER);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) if is_transient(&error) => {
                    log::warn!("{label} ({subject}) attempt {attempt}/{GET_ATTEMPTS}: {}", error_chain(&error));
                    last_error = Some(error);
                    continue;
                }
                Err(error) => bail!("{label} request failed ({subject}): {}", error_chain(&error)),
            };
            let status = response.status();
            if status.is_server_error() && attempt < GET_ATTEMPTS {
                log::warn!("{label} ({subject}) attempt {attempt}/{GET_ATTEMPTS}: HTTP {status}");
                continue;
            }
            match response.bytes().await {
                Ok(body) => return Ok((status, body.to_vec())),
                Err(error) if is_transient(&error) => {
                    log::warn!("{label} ({subject}) body attempt {attempt}/{GET_ATTEMPTS}: {}", error_chain(&error));
                    last_error = Some(error);
                }
                Err(error) => bail!("{label}: failed to read response body ({subject}): {}", error_chain(&error)),
            }
        }
        match last_error {
            Some(error) => bail!(
                "{label} request failed after {GET_ATTEMPTS} attempts ({subject}): {}",
                error_chain(&error)
            ),
            None => bail!("{label} request failed after {GET_ATTEMPTS} attempts ({subject})"),
        }
    }

    async fn get_json(client: &reqwest::Client, url: &str, label: &str, subject: &str) -> Result<serde_json::Value> {
        let (status, body) = get_with_retry(client, url, label, subject, false).await?;
        let body = String::from_utf8_lossy(&body);
        if !status.is_success() {
            bail!("{}: HTTP {} -- {}", label, status, char_prefix(&body, ERROR_SNIPPET_CHARS));
        }
        parse_json_body(&body, label)
    }

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

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub(super) struct Calibration {
        pub ra: f64,
        pub dec: f64,
        pub orientation: f64,
        pub pixscale: f64,
    }

    pub(super) fn parse_calibration(cal: &serde_json::Value, jid: u64) -> Result<Calibration> {
        let finite = |key: &str| cal[key].as_f64().filter(|v| v.is_finite());
        let unusable = |key: &str| {
            anyhow::anyhow!("astrometry.net calibration for job {jid} has no usable {key}; the solve cannot be used")
        };
        let ra = finite("ra").ok_or_else(|| unusable("ra"))?;
        let dec = finite("dec").ok_or_else(|| unusable("dec"))?;
        let pixscale = finite("pixscale").filter(|v| *v > 0.0).ok_or_else(|| unusable("pixscale"))?;
        Ok(Calibration {
            ra,
            dec,
            orientation: cal["orientation"].as_f64().unwrap_or(0.0),
            pixscale,
        })
    }

    pub(super) fn wcs_cards_from_wcs_file(bytes: &[u8]) -> Result<Vec<(String, String)>> {
        let header = crate::infra::fits::reader::parse_header_at(bytes, 0)
            .context("astrometry.net wcs_file is not a FITS header")?
            .header;
        let cards: Vec<(String, String)> = header
            .cards
            .iter()
            .filter(|(key, _)| crate::infra::fits::writer::is_wcs_card(key.trim()))
            .map(|(key, value)| (key.trim().to_string(), value.clone()))
            .collect();
        for key in ["CRVAL1", "CRVAL2", "CRPIX1", "CRPIX2"] {
            if !cards.iter().any(|(k, _)| k == key) {
                bail!("astrometry.net wcs_file has no {key}");
            }
        }
        Ok(cards)
    }

    async fn fetch_wcs_cards(client: &reqwest::Client, base_url: &str, jid: u64) -> Result<Vec<(String, String)>> {
        let url = format!("{}/wcs_file/{}", base_url, jid);
        let (status, bytes) = get_with_retry(client, &url, "wcs_file", &format!("job {jid}"), true).await?;
        if !status.is_success() {
            bail!("wcs_file: HTTP {}", status);
        }
        wcs_cards_from_wcs_file(&bytes)
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
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
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
            let sub_status = get_json(
                &client,
                &format!("{}/api/submissions/{}", base_url, subid),
                "Submission status",
                &format!("submission {subid}"),
            )
            .await?;
            if let Some(id) = first_job_id(&sub_status) {
                break id;
            }
            if polls % POLLS_PER_LOG == 0 {
                log::info!("Still waiting for job after {}s...", polls * POLL_INTERVAL.as_secs());
            }
        };
        log::info!("Job {} started, polling for solution...", jid);

        let mut polls: u64 = 0;
        let job_subject = format!("job {jid}");
        loop {
            let job_data =
                get_json(&client, &format!("{}/api/jobs/{}", base_url, jid), "Job status", &job_subject).await?;
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

        let cal = get_json(
            &client,
            &format!("{}/api/jobs/{}/calibration", base_url, jid),
            "Calibration",
            &job_subject,
        )
        .await?;

        let Calibration { ra: ra_center, dec: dec_center, orientation, pixscale: pixel_scale } =
            parse_calibration(&cal, jid)?;
        let field_w = pixel_scale * image_width as f64 / 60.0;
        let field_h = pixel_scale * image_height as f64 / 60.0;

        log::info!(
            "Solved: RA={:.4} Dec={:.4} scale={:.3}\"/px orient={:.1}deg FOV={:.1}'x{:.1}'",
            ra_center, dec_center, pixel_scale, orientation, field_w, field_h
        );

        let annotations_url = format!("{}/api/jobs/{}/annotations", base_url, jid);
        let annotations = match get_with_retry(&client, &annotations_url, "Annotations", &job_subject, true).await {
            Ok((status, body)) if status.is_success() => {
                match parse_json_body(&String::from_utf8_lossy(&body), "Annotations") {
                    Ok(json) => parse_annotations(&json),
                    Err(e) => {
                        log::warn!("Failed to parse annotations: {:#}", e);
                        Vec::new()
                    }
                }
            }
            Ok((status, _)) => {
                log::warn!("Annotations request for job {}: HTTP {}", jid, status);
                Vec::new()
            }
            Err(e) => {
                log::warn!("Annotations request failed: {:#}", e);
                Vec::new()
            }
        };

        let wcs_cards = fetch_wcs_cards(&client, base_url, jid).await.unwrap_or_else(|e| {
            log::warn!("WCS file for job {} unavailable: {:#}", jid, e);
            Vec::new()
        });

        Ok(SolveResult {
            ra_center,
            dec_center,
            orientation,
            pixel_scale,
            field_w_arcmin: field_w,
            field_h_arcmin: field_h,
            annotations,
            wcs_cards,
        })
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    const STRING_KEYS: &[&str] = &["CTYPE1", "CTYPE2", "CUNIT1", "CUNIT2", "DATE"];

    pub(crate) fn nova_wcs_cards() -> Vec<(String, String)> {
        [
            ("WCSAXES", "2"),
            ("CTYPE1", "RA---TAN-SIP"),
            ("CTYPE2", "DEC--TAN-SIP"),
            ("EQUINOX", "2000.0"),
            ("LONPOLE", "180.0"),
            ("LATPOLE", "0.0"),
            ("CRVAL1", "83.822"),
            ("CRVAL2", "-5.391"),
            ("CRPIX1", "512.5"),
            ("CRPIX2", "341.5"),
            ("CUNIT1", "deg"),
            ("CUNIT2", "deg"),
            ("CD1_1", "-5.55E-04"),
            ("CD1_2", "1.23E-05"),
            ("CD2_1", "1.21E-05"),
            ("CD2_2", "5.56E-04"),
            ("A_ORDER", "2"),
            ("A_0_2", "-3.0E-08"),
            ("A_1_1", "5.0E-08"),
            ("A_2_0", "1.2E-07"),
            ("B_ORDER", "2"),
            ("B_0_2", "1.1E-07"),
            ("B_1_1", "-4.0E-08"),
            ("B_2_0", "2.0E-08"),
            ("AP_ORDER", "2"),
            ("AP_0_2", "3.0E-08"),
            ("AP_1_1", "-5.0E-08"),
            ("AP_2_0", "-1.2E-07"),
            ("BP_ORDER", "2"),
            ("BP_0_2", "-1.1E-07"),
            ("BP_1_1", "4.0E-08"),
            ("BP_2_0", "-2.0E-08"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn card(key: &str, value: &str) -> String {
        let text = if STRING_KEYS.contains(&key) {
            format!("{:<8}= '{}'", key, value)
        } else {
            format!("{:<8}= {:>20}", key, value)
        };
        format!("{:<80}", text)
    }

    pub(crate) fn nova_wcs_file_from(wcs_cards: &[(String, String)]) -> Vec<u8> {
        let mut text = String::new();
        for (k, v) in [("SIMPLE", "T"), ("BITPIX", "8"), ("NAXIS", "0"), ("EXTEND", "T")] {
            text.push_str(&card(k, v));
        }
        for (k, v) in wcs_cards {
            text.push_str(&card(k, v));
            if k == "CD2_2" {
                text.push_str(&card("IMAGEW", "1024"));
                text.push_str(&card("IMAGEH", "683"));
            }
        }
        text.push_str(&card("DATE", "2026-10-02T12:00:00"));
        text.push_str(&format!("{:<80}", "COMMENT Solved by the offline test fixture"));
        text.push_str(&format!("{:<80}", "HISTORY Created by the Astrometry.net suite."));
        text.push_str(&format!("{:<80}", "END"));
        while text.len() % 2880 != 0 {
            text.push(' ');
        }
        text.into_bytes()
    }

    pub(crate) fn nova_wcs_file() -> Vec<u8> {
        nova_wcs_file_from(&nova_wcs_cards())
    }
}

#[cfg(all(test, feature = "astrometry-net"))]
mod tests {
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::astrometry_net_impl::{
        char_prefix, first_job_id, parse_calibration, parse_json_body, wcs_cards_from_wcs_file,
    };
    use super::test_support::{nova_wcs_cards, nova_wcs_file, nova_wcs_file_from};
    use super::{solve_astrometry_net, SolveConfig};

    const FULL_CALIBRATION: &str =
        r#"{"ra":83.822,"dec":-5.391,"orientation":91.2,"pixscale":2.0,"parity":1.0,"radius":0.4}"#;

    fn solve_config(api_url: String) -> SolveConfig {
        SolveConfig {
            api_url,
            api_key: "key".into(),
            ra_hint: None,
            dec_hint: None,
            radius_hint: None,
            scale_low: None,
            scale_high: None,
            scale_units: None,
            timeout_secs: 30,
        }
    }

    fn respond(stream: &mut std::net::TcpStream, status: &str, content_type: &str, body: &[u8]) {
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(body);
    }

    fn serve_a_solved_job(calibration: &'static str, wcs_file: Option<Vec<u8>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let line = read_request_line(&mut stream);
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                let json = "application/json";
                match path.as_str() {
                    "/api/login" => respond(&mut stream, "200 OK", json, br#"{"status":"success","session":"test-session"}"#),
                    "/api/upload" => respond(&mut stream, "200 OK", json, br#"{"status":"success","subid":7}"#),
                    "/api/submissions/7" => respond(&mut stream, "200 OK", json, br#"{"jobs":[42]}"#),
                    "/api/jobs/42" => respond(&mut stream, "200 OK", json, br#"{"status":"success"}"#),
                    "/api/jobs/42/calibration" => respond(&mut stream, "200 OK", json, calibration.as_bytes()),
                    "/api/jobs/42/annotations" => respond(&mut stream, "200 OK", json, br#"{"annotations":[]}"#),
                    "/wcs_file/42" => match &wcs_file {
                        Some(bytes) => respond(&mut stream, "200 OK", "application/fits", bytes),
                        None => respond(&mut stream, "404 Not Found", "text/plain", b"no wcs file"),
                    },
                    _ => respond(&mut stream, "404 Not Found", "text/plain", b"unknown route"),
                }
            }
        });
        url
    }

    fn upload_fixture(dir: &tempfile::TempDir) -> String {
        let fits = dir.path().join("upload.fits");
        std::fs::write(&fits, vec![b' '; 2880]).unwrap();
        fits.to_str().unwrap().to_string()
    }

    #[test]
    fn calibration_without_ra_dec_or_pixscale_is_refused() {
        let err = parse_calibration(&json!({"dec": -5.391, "pixscale": 2.0}), 42).unwrap_err().to_string();
        assert!(err.contains("job 42") && err.contains("no usable ra;"), "{err}");
        let err = parse_calibration(&json!({"ra": 83.822, "pixscale": 2.0}), 7).unwrap_err().to_string();
        assert!(err.contains("job 7") && err.contains("no usable dec;"), "{err}");
        let err = parse_calibration(&json!({"ra": 83.822, "dec": -5.391, "pixscale": 0}), 42).unwrap_err().to_string();
        assert!(err.contains("no usable pixscale;"), "{err}");
        let err = parse_calibration(&json!({"ra": 83.822, "dec": -5.391}), 42).unwrap_err().to_string();
        assert!(err.contains("no usable pixscale;"), "{err}");

        let cal = parse_calibration(&serde_json::from_str(FULL_CALIBRATION).unwrap(), 42).unwrap();
        assert_eq!((cal.ra, cal.dec, cal.orientation, cal.pixscale), (83.822, -5.391, 91.2, 2.0));
        let no_orientation = parse_calibration(&json!({"ra": 1.0, "dec": 2.0, "pixscale": 3.0}), 1).unwrap();
        assert_eq!(no_orientation.orientation, 0.0);
    }

    #[test]
    fn wcs_file_keeps_only_the_wcs_cards() {
        let cards = wcs_cards_from_wcs_file(&nova_wcs_file()).unwrap();
        let value = |key: &str| cards.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        assert_eq!(value("CTYPE1"), Some("RA---TAN-SIP"));
        assert_eq!(value("A_ORDER"), Some("2"));
        assert_eq!(value("AP_2_0"), Some("-1.2E-07"));
        assert_eq!(value("CRPIX1"), Some("512.5"));
        for key in ["IMAGEW", "IMAGEH", "DATE", "COMMENT", "HISTORY", "SIMPLE", "NAXIS", "BITPIX", "EXTEND"] {
            assert_eq!(value(key), None, "{key} leaked into the WCS cards");
        }
        assert_eq!(cards, nova_wcs_cards());

        let without_crval: Vec<(String, String)> =
            nova_wcs_cards().into_iter().filter(|(k, _)| k != "CRVAL2").collect();
        let err = wcs_cards_from_wcs_file(&nova_wcs_file_from(&without_crval)).unwrap_err().to_string();
        assert!(err.contains("has no CRVAL2"), "{err}");
        assert!(wcs_cards_from_wcs_file(b"not a fits header").is_err());
    }

    #[tokio::test]
    async fn a_successful_solve_returns_the_wcs_cards_of_the_job() {
        let api_url = serve_a_solved_job(FULL_CALIBRATION, Some(nova_wcs_file()));
        let dir = tempfile::tempdir().unwrap();
        let result = solve_astrometry_net(&upload_fixture(&dir), 1024, 683, &solve_config(api_url)).await.unwrap();
        assert_eq!(result.ra_center, 83.822);
        assert_eq!(result.dec_center, -5.391);
        assert_eq!(result.pixel_scale, 2.0);
        assert_eq!(result.orientation, 91.2);
        assert!((result.field_w_arcmin - 2.0 * 1024.0 / 60.0).abs() < 1e-9);
        assert!(!result.wcs_cards.is_empty(), "the solve carries no WCS cards");
        assert_eq!(result.wcs_cards, nova_wcs_cards());
    }

    #[tokio::test]
    async fn a_missing_wcs_file_keeps_the_solve_with_no_cards() {
        let api_url = serve_a_solved_job(FULL_CALIBRATION, None);
        let dir = tempfile::tempdir().unwrap();
        let result = solve_astrometry_net(&upload_fixture(&dir), 1024, 683, &solve_config(api_url)).await.unwrap();
        assert_eq!(result.ra_center, 83.822);
        assert!(result.wcs_cards.is_empty(), "{:?}", result.wcs_cards);
    }

    #[tokio::test]
    async fn a_calibration_without_ra_fails_the_solve() {
        let api_url =
            serve_a_solved_job(r#"{"dec":-5.391,"orientation":91.2,"pixscale":2.0}"#, Some(nova_wcs_file()));
        let dir = tempfile::tempdir().unwrap();
        let err = solve_astrometry_net(&upload_fixture(&dir), 1024, 683, &solve_config(api_url))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("job 42") && err.contains("no usable ra;"), "{err}");
    }

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

    fn serve_a_flaky_solved_job(
        seen: Arc<Mutex<Vec<String>>>,
        dropped_calibrations: usize,
        busy_job_polls: usize,
    ) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut calibrations = 0usize;
            let mut job_polls = 0usize;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let line = read_request_line(&mut stream);
                let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                seen.lock().unwrap().push(path.clone());
                let json = "application/json";
                match path.as_str() {
                    "/api/login" => respond(&mut stream, "200 OK", json, br#"{"status":"success","session":"test-session"}"#),
                    "/api/upload" => respond(&mut stream, "200 OK", json, br#"{"status":"success","subid":7}"#),
                    "/api/submissions/7" => respond(&mut stream, "200 OK", json, br#"{"jobs":[42]}"#),
                    "/api/jobs/42" => {
                        job_polls += 1;
                        if job_polls <= busy_job_polls {
                            respond(&mut stream, "503 Service Unavailable", "text/html", b"<html>overloaded</html>");
                        } else {
                            respond(&mut stream, "200 OK", json, br#"{"status":"success"}"#);
                        }
                    }
                    "/api/jobs/42/calibration" => {
                        calibrations += 1;
                        if calibrations <= dropped_calibrations {
                            let _ = stream.shutdown(std::net::Shutdown::Both);
                        } else {
                            respond(&mut stream, "200 OK", json, FULL_CALIBRATION.as_bytes());
                        }
                    }
                    "/api/jobs/42/annotations" => respond(&mut stream, "200 OK", json, br#"{"annotations":[]}"#),
                    "/wcs_file/42" => respond(&mut stream, "200 OK", "application/fits", &nova_wcs_file()),
                    _ => respond(&mut stream, "404 Not Found", "text/plain", b"unknown route"),
                }
            }
        });
        url
    }

    fn count(seen: &Arc<Mutex<Vec<String>>>, path: &str) -> usize {
        seen.lock().unwrap().iter().filter(|p| p.as_str() == path).count()
    }

    #[tokio::test]
    async fn a_dropped_calibration_connection_is_retried_and_the_solve_succeeds() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let api_url = serve_a_flaky_solved_job(Arc::clone(&seen), 1, 0);
        let dir = tempfile::tempdir().unwrap();
        let result = solve_astrometry_net(&upload_fixture(&dir), 1024, 683, &solve_config(api_url)).await;
        let result = result.unwrap_or_else(|e| panic!("the solve failed on one dropped connection: {e:#}"));
        assert_eq!(result.ra_center, 83.822);
        assert_eq!(result.wcs_cards, nova_wcs_cards());
        assert_eq!(count(&seen, "/api/jobs/42/calibration"), 2);
    }

    #[tokio::test]
    async fn an_overloaded_job_status_is_polled_again() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let api_url = serve_a_flaky_solved_job(Arc::clone(&seen), 0, 1);
        let dir = tempfile::tempdir().unwrap();
        let result = solve_astrometry_net(&upload_fixture(&dir), 1024, 683, &solve_config(api_url)).await;
        let result = result.unwrap_or_else(|e| panic!("the solve failed on one HTTP 503: {e:#}"));
        assert_eq!(result.dec_center, -5.391);
        assert_eq!(count(&seen, "/api/jobs/42"), 2);
    }

    #[tokio::test]
    async fn a_calibration_that_never_answers_names_the_cause() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let api_url = serve_a_flaky_solved_job(Arc::clone(&seen), usize::MAX, 0);
        let dir = tempfile::tempdir().unwrap();
        let err = solve_astrometry_net(&upload_fixture(&dir), 1024, 683, &solve_config(api_url))
            .await
            .unwrap_err();
        let text = format!("{err:#}");
        assert!(text.starts_with("Calibration request failed after 3 attempts"), "{text}");
        assert!(text.contains("job 42"), "{text}");
        assert!(text.matches(": ").count() >= 2, "the transport cause is missing: {text}");
        assert_eq!(count(&seen, "/api/jobs/42/calibration"), 3);
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
