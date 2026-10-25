use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use reqwest::{Client, Url};

use super::error::MastError;
use super::settings::MastSettings;
use crate::core::mast::errors::{
    bad_input, cancelled, classify, internal, io_cause, transport_message, OpKind, Operation,
};
use crate::core::mast::ids::{path_component, validate_dataset_id, validate_filename};
use crate::core::mast::types::{
    MastCmdError, MastDownloadAction, MastDownloadInput, MastErrorCode, MastErrorContext,
    MastMissionId,
};
use crate::infra::net::download::DownloadError;
use crate::infra::net::http::{build_client, HttpProfile, RedirectRule};
use crate::infra::net::paths::{self, DataRoot};

pub const URI_MAX: usize = 512;
pub const MISC_PROGRAM: &str = "misc";
pub const HLSP_FOLDER: &str = "HLSP";
pub const DOWNLOAD_FILE_PATH: &str = "api/v0.1/Download/file";
pub const MAX_PRODUCT_BYTES: u64 = 20_000_000_000;
pub const TRANSFER_REDIRECT_HOSTS: [&str; 2] =
    ["mast.stsci.edu", "stpubdata.s3.us-east-1.amazonaws.com"];
pub const QUOTA_FOLDERS: [&str; 4] = ["JWST", "HST", "TESS", HLSP_FOLDER];
const MAST_SCHEME: &str = "mast:";
const PRODUCT_SEGMENT: &str = "product";
const ROMAN_PREFIX: &str = "ROMAN";
const PROGRAM_DIGITS: usize = 6;
const WINDOWS_DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
const URI_MISSIONS: [(&str, MastMissionId); 3] = [
    ("JWST", MastMissionId::Jwst),
    ("HST", MastMissionId::Hst),
    ("TESS", MastMissionId::Tess),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MastUri {
    Product {
        mission: MastMissionId,
        filename: String,
    },
    Hlsp {
        collection: String,
        filename: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub uri: String,
    pub program: Option<String>,
    pub folders: [String; 2],
    pub filename: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Storage {
    pub root: PathBuf,
    pub quota_bytes: u64,
}

fn uri_refused(message: impl Into<String>) -> MastCmdError {
    MastCmdError::new(MastErrorCode::UriRefused, message)
}

fn uri_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/')
}

fn uri_mission_name(mission: MastMissionId) -> Option<&'static str> {
    URI_MISSIONS
        .iter()
        .find(|(_, known)| *known == mission)
        .map(|(name, _)| *name)
}

fn is_roman(name: &str) -> bool {
    name.to_ascii_uppercase().starts_with(ROMAN_PREFIX)
}

fn plain_file(name: &str) -> Result<(), MastCmdError> {
    validate_filename(name).map_err(|_| bad_input("uri", "the file name is not a plain file name"))
}

pub fn validate_uri(text: &str) -> Result<MastUri, MastCmdError> {
    if text.len() > URI_MAX {
        return Err(bad_input("uri", "must be at most 512 characters"));
    }
    let Some(rest) = text.strip_prefix(MAST_SCHEME) else {
        return Err(uri_refused("only mast: product URIs are downloaded"));
    };
    if !rest.chars().all(uri_char) {
        return Err(bad_input("uri", "use letters, digits and . _ - / only"));
    }
    let segments: Vec<&str> = rest.split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || segment.starts_with('.'))
    {
        return Err(bad_input(
            "uri",
            "must not hold empty segments or segments starting with a dot",
        ));
    }
    let first = segments[0];
    if is_roman(first) {
        return Err(uri_refused("Roman downloads are not enabled yet"));
    }
    if first == HLSP_FOLDER {
        let [_, collection, .., filename] = segments.as_slice() else {
            return Err(bad_input(
                "uri",
                "an HLSP URI names a collection and a file",
            ));
        };
        let collection = path_component(collection)
            .map_err(|_| bad_input("uri", "the HLSP collection is not a plain folder name"))?;
        plain_file(filename)?;
        return Ok(MastUri::Hlsp {
            collection: collection.to_owned(),
            filename: (*filename).to_owned(),
        });
    }
    let Some(mission) = URI_MISSIONS
        .iter()
        .find(|(name, _)| *name == first)
        .map(|(_, mission)| *mission)
    else {
        return Err(uri_refused(format!(
            "mast:{first}/ files are not downloaded"
        )));
    };
    match segments.as_slice() {
        [_, kind, filename] if *kind == PRODUCT_SEGMENT => {
            plain_file(filename)?;
            Ok(MastUri::Product {
                mission,
                filename: (*filename).to_owned(),
            })
        }
        [_, kind, ..] if *kind == PRODUCT_SEGMENT => {
            Err(bad_input("uri", "a product URI names one file"))
        }
        _ => Err(uri_refused(format!(
            "only mast:{first}/product/ files are downloaded"
        ))),
    }
}

