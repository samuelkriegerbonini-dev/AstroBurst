use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use anyhow::{bail, ensure, Result};
use sha2::{Digest, Sha256};
use crate::infra::progress::ProgressHandle;

pub(in crate::service::sagui) fn hash_input(input: &Path, progress: &ProgressHandle, failure: &str) -> Result<String> {
    sha256(input, progress).map_err(|error| if progress.is_cancelled() { error } else { error.context(format!("The input cube {} {failure}", input.display())) })
}

pub(in crate::service::sagui) fn sha256(path: &Path, progress: &ProgressHandle) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        ensure!(!progress.is_cancelled(), "SAGUI cancelled during integrity verification");
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(crate) fn library_fingerprint(path: &Path, progress: &ProgressHandle) -> Result<BTreeMap<String, String>> {
    let root = path.canonicalize()?;
    let mut pending = vec![root.clone()];
    let mut visited = HashSet::new();
    let mut files = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        ensure!(!progress.is_cancelled(), "SAGUI cancelled during library verification");
        ensure!(visited.insert(directory.clone()), "The private R library contains a directory cycle");
        for entry in fs::read_dir(directory)? {
            let path = entry?.path().canonicalize()?;
            ensure!(path.starts_with(&root), "The private R library contains an external link");
            if path.is_dir() { pending.push(path); }
            else { files.insert(path.strip_prefix(&root)?.to_string_lossy().into_owned(), sha256(&path, progress)?); }
        }
    }
    Ok(files)
}

pub(in crate::service::sagui) struct LibraryIdentity {
    pub rscript_sha256: String,
    pub files: BTreeMap<String, String>,
}

pub(in crate::service::sagui) fn installed_identity(rscript: &Path, library: &Path, progress: &ProgressHandle) -> Result<Option<LibraryIdentity>> {
    let Some(manifest_path) = library.parent().map(|installation| installation.join("manifest.json")) else { return Ok(None) };
    if !(manifest_path.is_file() && rscript.is_file() && library.is_dir()) { return Ok(None); }
    let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&fs::read(&manifest_path)?) else { return Ok(None) };
    let (Some(recorded_rscript), Some(recorded_files)) = (manifest["rscript_sha256"].as_str(), manifest["private_library"].as_object()) else { return Ok(None) };
    if manifest["status"] != "ready" { return Ok(None); }
    let advice = format!("its installation record is {}; install the SAGUI runtime again, which makes a new installation and keeps this one", manifest_path.display());
    let rscript_sha256 = sha256(rscript, progress)?;
    ensure!(rscript_sha256 == recorded_rscript, "Rscript {} changed since it was installed: {advice}", rscript.display());
    let files = library_fingerprint(library, progress)?;
    let recorded: BTreeMap<&str, &str> = recorded_files.iter().filter_map(|(file, hash)| Some((file.as_str(), hash.as_str()?))).collect();
    let names: std::collections::BTreeSet<&str> = recorded.keys().copied().chain(files.keys().map(String::as_str)).collect();
    for name in names {
        let change = match (recorded.get(name), files.get(name)) {
            (Some(before), Some(now)) if *before == now.as_str() => continue,
            (Some(_), Some(_)) => "has other content",
            (Some(_), None) => "is missing",
            (None, _) => "was added",
        };
        bail!("The private R library {} changed since it was installed: {name} {change}; {advice}", library.display());
    }
    Ok(Some(LibraryIdentity { rscript_sha256, files }))
}
