use reqwest::{header::ACCEPT, Client, Url};
use serde::Deserialize;

use super::{
    download_url, hex, parse_digest, refused, StoreError, ZenodoConsent, ZenodoFile, ZenodoSource,
    SIRIL_GAIA_ARCHIVE_KEY, SIRIL_GAIA_ARCHIVE_SUM_KEY, SIRIL_GAIA_DECODED_SUM_KEY,
};
use crate::core::astrometry::healpix_cat::SIRIL_ASTROMETRIC_FILE;
use crate::infra::net::http::{host_and_port, net_failure_text, net_kind, status_failure_text};

const ZENODO_RECORD_MAX_BYTES: u64 = 1 << 20;
const ZENODO_SUM_MAX_BYTES: u64 = 64 << 10;
const JSON_MEDIA_TYPE: &str = "application/json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZenodoListing {
    pub doi: Option<String>,
    pub title: Option<String>,
    pub licence: Option<String>,
    pub page: Option<String>,
    pub files: Vec<(ZenodoFile, Url)>,
}

impl ZenodoListing {
    fn file(&self, key: &str) -> Option<&(ZenodoFile, Url)> {
        self.files.iter().find(|(file, _)| file.key == key)
    }
}

#[derive(Deserialize)]
struct RecordJson {
    doi: Option<String>,
    #[serde(default)]
    metadata: RecordMetadataJson,
    #[serde(default)]
    links: RecordLinksJson,
    files: Vec<RecordFileJson>,
}

#[derive(Default, Deserialize)]
struct RecordMetadataJson {
    title: Option<String>,
    license: Option<serde_json::Value>,
}

#[derive(Default, Deserialize)]
struct RecordLinksJson {
    self_html: Option<String>,
}

#[derive(Deserialize)]
struct RecordFileJson {
    key: String,
    size: u64,
    checksum: Option<String>,
    links: RecordFileLinksJson,
}

#[derive(Deserialize)]
struct RecordFileLinksJson {
    #[serde(rename = "self")]
    content: String,
}