pub fn product_uri(mission: MastMissionId, filename: &str) -> Result<String, MastCmdError> {
    validate_filename(filename)?;
    if is_roman(mission.slug()) {
        return Err(uri_refused("Roman downloads are not enabled yet"));
    }
    let name = uri_mission_name(mission).ok_or_else(|| {
        uri_refused(format!(
            "{} products are not downloaded yet",
            mission.slug()
        ))
    })?;
    Ok(format!("{MAST_SCHEME}{name}/{PRODUCT_SEGMENT}/{filename}"))
}

pub fn download_url(base: &str, uri: &str) -> Result<Url, MastCmdError> {
    let mut url = Url::parse(base)
        .and_then(|base| base.join(DOWNLOAD_FILE_PATH))
        .map_err(|_| internal())?;
    url.query_pairs_mut().append_pair("uri", uri);
    Ok(url)
}

pub fn program_folder(program: Option<&str>) -> Result<Option<String>, MastCmdError> {
    let Some(text) = program.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let digits = text.len() <= PROGRAM_DIGITS && text.bytes().all(|b| b.is_ascii_digit());
    match text.parse::<u32>().ok().filter(|_| digits) {
        Some(number) => Ok(Some(number.to_string())),
        None => Err(bad_input("program", "use 1 to 6 digits")),
    }
}

pub fn plan(mission: MastMissionId, input: &MastDownloadInput) -> Result<Target, MastCmdError> {
    validate_dataset_id(&input.dataset_id)?;
    validate_filename(&input.filename)?;
    let program = program_folder(input.program.as_deref())?;
    let uri = match &input.uri {
        Some(text) => text.clone(),
        None => product_uri(mission, &input.filename)?,
    };
    let (folders, program, filename) = match validate_uri(&uri)? {
        MastUri::Hlsp {
            collection,
            filename,
        } => ([HLSP_FOLDER.to_owned(), collection], None, filename),
        MastUri::Product {
            mission: named,
            filename,
        } => {
            if named != mission {
                return Err(bad_input("uri", "names another mission than the request"));
            }
            let name = uri_mission_name(named).ok_or_else(internal)?;
            let folder = program.clone().unwrap_or_else(|| MISC_PROGRAM.to_owned());
            ([name.to_owned(), folder], program, filename)
        }
    };
    if filename != input.filename {
        return Err(bad_input("uri", "does not end with the item's file name"));
    }
    Ok(Target {
        uri,
        program,
        folders,
        filename,
    })
}

fn outside_root() -> MastCmdError {
    bad_input("filename", "resolves outside the MAST download folder")
}

fn reserved_by_windows(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let numbered = stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(stem.as_bytes()[3], b'1'..=b'9');
    WINDOWS_DEVICES.contains(&stem.as_str()) || numbered || name.ends_with('.')
}

pub fn destination(root: &Path, target: &Target) -> Result<PathBuf, MastCmdError> {
    let root = std::path::absolute(root).map_err(|_| outside_root())?;
    let names = [
        target.folders[0].as_str(),
        target.folders[1].as_str(),
        target.filename.as_str(),
    ];
    if names.iter().any(|name| reserved_by_windows(name)) {
        return Err(bad_input("filename", "is a name Windows reserves"));
    }
    let joined = names
        .iter()
        .fold(root.clone(), |path, name| path.join(name));
    let resolved = std::path::absolute(&joined).map_err(|_| outside_root())?;
    let below = resolved.strip_prefix(&root).map_err(|_| outside_root())?;
    let parts: Vec<Component> = below.components().collect();
    let exact = parts.len() == names.len()
        && parts.iter().zip(names).all(
            |(part, name)| matches!(part, Component::Normal(found) if *found == OsStr::new(name)),
        );
    if exact {
        Ok(resolved)
    } else {
        Err(outside_root())
    }
}

