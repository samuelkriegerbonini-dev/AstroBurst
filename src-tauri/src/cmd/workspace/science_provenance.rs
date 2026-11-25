use std::fs::{metadata, File};
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use anyhow::{ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::core::cube::companions::{open_cube_set_fresh, CubeSet};
use crate::core::history::stamp::utc_timestamp;
use crate::types::header::HduHeader;
use crate::types::image_ref::{BandPlaneKey, ImageRef, PlaneSelector};
use crate::infra::fits::dispatcher::resolve_single_image_fresh;
use crate::infra::fits::gzip::SharedTempDir;

const MANIFEST_NAME: &str = "provenance.json";
pub(crate) const SCIENTIFIC_INTEGRITY: &str = "sha256_before_open_and_before_publication";
pub(crate) const SCIENTIFIC_INTEGRITY_LIMIT: &str = "Hashes verify acquisition and publication checkpoints, not an immutable snapshot; a transient rewrite that restores the original bytes between checkpoints cannot be detected";

pub(crate) const HEADER_ABOP: &str = "ABOP";
pub(crate) const HEADER_ABINPUT: &str = "ABINPUT";
pub(crate) const HEADER_ABMASK: &str = "ABMASK";
const HEADER_ABSRC: &str = "ABSRC";
const HEADER_ABSRCHDU: &str = "ABSRCHDU";
const HEADER_ABSRCPLN: &str = "ABSRCPLN";
const HEADER_ABSRCARR: &str = "ABSRCARR";
const HEADER_ABSRCSZ: &str = "ABSRCSZ";
const HEADER_ABSRCMT: &str = "ABSRCMT";
const HEADER_ABSRCSHA: &str = "ABSRCSHA";
const HEADER_ABINPUTS: &str = "ABINPUTS";
const HEADER_ABPARAMS: &str = "ABPARAMS";
const HEADER_ABVER: &str = "ABVER";
const STALE_PRODUCT_KEYS: [&str; 13] = [
    HEADER_ABOP, HEADER_ABINPUT, HEADER_ABMASK, HEADER_ABSRC, HEADER_ABSRCHDU, HEADER_ABSRCPLN, HEADER_ABSRCARR,
    HEADER_ABSRCSZ, HEADER_ABSRCMT, HEADER_ABSRCSHA, HEADER_ABINPUTS, HEADER_ABPARAMS, HEADER_ABVER,
];
const STALE_PRODUCT_PREFIX: &str = "ABSTF";
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const PNG_IHDR_END: usize = 8 + 4 + 4 + 13 + 4;
const PNG_SOFTWARE: &str = "Software";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct InputIdentity {
    pub role: String,
    #[serde(rename = "ref")]
    pub input_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hdu: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plane: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl InputIdentity {
    pub fn of(role: &str, input_ref: &str) -> Self {
        let parsed = ImageRef::parse(input_ref);
        let array = match &parsed.plane {
            PlaneSelector::Array(name) => Some(name.clone()),
            _ => None,
        };
        let (file, hdu, plane) = match BandPlaneKey::parse(input_ref) {
            Some(band) => (band.file, Some(band.hdu), Some(band.plane)),
            None => {
                let hdu = match parsed.plane {
                    PlaneSelector::Hdu(index) => Some(index),
                    _ => None,
                };
                (parsed.path, hdu, None)
            }
        };
        let stamp = metadata(&file).ok().filter(|meta| meta.is_file());
        Self {
            role: role.to_string(),
            input_ref: input_ref.to_string(),
            hdu,
            plane,
            array,
            size_bytes: stamp.as_ref().map(|meta| meta.len()),
            modified_at_utc: stamp.and_then(|meta| meta.modified().ok()).map(utc_timestamp),
            sha256: None,
        }
    }
}

struct ProductFact {
    key: String,
    fits: String,
    png: String,
    text: bool,
}

pub(crate) struct ProductRecord {
    facts: Vec<ProductFact>,
}

fn fits_ascii(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for c in value.chars() {
        if (' '..='~').contains(&c) && c != '%' {
            encoded.push(c);
            continue;
        }
        let mut buf = [0u8; 4];
        for b in c.encode_utf8(&mut buf).bytes() {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    encoded
}

fn json_ascii(value: &Value) -> String {
    let mut out = String::new();
    for c in value.to_string().chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units).iter() {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

fn fits_real(value: f64) -> String {
    let text = format!("{value:E}");
    match text.split_once('E') {
        Some((mantissa, exponent)) if !mantissa.contains('.') => format!("{mantissa}.0E{exponent}"),
        _ => text,
    }
}

impl ProductRecord {
    pub fn new(operation: &str, inputs: &[InputIdentity], parameters: &Value) -> Self {
        let mut record = Self { facts: Vec::new() }.text(HEADER_ABOP, operation);
        if let Some(primary) = inputs.first() {
            record = record.text(HEADER_ABSRC, &primary.input_ref);
            if let Some(hdu) = primary.hdu {
                record = record.integer(HEADER_ABSRCHDU, hdu as u64);
            }
            if let Some(plane) = primary.plane {
                record = record.integer(HEADER_ABSRCPLN, plane as u64);
            }
            if let Some(array) = &primary.array {
                record = record.text(HEADER_ABSRCARR, array);
            }
            if let Some(size) = primary.size_bytes {
                record = record.integer(HEADER_ABSRCSZ, size);
            }
            if let Some(mtime) = &primary.modified_at_utc {
                record = record.text(HEADER_ABSRCMT, mtime);
            }
            if let Some(sha256) = &primary.sha256 {
                record = record.text(HEADER_ABSRCSHA, sha256);
            }
        }
        if inputs.len() > 1 {
            record = record.json(HEADER_ABINPUTS, &serde_json::to_value(inputs).unwrap_or(Value::Null));
        }
        if !parameters.is_null() {
            record = record.json(HEADER_ABPARAMS, parameters);
        }
        record.text(HEADER_ABVER, env!("CARGO_PKG_VERSION"))
    }

    pub fn text(mut self, key: &str, value: &str) -> Self {
        self.facts.push(ProductFact { key: key.to_string(), fits: fits_ascii(value), png: value.to_string(), text: true });
        self
    }

    pub fn json(mut self, key: &str, value: &Value) -> Self {
        self.facts.push(ProductFact { key: key.to_string(), fits: json_ascii(value), png: value.to_string(), text: true });
        self
    }

    pub fn integer(mut self, key: &str, value: u64) -> Self {
        self.facts.push(ProductFact { key: key.to_string(), fits: value.to_string(), png: value.to_string(), text: false });
        self
    }

    pub fn real(mut self, key: &str, value: f64) -> Self {
        if value.is_finite() {
            self.facts.push(ProductFact { key: key.to_string(), fits: fits_real(value), png: value.to_string(), text: false });
        }
        self
    }

    pub fn stamp(&self, header: &mut HduHeader) {
        let stale: Vec<String> = header
            .cards
            .iter()
            .map(|(key, _)| key.trim().to_string())
            .filter(|key| STALE_PRODUCT_KEYS.contains(&key.as_str()) || key.starts_with(STALE_PRODUCT_PREFIX))
            .collect();
        for key in stale {
            header.remove(&key);
        }
        for fact in &self.facts {
            header.set(&fact.key, fact.fits.clone());
            if let Some(keys) = header.string_keys.as_mut() {
                if fact.text {
                    keys.insert(fact.key.clone());
                } else {
                    keys.remove(&fact.key);
                }
            }
        }
    }

    pub fn write_png_text(&self, path: &str) -> Result<()> {
        let bytes = std::fs::read(path).with_context(|| format!("Cannot read the PNG {path} to record its provenance"))?;
        ensure!(bytes.len() > PNG_IHDR_END && bytes.starts_with(PNG_SIGNATURE) && &bytes[12..16] == b"IHDR", "{path} is not a PNG file");
        let software = format!("AstroBurst {}", env!("CARGO_PKG_VERSION"));
        let entries = std::iter::once((PNG_SOFTWARE, software.as_str())).chain(self.facts.iter().map(|fact| (fact.key.as_str(), fact.png.as_str())));
        let mut out = Vec::with_capacity(bytes.len() + 1024);
        out.extend_from_slice(&bytes[..PNG_IHDR_END]);
        for (key, text) in entries {
            let mut data = Vec::with_capacity(key.len() + text.len() + 5);
            data.extend_from_slice(key.as_bytes());
            data.extend_from_slice(&[0, 0, 0, 0, 0]);
            data.extend_from_slice(text.as_bytes());
            let mut crc = flate2::Crc::new();
            crc.update(b"iTXt");
            crc.update(&data);
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(b"iTXt");
            out.extend_from_slice(&data);
            out.extend_from_slice(&crc.sum().to_be_bytes());
        }
        out.extend_from_slice(&bytes[PNG_IHDR_END..]);
        let folder = Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(folder)?;
        std::io::Write::write_all(&mut file, &out)?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SourceIdentity {
    pub input_ref: String,
    pub filesystem_path: String,
    pub size_bytes: u64,
    pub modified_at_utc: Option<String>,
    pub sha256: String,
}

impl SourceIdentity {
    pub fn capture(path: &str, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        ensure!(!cancelled(), "Scientific input verification cancelled");
        let filesystem_path = ImageRef::parse(path).path;
        let mut file = File::open(&filesystem_path).with_context(|| format!("Cannot identify science input {filesystem_path}"))?;
        let before = file.metadata()?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        let mut bytes = 0u64;
        loop {
            ensure!(!cancelled(), "Scientific input verification cancelled");
            let count = file.read(&mut buffer)?;
            if count == 0 { break; }
            hasher.update(&buffer[..count]);
            bytes += count as u64;
        }
        ensure!(!cancelled(), "Scientific input verification cancelled");
        let after = file.metadata()?;
        let current = metadata(&filesystem_path)?;
        ensure!(bytes == before.len() && before.len() == after.len() && after.len() == current.len()
            && before.modified().ok() == after.modified().ok() && after.modified().ok() == current.modified().ok(),
            "Science input changed while identifying it: {path}");
        Ok(Self {
            input_ref: path.to_string(), filesystem_path, size_bytes: bytes,
            modified_at_utc: before.modified().ok().map(utc_timestamp),
            sha256: format!("{:x}", hasher.finalize()),
        })
    }

    pub fn verify(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let current = Self::capture(&self.input_ref, cancelled)?;
        ensure!(&current == self, "Science input changed during calculation: {}. Reopen it and calculate again", self.input_ref);
        Ok(())
    }
}

pub(crate) struct VerifiedSource {
    pub identity: SourceIdentity,
    pub readable: String,
    _resolved: Option<SharedTempDir>,
}

impl VerifiedSource {
    pub fn open(path: &str, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        let identity = SourceIdentity::capture(path, cancelled)?;
        let (readable, resolved) = resolve_single_image_fresh(&identity.filesystem_path)?;
        ensure!(!cancelled(), "Scientific input verification cancelled");
        Ok(Self { identity, readable: readable.to_string_lossy().into_owned(), _resolved: resolved })
    }

    pub fn image_ref(&self) -> String {
        let mut reference = ImageRef::parse(&self.identity.input_ref);
        reference.path = self.readable.clone();
        reference.cache_key()
    }
}

pub(crate) struct VerifiedCubeSet {
    pub set: CubeSet,
    source: VerifiedSource,
}

impl VerifiedCubeSet {
    #[cfg(test)]
    pub fn verify_unchanged(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.source.identity.verify(cancelled)
    }

    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_cancel(path, &|| false)
    }

    pub fn open_with_cancel(path: &str, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        let source = VerifiedSource::open(path, cancelled)?;
        let set = open_cube_set_fresh(&source.image_ref())?;
        ensure!(!cancelled(), "Scientific input verification cancelled");
        Ok(Self { set, source })
    }
}

pub(crate) fn publish_source_manifest(
    output_dir: &str,
    operation: &str,
    sources: &[(&str, &SourceIdentity)],
    parameters: Value,
    products: Value,
    cancelled: &dyn Fn() -> bool,
) -> Result<String> {
    for (_, source) in sources { source.verify(cancelled)?; }
    let manifest = json!({
        "schema_version": 1,
        "operation": operation,
        "created_at_utc": utc_timestamp(SystemTime::now()),
        "software_version": env!("CARGO_PKG_VERSION"),
        "integrity": SCIENTIFIC_INTEGRITY,
        "integrity_limit": SCIENTIFIC_INTEGRITY_LIMIT,
        "sources": sources.iter().map(|(role, source)| json!({ "role": role, "identity": source })).collect::<Vec<_>>(),
        "parameters": parameters,
        "products": products,
    });
    let path = Path::new(output_dir).join(MANIFEST_NAME);
    let mut file = tempfile::NamedTempFile::new_in(output_dir)?;
    serde_json::to_writer_pretty(&mut file, &manifest)?;
    ensure!(!cancelled(), "Scientific input verification cancelled");
    file.persist_noclobber(&path)?;
    Ok(path.to_string_lossy().into_owned())
}

#[derive(Debug, Clone, Serialize)]
pub struct ScientificSource {
    pub input_ref: String,
    pub filesystem_path: String,
    pub size_bytes: u64,
    pub modified_at_utc: Option<String>,
    pub sha256: String,
    pub sci_hdu: usize,
    pub err_hdu: Option<usize>,
    pub dq_hdu: Option<usize>,
    pub dimensions: [usize; 3],
    pub bunit: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

    use super::*;
    use crate::core::cube::cache::GLOBAL_CUBE_CACHE;
    use crate::core::cube::lazy::test_support::write_line_cube_mef;

    #[test]
    fn manifests_identify_the_input_hdus_requested_and_effective_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.fits");
        write_line_cube_mef(&path, 0.01, |_, _, _| 0.5, |_, _, _| 0);
        let input_ref = format!("{}#hdu=1", path.display());
        let set = VerifiedCubeSet::open(&input_ref).unwrap();
        let requested = json!({ "use_err": false, "z0": 8, "z1": 32 });
        let effective = json!({ "err_hdu": null, "dq_hdu": 3 });
        let mut run = ScientificRun::new(&input_ref, dir.path().to_str().unwrap(), "linefit", &set, requested.clone()).unwrap();
        let products = json!({ "fits_path": format!("{}/flux.fits", run.output_dir) });
        let notes = vec!["ERR explicitly disabled".to_string()];
        let manifest = run.finish(effective.clone(), products.clone(), &notes).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
        assert_eq!(saved["provenance"]["source"]["input_ref"], input_ref);
        assert_eq!(saved["provenance"]["source"]["sci_hdu"], 1);
        assert_eq!(saved["provenance"]["source"]["err_hdu"], 2);
        assert_eq!(saved["provenance"]["source"]["dq_hdu"], 3);
        assert_eq!(saved["provenance"]["source"]["size_bytes"], metadata(&path).unwrap().len());
        assert_eq!(saved["provenance"]["source"]["sha256"], format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())));
        assert_eq!(saved["provenance"]["integrity"], SCIENTIFIC_INTEGRITY);
        assert_eq!(saved["provenance"]["parameters"], requested);
        assert_eq!(saved["provenance"]["effective"], effective);
        assert_eq!(saved["products"], products);
        assert_eq!(saved["notes"], json!(notes));
        assert_eq!(saved["provenance"], serde_json::to_value(&run.provenance).unwrap());
        let original = std::fs::read(&manifest).unwrap();
        assert!(run.finish(json!({ "replacement": true }), json!({}), &[]).is_err());
        assert_eq!(std::fs::read(&manifest).unwrap(), original);
        GLOBAL_CUBE_CACHE.invalidate(&input_ref);
    }

    #[test]
    fn science_product_headers_name_the_input_hash_size_time_and_build() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.fits");
        write_line_cube_mef(&path, 0.01, |_, _, _| 0.5, |_, _, _| 0);
        let input_ref = format!("{}#hdu=1", path.display());
        let set = VerifiedCubeSet::open(&input_ref).unwrap();
        let run = ScientificRun::new(&input_ref, dir.path().to_str().unwrap(), "collapse", &set, json!({})).unwrap();
        let mut header = HduHeader::empty();
        run.stamp_header(&mut header);
        GLOBAL_CUBE_CACHE.invalidate(&input_ref);
        assert_eq!(header.get("ABSRCSHA"), Some(format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())).as_str()), "{:?}", header.cards);
        assert_eq!(header.get_i64("ABSRCSZ"), Some(metadata(&path).unwrap().len() as i64));
        assert_eq!(header.get("ABSRCMT"), Some(utc_timestamp(metadata(&path).unwrap().modified().unwrap()).as_str()));
        assert_eq!(header.get_i64("ABSRCHDU"), Some(1));
        assert_eq!(header.get("ABVER"), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(header.get("ABOP"), Some("collapse"));
        assert_eq!(header.get("ABSRC"), Some(input_ref.as_str()));
    }

    #[test]
    fn a_source_version_change_prevents_a_success_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.fits");
        write_line_cube_mef(&path, 0.01, |_, _, _| 0.5, |_, _, _| 0);
        let input_ref = path.to_str().unwrap();
        let set = VerifiedCubeSet::open(input_ref).unwrap();
        let mut run = ScientificRun::new(input_ref, dir.path().to_str().unwrap(), "collapse", &set, json!({})).unwrap();
        let modified = metadata(&path).unwrap().modified().unwrap();
        OpenOptions::new().write(true).open(&path).unwrap().set_modified(modified + std::time::Duration::from_secs(1)).unwrap();
        let error = run.finish(json!({}), json!({}), &[]).unwrap_err().to_string();
        assert!(error.contains("Science input changed during calculation"), "{error}");
        assert!(!std::path::Path::new(&run.output_dir).join(MANIFEST_NAME).exists());
        GLOBAL_CUBE_CACHE.invalidate(input_ref);
    }

    #[test]
    fn identity_proof_same_stamp_content_change_must_prevent_publication() {
        use std::io::{Seek, SeekFrom, Write};

        for extension in ["SCI", "ERR", "DQ"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.fits");
        write_line_cube_mef(&path, 0.01, |_, _, _| 0.5, |_, _, _| 0);
        let input_ref = path.to_str().unwrap();
        let set = VerifiedCubeSet::open(input_ref).unwrap();
        let mut run = ScientificRun::new(input_ref, dir.path().to_str().unwrap(), "moments", &set, json!({})).unwrap();
        let original = std::fs::read(&path).unwrap();
        let modified = metadata(&path).unwrap().modified().unwrap();
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        let cube = match extension { "SCI" => &set.set.sci, "ERR" => set.set.err.as_ref().unwrap(), _ => set.set.dq.as_ref().unwrap() };
        file.seek(SeekFrom::Start(cube.geometry.data_offset as u64)).unwrap();
        file.write_all(&42.0_f32.to_be_bytes()).unwrap();
        file.flush().unwrap();
        file.set_modified(modified).unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), original);
        assert_eq!(metadata(&path).unwrap().len(), original.len() as u64);
        assert_eq!(metadata(&path).unwrap().modified().unwrap(), modified);
        let publication = run.finish(json!({}), json!({}), &[]);
        eprintln!("identity proof: same-stamp mutated source publication={publication:?}");
        GLOBAL_CUBE_CACHE.invalidate(input_ref);
        assert!(publication.is_err(), "different source bytes must not produce a successful provenance manifest");
        assert!(!Path::new(&run.output_dir).join(MANIFEST_NAME).exists());
        }
    }

    #[test]
    fn identity_proof_source_changed_after_loading_must_not_be_claimed_as_loaded_input() {
        use std::io::{Seek, SeekFrom, Write};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.fits");
        write_line_cube_mef(&path, 0.01, |_, _, _| 0.5, |_, _, _| 0);
        let input_ref = path.to_str().unwrap();
        let set = VerifiedCubeSet::open(input_ref).unwrap();
        let modified = metadata(&path).unwrap().modified().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let card = bytes.chunks_exact(80).position(|card| card.starts_with(b"BUNIT   ")).unwrap();
        let mut replacement = [b' '; 80];
        let text = b"BUNIT   = 'mJy/beam'";
        replacement[..text.len()].copy_from_slice(text);
        let mut file = OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start((card * 80) as u64)).unwrap();
        file.write_all(&replacement).unwrap();
        file.flush().unwrap();
        file.set_modified(modified + std::time::Duration::from_secs(1)).unwrap();
        let current = crate::core::cube::lazy::LazyCube::open(input_ref).unwrap();
        assert_ne!(set.set.sci.header.get("BUNIT"), current.header.get("BUNIT"));
        let outcome = ScientificRun::new(input_ref, dir.path().to_str().unwrap(), "collapse", &set, json!({}));
        if let Ok(mut run) = outcome {
            let publication = run.finish(json!({}), json!({}), &[]);
            eprintln!("identity proof: stale BUNIT={:?}; current BUNIT={:?}; newly captured stamp={:?}; publication={publication:?}", set.set.sci.header.get("BUNIT"), current.header.get("BUNIT"), run.provenance.source.modified_at_utc);
            GLOBAL_CUBE_CACHE.invalidate(input_ref);
            assert!(publication.is_err(), "a previously loaded cube must not be identified as the new source version");
        } else {
            GLOBAL_CUBE_CACHE.invalidate(input_ref);
        }
    }
    #[test]
    fn identity_proof_same_size_and_mtime_must_not_reuse_stale_calibration() {
        use std::io::{Seek, SeekFrom, Write};
        use crate::core::cube::lazy::test_support::write_cube;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calibration.fits");
        write_cube(&path, 2, 2, 2, &[("BSCALE", "1.0")], |_, _, _| 3.0);
        let key = path.to_str().unwrap();
        let cache = &GLOBAL_CUBE_CACHE;
        let before = cache.get_or_open(key).unwrap();
        assert_eq!(before.extract_spectrum_at(0, 0).unwrap(), [3.0, 3.0]);
        let stamp = (metadata(&path).unwrap().len(), metadata(&path).unwrap().modified().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        let card = bytes.chunks_exact(80).position(|card| card.starts_with(b"BSCALE  ")).unwrap();
        let mut replacement = [b' '; 80];
        let text = b"BSCALE  =                  2.0";
        replacement[..text.len()].copy_from_slice(text);
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.seek(SeekFrom::Start((card * 80) as u64)).unwrap();
        file.write_all(&replacement).unwrap();
        file.flush().unwrap();
        file.set_modified(stamp.1).unwrap();
        assert_eq!((metadata(&path).unwrap().len(), metadata(&path).unwrap().modified().unwrap()), stamp);
        let fresh = crate::core::cube::lazy::LazyCube::open(key).unwrap().extract_spectrum_at(0, 0).unwrap();
        let cached = VerifiedCubeSet::open(key).unwrap().set.sci.extract_spectrum_at(0, 0).unwrap();
        cache.invalidate(key);
        eprintln!("identity proof: unchanged stamp={stamp:?}; fresh={fresh:?}; cached={cached:?}");
        assert_eq!(cached, fresh, "a scientific reader must not reuse the old BSCALE after the content changes");
    }

    #[test]
    fn identity_proof_gzip_acquisition_bypasses_stale_decompression_and_keeps_companions() {
        use std::io::Write;
        use crate::core::cube::companions::open_cube_set;
        use crate::core::cube::lazy::test_support::write_sci_err_dq_mef;

        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("cube.fits");
        let gzip = dir.path().join("cube.fits.gz");
        let compress = || {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::none());
            encoder.write_all(&std::fs::read(&plain).unwrap()).unwrap();
            std::fs::write(&gzip, encoder.finish().unwrap()).unwrap();
        };
        write_sci_err_dq_mef(&plain, 2, 2, 2, |_, _, _| 3.0, |_, _, _| 0.5, |_, _, _| 0);
        compress();
        let path = gzip.to_str().unwrap();
        let old = open_cube_set(path).unwrap();
        let stamp = metadata(&gzip).unwrap();
        assert_eq!(old.sci.extract_spectrum_at(0, 0).unwrap(), [3.0, 3.0]);
        write_sci_err_dq_mef(&plain, 2, 2, 2, |_, _, _| 6.0, |_, _, _| 2.5, |_, _, _| 4);
        compress();
        OpenOptions::new().write(true).open(&gzip).unwrap().set_modified(stamp.modified().unwrap()).unwrap();
        assert_eq!(metadata(&gzip).unwrap().len(), stamp.len());
        let verified = VerifiedCubeSet::open(path).unwrap();
        assert_eq!(verified.set.sci.extract_spectrum_at(0, 0).unwrap(), [6.0, 6.0]);
        assert_eq!(verified.set.err.as_ref().unwrap().extract_spectrum_at(0, 0).unwrap(), [2.5, 2.5]);
        assert_eq!(verified.set.dq.as_ref().unwrap().extract_spectrum_at(0, 0).unwrap(), [4.0, 4.0]);
        verified.verify_unchanged(&|| false).unwrap();
        GLOBAL_CUBE_CACHE.invalidate(path);
    }

    #[test]
    fn hashing_can_be_cancelled_between_chunks_and_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.bin");
        std::fs::write(&path, vec![7u8; 3 << 20]).unwrap();
        let checks = std::cell::Cell::new(0);
        let cancel = || { checks.set(checks.get() + 1); checks.get() >= 4 };
        let error = SourceIdentity::capture(path.to_str().unwrap(), &cancel).unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(checks.get(), 4);
        let cube = dir.path().join("cube.fits");
        write_line_cube_mef(&cube, 0.01, |_, _, _| 0.5, |_, _, _| 0);
        let input = cube.to_str().unwrap();
        let verified = VerifiedCubeSet::open(input).unwrap();
        let mut run = ScientificRun::new(input, dir.path().to_str().unwrap(), "moments", &verified, json!({})).unwrap();
        assert!(run.finish_with_cancel(json!({}), json!({}), &[], &|| true).unwrap_err().to_string().contains("cancelled"));
        assert!(!Path::new(&run.output_dir).join(MANIFEST_NAME).exists());
    }

    #[test]
    #[ignore = "Local I/O timing requires ASTROBURST_IDENTITY_BENCH_PATH"]
    fn identity_hash_throughput_for_local_input() {
        let path = std::env::var("ASTROBURST_IDENTITY_BENCH_PATH").unwrap();
        let started = std::time::Instant::now();
        let identity = SourceIdentity::capture(&path, &|| false).unwrap();
        let acquisition_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = std::time::Instant::now();
        identity.verify(&|| false).unwrap();
        let verification_ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("bytes={} acquisition_ms={acquisition_ms:.3} verification_ms={verification_ms:.3} sha256={}", identity.size_bytes, identity.sha256);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ScientificProvenance {
    pub schema_version: u8,
    pub run_id: String,
    pub operation: String,
    pub created_at_utc: String,
    pub software_version: String,
    pub source: ScientificSource,
    pub parameters: Value,
    pub effective: Value,
    pub integrity: &'static str,
    pub integrity_limit: &'static str,
}

pub(crate) struct ScientificRun {
    pub output_dir: String,
    pub provenance: ScientificProvenance,
    identity: SourceIdentity,
    folder: Option<tempfile::TempDir>,
}

impl ScientificRun {
    pub fn new(path: &str, output_dir: &str, operation: &str, input: &VerifiedCubeSet, parameters: Value) -> Result<Self> {
        let identity = input.source.identity.clone();
        ensure!(identity.input_ref == path, "Scientific acquisition does not match the requested input");
        let set = &input.set;
        let folder = tempfile::Builder::new().prefix(&format!("{operation}-")).tempdir_in(output_dir)?;
        let run_id = folder.path().file_name().context("Scientific run has no folder name")?.to_string_lossy().into_owned();
        let g = &set.sci.geometry;
        Ok(Self {
            output_dir: folder.path().to_string_lossy().into_owned(),
            provenance: ScientificProvenance {
                schema_version: 1,
                run_id,
                operation: operation.to_string(),
                created_at_utc: utc_timestamp(SystemTime::now()),
                software_version: env!("CARGO_PKG_VERSION").to_string(),
                source: ScientificSource {
                    input_ref: path.to_string(),
                    filesystem_path: identity.filesystem_path.clone(),
                    size_bytes: identity.size_bytes,
                    modified_at_utc: identity.modified_at_utc.clone(),
                    sha256: identity.sha256.clone(),
                    sci_hdu: set.sci.hdu_index,
                    err_hdu: set.err.as_ref().map(|cube| cube.hdu_index),
                    dq_hdu: set.dq.as_ref().map(|cube| cube.hdu_index),
                    dimensions: [g.naxis1, g.naxis2, g.depth],
                    bunit: set.sci.header.get("BUNIT").map(str::to_string),
                },
                parameters,
                effective: Value::Null,
                integrity: SCIENTIFIC_INTEGRITY,
                integrity_limit: SCIENTIFIC_INTEGRITY_LIMIT,
            },
            identity,
            folder: Some(folder),
        })
    }

    pub fn stamp_header(&self, header: &mut HduHeader) {
        header.set("ABRUNID", self.provenance.run_id.clone());
        header.set("ABPROV", MANIFEST_NAME.to_string());
        header.set("ABSRC", self.provenance.source.input_ref.clone());
        header.set("ABSCIHDU", self.provenance.source.sci_hdu.to_string());
        let source = &self.provenance.source;
        header.set(HEADER_ABOP, self.provenance.operation.clone());
        header.set(HEADER_ABSRCHDU, source.sci_hdu.to_string());
        header.set(HEADER_ABSRCSZ, source.size_bytes.to_string());
        if let Some(mtime) = &source.modified_at_utc {
            header.set(HEADER_ABSRCMT, mtime.clone());
        }
        header.set(HEADER_ABSRCSHA, source.sha256.clone());
        header.set(HEADER_ABVER, self.provenance.software_version.clone());
    }

    pub fn finish(&mut self, effective: Value, products: Value, notes: &[String]) -> Result<String> {
        self.finish_with_cancel(effective, products, notes, &|| false)
    }

    pub fn finish_with_cancel(&mut self, effective: Value, products: Value, notes: &[String], cancelled: &dyn Fn() -> bool) -> Result<String> {
        self.identity.verify(cancelled)?;
        self.provenance.effective = effective;
        let path = std::path::Path::new(&self.output_dir).join(MANIFEST_NAME);
        let mut file = tempfile::NamedTempFile::new_in(&self.output_dir)?;
        serde_json::to_writer_pretty(&mut file, &json!({ "provenance": self.provenance, "products": products, "notes": notes }))?;
        ensure!(!cancelled(), "Scientific input verification cancelled");
        file.persist_noclobber(&path)?;
        if let Some(folder) = self.folder.take() { let _ = folder.keep(); }
        Ok(path.to_string_lossy().into_owned())
    }
}