pub fn parse_zenodo_record(record_url: &Url, body: &[u8]) -> Result<ZenodoListing, String> {
    let record: RecordJson = serde_json::from_slice(body).map_err(|error| {
        format!("the Zenodo record {record_url} is not the JSON record this app reads ({error})")
    })?;
    let host = record_url.host_str().unwrap_or_default();
    let files = record
        .files
        .into_iter()
        .map(|file| {
            let url = download_url(&file.links.content)
                .map_err(|reason| format!("the Zenodo file {}: {reason}", file.key))?;
            if !url
                .host_str()
                .is_some_and(|linked| linked.eq_ignore_ascii_case(host))
            {
                return Err(format!(
                    "the Zenodo file {} links to {}, not to the record's host {host}",
                    file.key,
                    url.host_str().unwrap_or("no host")
                ));
            }
            let listed = ZenodoFile {
                key: file.key,
                size: file.size,
                checksum: file.checksum,
            };
            Ok((listed, url))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let archive_size = files
        .iter()
        .find(|(file, _)| file.key == SIRIL_GAIA_ARCHIVE_KEY)
        .map(|(file, _)| file.size);
    match archive_size {
        Some(0) => {
            return Err(format!(
                "the Zenodo record {record_url} lists {SIRIL_GAIA_ARCHIVE_KEY} with no size"
            ))
        }
        Some(_) => {}
        None => {
            let listed: Vec<&str> = files.iter().map(|(file, _)| file.key.as_str()).collect();
            return Err(format!(
                "the Zenodo record {record_url} lists no {SIRIL_GAIA_ARCHIVE_KEY}; it lists {}",
                if listed.is_empty() {
                    "no files".to_owned()
                } else {
                    listed.join(", ")
                }
            ));
        }
    }
    let licence = record.metadata.license.and_then(|value| {
        value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.as_str())
            .map(str::to_owned)
    });
    Ok(ZenodoListing {
        doi: record.doi,
        title: record.metadata.title,
        licence,
        page: record.links.self_html,
        files,
    })
}

pub(super) struct ZenodoRecord {
    pub(super) record_url: Url,
    pub(super) listing: ZenodoListing,
    pub(super) archive_url: Url,
    pub(super) archive_bytes: u64,
    pub(super) archive_sha256: Option<[u8; 32]>,
    pub(super) decoded_sha256: Option<[u8; 32]>,
}

impl ZenodoRecord {
    pub(super) fn check_consent(
        &self,
        id: &str,
        consent: &ZenodoConsent,
    ) -> Result<(), StoreError> {
        let agreed = consent
            .archive_sha256
            .as_deref()
            .map(str::to_ascii_lowercase);
        let published = self.archive_sha256.map(|digest| hex(&digest));
        if consent.archive_bytes == self.archive_bytes && agreed == published {
            return Ok(());
        }
        let shown =
            |digest: &Option<String>| digest.clone().unwrap_or_else(|| "not published".to_owned());
        Err(refused(
            id,
            format!(
                "the Zenodo record changed since you agreed to the download: it now lists {SIRIL_GAIA_ARCHIVE_KEY} at {} bytes with SHA-256 {}, you agreed to {} bytes with SHA-256 {}; review the download in Settings > Catalogs again",
                self.archive_bytes,
                shown(&published),
                consent.archive_bytes,
                shown(&agreed)
            ),
        ))
    }
}

pub(super) async fn fetch_zenodo_record(
    id: &str,
    client: &Client,
    source: &ZenodoSource,
) -> Result<ZenodoRecord, StoreError> {
    let record_url = download_url(&source.record_url).map_err(|reason| refused(id, reason))?;
    let body = fetch_small(
        id,
        client,
        &record_url,
        Some(JSON_MEDIA_TYPE),
        ZENODO_RECORD_MAX_BYTES,
        None,
    )
    .await?;
    let listing = parse_zenodo_record(&record_url, &body).map_err(|reason| refused(id, reason))?;
    let (archive, archive_url) =
        listing
            .file(SIRIL_GAIA_ARCHIVE_KEY)
            .cloned()
            .ok_or_else(|| {
                refused(
                    id,
                    format!("the Zenodo record lists no {SIRIL_GAIA_ARCHIVE_KEY}"),
                )
            })?;
    let archive_sha256 = published_sum(
        id,
        client,
        &listing,
        SIRIL_GAIA_ARCHIVE_SUM_KEY,
        SIRIL_GAIA_ARCHIVE_KEY,
    )
    .await?;
    let decoded_sha256 = published_sum(
        id,
        client,
        &listing,
        SIRIL_GAIA_DECODED_SUM_KEY,
        SIRIL_ASTROMETRIC_FILE,
    )
    .await?;
    Ok(ZenodoRecord {
        record_url,
        listing,
        archive_url,
        archive_bytes: archive.size,
        archive_sha256,
        decoded_sha256,
    })
}

async fn published_sum(
    id: &str,
    client: &Client,
    listing: &ZenodoListing,
    sum_key: &str,
    file_name: &str,
) -> Result<Option<[u8; 32]>, StoreError> {
    let Some((file, url)) = listing.file(sum_key) else {
        return Ok(None);
    };
    if file.size > ZENODO_SUM_MAX_BYTES {
        return Err(refused(
            id,
            format!(
                "the Zenodo record lists {sum_key} at {} bytes, more than a SHA-256 list holds",
                file.size
            ),
        ));
    }
    let body = fetch_small(id, client, url, None, ZENODO_SUM_MAX_BYTES, Some(sum_key)).await?;
    if body.len() as u64 != file.size {
        return Err(refused(
            id,
            format!(
                "{sum_key} arrived with {} bytes where the Zenodo record lists {}",
                body.len(),
                file.size
            ),
        ));
    }
    std::str::from_utf8(&body)
        .ok()
        .and_then(|text| parse_sha256sum(text, file_name))
        .map(Some)
        .ok_or_else(|| {
            refused(
                id,
                format!("{sum_key} on Zenodo names no SHA-256 for {file_name}"),
            )
        })
}

async fn fetch_small(
    id: &str,
    client: &Client,
    url: &Url,
    accept: Option<&str>,
    cap: u64,
    what: Option<&str>,
) -> Result<Vec<u8>, StoreError> {
    let host = host_and_port(url);
    let what = what.map_or_else(String::new, |what| format!("{what}: "));
    let unreadable = |error: reqwest::Error| {
        refused(id, format!("{what}{}", net_failure_text(net_kind(&error), &host)))
    };
    let too_large = || refused(id, format!("{what}{host} sent more than {cap} bytes"));
    let request = client.get(url.clone());
    let request = match accept {
        Some(media_type) => request.header(ACCEPT, media_type),
        None => request,
    };
    let mut response = request.send().await.map_err(unreadable)?;
    let status = response.status();
    if !status.is_success() {
        return Err(refused(
            id,
            format!("{what}{}", status_failure_text(status.as_u16(), &host)),
        ));
    }
    if response.content_length().is_some_and(|len| len > cap) {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(unreadable)? {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > cap {
            return Err(too_large());
        }
    }
    Ok(body)
}

pub fn parse_sha256sum(text: &str, file_name: &str) -> Option<[u8; 32]> {
    text.lines().find_map(|line| {
        let (digest, name) = line.trim_end().split_once(char::is_whitespace)?;
        let name = name.trim_start();
        let name = name.strip_prefix('*').unwrap_or(name);
        if name == file_name {
            parse_digest(digest)
        } else {
            None
        }
    })
}