pub fn check_root(root: &Path, reserved: &[PathBuf]) -> Result<(), MastCmdError> {
    if !root.is_absolute() {
        return Err(MastCmdError::new(
            MastErrorCode::Cache,
            "the MAST download folder is not an absolute path",
        ));
    }
    match reserved
        .iter()
        .find(|dir| crate::infra::config::is_same_or_inside(root, dir))
    {
        Some(dir) => Err(MastCmdError::new(
            MastErrorCode::Cache,
            format!(
                "the MAST download folder {} is inside {}, which the app clears; choose another Data folder in Settings",
                root.display(),
                dir.display()
            ),
        )),
        None => Ok(()),
    }
}

fn operation(mission: MastMissionId) -> Operation {
    Operation::new(Some(mission), OpKind::Download, DOWNLOAD_FILE_PATH)
}

fn context(mission: MastMissionId) -> MastErrorContext {
    MastErrorContext {
        operation: Some(DOWNLOAD_FILE_PATH.to_owned()),
        mission: Some(mission),
        ..Default::default()
    }
}

fn coded(
    code: MastErrorCode,
    message: impl Into<String>,
    context: MastErrorContext,
) -> MastCmdError {
    MastCmdError::new(code, message).with_context(context)
}

pub fn failure(
    error: DownloadError,
    mission: MastMissionId,
) -> (MastCmdError, Option<MastDownloadAction>) {
    use MastDownloadAction::{Redownload, Replace, Resume, Retry};
    let here = context(mission);
    match error {
        DownloadError::Cancelled => (cancelled(), Some(Resume)),
        DownloadError::Transport(text) | DownloadError::RedirectRefused(text) => (
            coded(MastErrorCode::Network, transport_message(&text), here),
            Some(Resume),
        ),
        DownloadError::Http { status } => {
            let answered = MastError::Status {
                status,
                url: String::new(),
                body: String::new(),
                detail: None,
                content_type: None,
                retry_after_secs: None,
            };
            let transient = status >= 500 || status == 408 || status == 429;
            (
                classify(&answered, &operation(mission), 1),
                transient.then_some(Retry),
            )
        }
        DownloadError::SizeMismatch { expected, actual } => (
            coded(
                MastErrorCode::SizeMismatch,
                format!("MAST listed {expected} bytes, the server sent {actual}"),
                MastErrorContext {
                    bytes: Some(actual),
                    ..here
                },
            ),
            Some(Retry),
        ),
        DownloadError::ChecksumMismatch {
            expected_hex,
            actual_hex,
        } => (
            coded(
                MastErrorCode::SizeMismatch,
                format!("sha256 {actual_hex} instead of {expected_hex}"),
                here,
            ),
            Some(Retry),
        ),
        DownloadError::TooLarge { cap } => (
            coded(
                MastErrorCode::TooLarge,
                format!("larger than the {cap}-byte limit for one file"),
                MastErrorContext {
                    bytes: Some(cap),
                    ..here
                },
            ),
            None,
        ),
        DownloadError::RangeNotSatisfiable => (
            coded(
                MastErrorCode::Server,
                "MAST refused the byte range (HTTP 416)",
                MastErrorContext {
                    status: Some(416),
                    ..here
                },
            ),
            Some(Retry),
        ),
        DownloadError::Busy => (
            coded(
                MastErrorCode::Busy,
                "this file is already downloading",
                here,
            ),
            None,
        ),
        DownloadError::Exists => (
            bad_input(
                "filename",
                "another file, not downloaded by AstroBurst, is already at this place",
            ),
            Some(Replace),
        ),
        DownloadError::Stale {
            local_etag,
            remote_etag,
        } => (
            coded(
                MastErrorCode::Stale,
                "MAST has a newer version of this file",
                MastErrorContext {
                    detail: Some(format!(
                        "local {}, MAST {}",
                        local_etag.as_deref().unwrap_or("none"),
                        remote_etag.as_deref().unwrap_or("none")
                    )),
                    ..here
                },
            ),
            Some(Redownload),
        ),
        DownloadError::Io(error) => (
            MastCmdError::new(MastErrorCode::Cache, io_cause(&error)),
            Some(Resume),
        ),
    }
}

pub fn transfer_client() -> reqwest::Result<Client> {
    build_client(&HttpProfile {
        redirect: RedirectRule::https(&TRANSFER_REDIRECT_HOSTS),
        ..HttpProfile::transfer()
    })
}

pub fn storage(settings: &MastSettings, reserved: &[PathBuf]) -> Result<Storage, MastCmdError> {
    let data = paths::resolved_data_root().map_err(|error| {
        MastCmdError::new(
            MastErrorCode::Cache,
            format!("the Data folder setting could not be read: {error:#}"),
        )
    })?;
    let quota_gb = crate::infra::config::load_config()
        .ok()
        .and_then(|config| config.mast_quota_gb);
    storage_in(&data, settings, reserved, quota_gb)
}

pub fn storage_in(
    data: &DataRoot,
    settings: &MastSettings,
    reserved: &[PathBuf],
    quota_gb: Option<f64>,
) -> Result<Storage, MastCmdError> {
    let root = data
        .require_present()
        .map_err(|message| MastCmdError::new(MastErrorCode::Cache, message))?
        .join(settings.cache_dir_name());
    check_root(&root, reserved)?;
    Ok(Storage {
        root,
        quota_bytes: paths::mast_quota_bytes(quota_gb),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::{MastErrorCode, OpensAs};
    use crate::infra::mast::client::DEFAULT_BASE_URL;

    const CAL: &str = "jw02739001001_02105_00001_nrca1_cal.fits";
    const CAL_DATASET: &str = "jw02739001001_02105_00001_nrca1";
    const CEERS: &str = "mast:HLSP/ceers/nircam1/ceers_nircam1_f200w_v0.5_i2d.fits";
    const CEERS_FILE: &str = "ceers_nircam1_f200w_v0.5_i2d.fits";

    fn input(dataset: &str, filename: &str, program: Option<&str>) -> MastDownloadInput {
        MastDownloadInput {
            dataset_id: dataset.to_owned(),
            filename: filename.to_owned(),
            program: program.map(str::to_owned),
            size_bytes: 117_573_120,
            uri: None,
            opens_as: Some(OpensAs::Image),
            replace: false,
        }
    }

    fn with_uri(mut item: MastDownloadInput, uri: &str) -> MastDownloadInput {
        item.uri = Some(uri.to_owned());
        item
    }

    fn code_of(result: Result<impl std::fmt::Debug, MastCmdError>) -> MastErrorCode {
        result.unwrap_err().code
    }

    fn product(mission: MastMissionId, filename: &str) -> MastUri {
        MastUri::Product {
            mission,
            filename: filename.to_owned(),
        }
    }

    #[test]
    fn uris_follow_the_allowlist() {
        let tess = "tess2018206045859-s0001-0000000025155310-0120-s_lc.fits";
        let accepted = [
            (
                format!("mast:JWST/product/{CAL}"),
                product(MastMissionId::Jwst, CAL),
            ),
            (
                "mast:HST/product/j9irw3fwq_flc.fits".to_owned(),
                product(MastMissionId::Hst, "j9irw3fwq_flc.fits"),
            ),
            (
                format!("mast:TESS/product/{tess}"),
                product(MastMissionId::Tess, tess),
            ),
            (
                CEERS.to_owned(),
                MastUri::Hlsp {
                    collection: "ceers".to_owned(),
                    filename: CEERS_FILE.to_owned(),
                },
            ),
        ];
        for (text, expected) in accepted {
            assert_eq!(validate_uri(&text).unwrap(), expected, "{text}");
        }
        let deep = format!(
            "mast:HLSP/c/{}/{}/{}.fits",
            "a".repeat(200),
            "b".repeat(200),
            "f".repeat(93)
        );
        assert_eq!(deep.len(), URI_MAX);
        assert!(validate_uri(&deep).is_ok());

        let too_long = format!(
            "mast:HLSP/c/{}/{}/{}.fits",
            "a".repeat(200),
            "b".repeat(200),
            "f".repeat(94)
        );
        assert_eq!(too_long.len(), URI_MAX + 1);
        for malformed in [
            too_long.as_str(),
            "mast:JWST/product/../x",
            "mast:HLSP/../x",
            "mast:HLSP/ceers/../../x.fits",
            "mast:JWST//product/x.fits",
            "mast:JWST/product/x.fits/",
            "mast:JWST/product/.hidden.fits",
            "mast:JWST/product/a b.fits",
            "mast:JWST/product/x%2Efits",
            "mast:JWST/product/a\\b.fits",
            "mast:JWST/product/caf\u{e9}.fits",
            "mast:JWST/product/a/b.fits",
            "mast:HLSP/ceers",
            "mast:",
        ] {
            let error = validate_uri(malformed).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{malformed}");
            assert!(error.message.starts_with("uri: "), "{}", error.message);
        }
        for refused in [
            "mast:ROMAN/product/r0001_cal.asdf",
            "mast:Roman/product/r0001_cal.asdf",
            "mast:ROMAN_CGI/product/x.fits",
            "http://mast.stsci.edu/x.fits",
            "https://mast.stsci.edu/api/v0.1/Download/file?uri=mast:JWST/product/x.fits",
            "mast:IUE/url/missions/iue/data/lwp00501.mxhi.gz",
            "mast:JWST/url/x.fits",
            "mast:CLASSY/product/x.fits",
        ] {
            assert_eq!(
                code_of(validate_uri(refused)),
                MastErrorCode::UriRefused,
                "{refused}"
            );
        }
    }

    #[test]
    fn product_uris_and_download_urls() {
        assert_eq!(
            product_uri(MastMissionId::Jwst, CAL).unwrap(),
            format!("mast:JWST/product/{CAL}")
        );
        assert_eq!(
            product_uri(MastMissionId::Hst, "j9irw3fwq_flc.fits").unwrap(),
            "mast:HST/product/j9irw3fwq_flc.fits"
        );
        for mission in [
            MastMissionId::Roman,
            MastMissionId::RomanCgi,
            MastMissionId::RomanSpectra,
        ] {
            assert_eq!(
                code_of(product_uri(mission, "r_cal.asdf")),
                MastErrorCode::UriRefused
            );
        }
        assert_eq!(
            code_of(product_uri(MastMissionId::Iue, "x.fits")),
            MastErrorCode::UriRefused
        );
        assert_eq!(
            code_of(product_uri(MastMissionId::Jwst, "../x")),
            MastErrorCode::BadInput
        );

        let canned =
            download_url("http://127.0.0.1:43123", "mast:JWST/product/a_cal.fits").unwrap();
        assert_eq!(canned.path(), "/api/v0.1/Download/file");
        let pairs: Vec<(String, String)> = canned
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            pairs,
            [("uri".to_owned(), "mast:JWST/product/a_cal.fits".to_owned())]
        );
        assert_eq!(
            canned.as_str(),
            "http://127.0.0.1:43123/api/v0.1/Download/file?uri=mast%3AJWST%2Fproduct%2Fa_cal.fits"
        );
        let live = download_url(DEFAULT_BASE_URL, CEERS).unwrap();
        assert!(
            live.as_str().starts_with(
                "https://mast.stsci.edu/api/v0.1/Download/file?uri=mast%3AHLSP%2Fceers%2F"
            ),
            "{live}"
        );
        assert_eq!(
            code_of(download_url("not a base", CEERS)),
            MastErrorCode::Decode
        );
    }

    #[test]
    fn destinations_derive_the_program_folder_before_any_io() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data").join("mast");

        let jwst = plan(MastMissionId::Jwst, &input(CAL_DATASET, CAL, Some("2739"))).unwrap();
        assert_eq!(jwst.uri, format!("mast:JWST/product/{CAL}"));
        assert_eq!(jwst.program.as_deref(), Some("2739"));
        assert_eq!(
            destination(&root, &jwst).unwrap(),
            root.join("JWST").join("2739").join(CAL)
        );

        let padded = plan(
            MastMissionId::Jwst,
            &input(CAL_DATASET, CAL, Some(" 02739 ")),
        )
        .unwrap();
        assert_eq!(padded.folders, ["JWST".to_owned(), "2739".to_owned()]);

        let hst = plan(
            MastMissionId::Hst,
            &input("J9IRW3FWQ", "j9irw3fwq_flc.fits", Some("10533")),
        )
        .unwrap();
        assert_eq!(
            destination(&root, &hst).unwrap(),
            root.join("HST").join("10533").join("j9irw3fwq_flc.fits")
        );

        for absent in [None, Some(""), Some("   ")] {
            let misc = plan(
                MastMissionId::Hst,
                &input("J9IRW3FWQ", "j9irw3fwq_flc.fits", absent),
            )
            .unwrap();
            assert_eq!(misc.program, None);
            assert_eq!(
                destination(&root, &misc).unwrap(),
                root.join("HST")
                    .join(MISC_PROGRAM)
                    .join("j9irw3fwq_flc.fits")
            );
        }

        for bad in [
            "..\\..\\x",
            "jw/../..",
            "12345678",
            "1234567",
            "27a9",
            "-1",
            "1.5",
            "2739/1",
        ] {
            let error = plan(MastMissionId::Jwst, &input(CAL_DATASET, CAL, Some(bad))).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{bad}");
            assert!(
                error.message.starts_with("program: "),
                "{bad}: {}",
                error.message
            );
        }
        assert_eq!(
            program_folder(Some("999999")).unwrap().as_deref(),
            Some("999999")
        );
        assert_eq!(
            program_folder(Some("000000")).unwrap().as_deref(),
            Some("0")
        );
        assert_eq!(program_folder(None).unwrap(), None);

        let hlsp = plan(
            MastMissionId::Jwst,
            &with_uri(input("ceers_nircam1", CEERS_FILE, Some("1345")), CEERS),
        )
        .unwrap();
        assert_eq!(hlsp.uri, CEERS);
        assert_eq!(hlsp.program, None);
        assert_eq!(
            destination(&root, &hlsp).unwrap(),
            root.join(HLSP_FOLDER).join("ceers").join(CEERS_FILE)
        );

        let same = plan(
            MastMissionId::Jwst,
            &with_uri(
                input(CAL_DATASET, CAL, Some("2739")),
                &format!("mast:JWST/product/{CAL}"),
            ),
        )
        .unwrap();
        assert_eq!(same, jwst);

        let other_mission = plan(
            MastMissionId::Hst,
            &with_uri(
                input(CAL_DATASET, CAL, None),
                &format!("mast:JWST/product/{CAL}"),
            ),
        )
        .unwrap_err();
        assert_eq!(
            (
                other_mission.code,
                other_mission.message.starts_with("uri: ")
            ),
            (MastErrorCode::BadInput, true)
        );
        let other_file = plan(
            MastMissionId::Jwst,
            &with_uri(input("ceers_nircam1", "other_i2d.fits", None), CEERS),
        )
        .unwrap_err();
        assert_eq!(
            (other_file.code, other_file.message.starts_with("uri: ")),
            (MastErrorCode::BadInput, true)
        );

        assert_eq!(
            code_of(plan(
                MastMissionId::Roman,
                &input("r0001", "r0001_cal.asdf", None)
            )),
            MastErrorCode::UriRefused
        );
        assert_eq!(
            code_of(plan(
                MastMissionId::Jwst,
                &with_uri(
                    input("r0001", "r0001_cal.asdf", None),
                    "mast:ROMAN/product/r0001_cal.asdf"
                )
            )),
            MastErrorCode::UriRefused
        );
        let bad_dataset = plan(MastMissionId::Jwst, &input("../x", CAL, None)).unwrap_err();
        assert!(
            bad_dataset.message.starts_with("datasetId: "),
            "{}",
            bad_dataset.message
        );
        let bad_file =
            plan(MastMissionId::Jwst, &input(CAL_DATASET, "../x.fits", None)).unwrap_err();
        assert!(
            bad_file.message.starts_with("filename: "),
            "{}",
            bad_file.message
        );

        assert!(!dir.path().join("data").exists(), "planning wrote to disk");
    }

    #[test]
    fn the_final_path_check_refuses_names_that_windows_reserves_or_rewrites() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("mast");
        for name in [
            "NUL",
            "CON",
            "con.fits",
            "Aux.x1d.fits",
            "COM1",
            "lpt9.fits",
            "x.",
            "a_cal.fits.",
        ] {
            let target =
                plan(MastMissionId::Jwst, &input(CAL_DATASET, name, Some("2739"))).unwrap();
            let error = destination(&root, &target).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{name}");
            assert!(
                error.message.starts_with("filename: "),
                "{name}: {}",
                error.message
            );
        }
        assert!(!root.exists());
        for name in [
            "COM0.fits",
            "console_cal.fits",
            "nul_x1d.fits",
            "LPT10",
            "auxiliary_cal.fits",
        ] {
            let target =
                plan(MastMissionId::Jwst, &input(CAL_DATASET, name, Some("2739"))).unwrap();
            assert!(destination(&root, &target).is_ok(), "{name}");
        }
    }

    #[test]
    fn engine_errors_map_to_mast_codes_and_recovery_actions() {
        use MastDownloadAction::{Redownload, Replace, Resume, Retry};
        let jwst = MastMissionId::Jwst;

        let (error, action) = failure(DownloadError::Cancelled, jwst);
        assert_eq!(
            (error.code, action),
            (MastErrorCode::Cancelled, Some(Resume))
        );

        let (error, action) = failure(
            DownloadError::Transport("error sending request for url (http://127.0.0.1:9/api/v0.1/Download/file?uri=x): connection refused".into()),
            jwst,
        );
        assert_eq!((error.code, action), (MastErrorCode::Network, Some(Resume)));
        assert!(
            !error.message.contains("://") && !error.message.contains('?'),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("connection refused"),
            "{}",
            error.message
        );
        assert_eq!(error.context.operation.as_deref(), Some(DOWNLOAD_FILE_PATH));

        let (error, action) = failure(DownloadError::Http { status: 503 }, jwst);
        assert_eq!((error.code, action), (MastErrorCode::Server, Some(Retry)));
        assert_eq!(error.context.status, Some(503));
        assert_eq!(error.context.operation.as_deref(), Some(DOWNLOAD_FILE_PATH));
        let (error, action) = failure(DownloadError::Http { status: 404 }, jwst);
        assert_eq!(
            (error.code, error.context.status, action),
            (MastErrorCode::Server, Some(404), None)
        );
        let (error, action) = failure(DownloadError::Http { status: 401 }, MastMissionId::Hst);
        assert_eq!((error.code, action), (MastErrorCode::Unauthorized, None));
        let (error, action) = failure(DownloadError::Http { status: 429 }, jwst);
        assert_eq!(
            (error.code, action),
            (MastErrorCode::RateLimited, Some(Retry))
        );
        for status in [400, 403, 410, 422] {
            assert_eq!(
                failure(DownloadError::Http { status }, jwst).1,
                None,
                "HTTP {status}"
            );
        }
        for status in [408, 500, 502, 504] {
            assert_eq!(
                failure(DownloadError::Http { status }, jwst).1,
                Some(Retry),
                "HTTP {status}"
            );
        }

        let (error, action) = failure(
            DownloadError::SizeMismatch {
                expected: 3_145_728,
                actual: 3_145_729,
            },
            jwst,
        );
        assert_eq!(
            (error.code, action),
            (MastErrorCode::SizeMismatch, Some(Retry))
        );
        assert!(
            error.message.contains("3145728") && error.message.contains("3145729"),
            "{}",
            error.message
        );
        assert_eq!(error.context.bytes, Some(3_145_729));

        let (error, action) = failure(
            DownloadError::ChecksumMismatch {
                expected_hex: "aa".into(),
                actual_hex: "bb".into(),
            },
            jwst,
        );
        assert_eq!(
            (error.code, action),
            (MastErrorCode::SizeMismatch, Some(Retry))
        );
        assert!(
            error.message.contains("aa") && error.message.contains("bb"),
            "{}",
            error.message
        );

        let (error, action) = failure(
            DownloadError::TooLarge {
                cap: MAX_PRODUCT_BYTES,
            },
            jwst,
        );
        assert_eq!(
            (error.code, action, error.context.bytes),
            (MastErrorCode::TooLarge, None, Some(MAX_PRODUCT_BYTES))
        );

        let (error, action) = failure(DownloadError::RangeNotSatisfiable, jwst);
        assert_eq!(
            (error.code, action, error.context.status),
            (MastErrorCode::Server, Some(Retry), Some(416))
        );

        let (error, action) = failure(DownloadError::Busy, jwst);
        assert_eq!((error.code, action), (MastErrorCode::Busy, None));

        let (error, action) = failure(DownloadError::Exists, jwst);
        assert_eq!(
            (error.code, action),
            (MastErrorCode::BadInput, Some(Replace))
        );
        assert!(error.message.starts_with("filename: "), "{}", error.message);

        let (error, action) = failure(
            DownloadError::Stale {
                local_etag: Some("\"ab-v0\"".into()),
                remote_etag: Some("\"ab-v1\"".into()),
            },
            jwst,
        );
        assert_eq!(
            (error.code, action),
            (MastErrorCode::Stale, Some(Redownload))
        );
        assert!(error
            .context
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("ab-v0") && d.contains("ab-v1")));

        let (error, action) = failure(DownloadError::Io(std::io::Error::other("disk full")), jwst);
        assert_eq!((error.code, action), (MastErrorCode::Cache, Some(Resume)));
        assert_eq!(error.message, "disk full");
    }

    #[test]
    fn a_download_folder_inside_the_app_folders_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = dir
            .path()
            .join("AppData")
            .join("Roaming")
            .join("com.astroburst.app");
        let app_local = dir
            .path()
            .join("AppData")
            .join("Local")
            .join("com.astroburst.app");
        let reserved = [app_data.clone(), app_local.clone()];
        assert!(check_root(&dir.path().join("data").join("mast"), &reserved).is_ok());
        assert!(check_root(
            &dir.path()
                .join("AppData")
                .join("Local")
                .join("astroburst")
                .join("mast"),
            &reserved
        )
        .is_ok());
        for inside in [
            app_data.join("mast"),
            app_local.join("output").join("mast"),
            app_local.clone(),
        ] {
            let error = check_root(&inside, &reserved).unwrap_err();
            assert_eq!(error.code, MastErrorCode::Cache, "{}", inside.display());
        }
        assert_eq!(
            code_of(check_root(Path::new("relative/mast"), &reserved)),
            MastErrorCode::Cache
        );
    }

    #[test]
    fn a_missing_chosen_data_folder_is_refused_and_a_default_one_is_used() {
        use crate::infra::mast::settings::ENV_MAST_URL;
        use crate::infra::net::paths::DataRootSource;
        let dir = tempfile::tempdir().unwrap();
        let unplugged = dir.path().join("E-drive").join("astro");
        let reserved = [dir
            .path()
            .join("AppData")
            .join("Local")
            .join("com.astroburst.app")];
        let live = MastSettings::default();
        let canned = MastSettings::from_lookup(|name| {
            (name == ENV_MAST_URL).then(|| "http://127.0.0.1:43123".to_owned())
        });
        for source in [DataRootSource::Env, DataRootSource::Configured] {
            let chosen = DataRoot {
                path: unplugged.clone(),
                source,
            };
            let error = storage_in(&chosen, &live, &reserved, None).unwrap_err();
            assert_eq!(error.code, MastErrorCode::Cache, "{source:?}");
            assert!(
                error.message.contains("does not exist"),
                "{}",
                error.message
            );
        }
        assert!(!unplugged.exists(), "the missing folder was recreated");
        for source in [DataRootSource::Platform, DataRootSource::TestData] {
            let default = DataRoot {
                path: unplugged.clone(),
                source,
            };
            let storage = storage_in(&default, &live, &reserved, Some(0.5)).unwrap();
            assert_eq!(
                storage,
                Storage {
                    root: unplugged.join("mast"),
                    quota_bytes: 500_000_000
                }
            );
        }
        let present = DataRoot {
            path: dir.path().to_path_buf(),
            source: DataRootSource::Configured,
        };
        let storage = storage_in(&present, &canned, &reserved, None).unwrap();
        assert_eq!(
            storage,
            Storage {
                root: dir.path().join("mast-test"),
                quota_bytes: 20_000_000_000
            }
        );
        let inside = DataRoot {
            path: reserved[0].clone(),
            source: DataRootSource::Platform,
        };
        assert_eq!(
            code_of(storage_in(&inside, &live, &reserved, None)),
            MastErrorCode::Cache
        );
    }
}
